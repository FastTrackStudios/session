//! Generate the Click / Count / Guide tracks as MIDI.
//!
//! The authoring half of the guide. `session_guide` computes *what* the
//! guide is — count-in beats from the ported legacy calculator, section
//! announcements, and the click grid — as a flat list of
//! [`GuideMidiNote`]s with no DAW types in sight. This module is the part
//! that knows about REAPER: it resolves the song, reads the tempo map,
//! finds or creates the three tracks, and writes the notes.
//!
//! That split is deliberate. The note list is a pure function of song
//! sections plus tempo, so it's tested in `session_guide` without a DAW;
//! everything here is plumbing that a headless backend
//! (`daw_standalone`) satisfies just as well as REAPER.
//!
//! Contract in [`session_proto::guide`].
//!
//! ## Idempotency
//!
//! Every generator clears its own tracks across the song's span before
//! writing. Re-running after editing sections replaces the guide rather
//! than layering a second copy on top — which matters because these are
//! actions people will hit repeatedly while arranging.

use daw::service::{
    Items, Midi, MidiNoteCreate, PositionConversion, PositionInSeconds, ProjectContext, Projects,
    TempoMap, TrackRef, Tracks,
};
use daw_proto::{DawError, DawResult};
use session_guide::midi::{ClickSubdivision, GuideMidiNote, TempoSegment, click_notes, cue_notes};
use session_guide::{CueSchedule, GuideSongTiming, ScheduleOptions, sections_from_song};
use session_proto::GuideTrackRole;

use crate::song::SongBuilder;

/// What to stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideScope {
    /// Click grid only.
    Click,
    /// Count-in and section announcements only.
    Cues,
    /// Everything.
    All,
}

impl GuideScope {
    /// Which tracks this scope owns — and therefore which ones it clears
    /// before writing. A scope must never clear a track it isn't going to
    /// rewrite, or "generate click" would silently wipe the guide.
    fn roles(self) -> &'static [GuideTrackRole] {
        match self {
            Self::Click => &[GuideTrackRole::Click],
            Self::Cues => &[GuideTrackRole::Count, GuideTrackRole::Guide],
            Self::All => &[
                GuideTrackRole::Click,
                GuideTrackRole::Count,
                GuideTrackRole::Guide,
            ],
        }
    }
}

/// Serves [`session_proto::guide::GuideActions`] against a DAW backend.
pub struct Guide<D> {
    daw: D,
}

impl<D> Guide<D> {
    pub fn new(daw: D) -> Self {
        Self { daw }
    }
}

/// The backend capabilities guide generation needs.
pub trait GuideDaw:
    Projects + Tracks + Items + TempoMap + PositionConversion + Midi + Send + Sync + 'static
{
}

impl<T> GuideDaw for T where
    T: Projects + Tracks + Items + TempoMap + PositionConversion + Midi + Send + Sync + 'static
{
}

impl<D: GuideDaw> session_proto::guide::GuideActions for Guide<D> {
    fn generate_guide_tracks(&self) -> DawResult<()> {
        self.generate(GuideScope::All)
    }

    fn generate_click_track(&self) -> DawResult<()> {
        self.generate(GuideScope::Click)
    }

    fn generate_cue_tracks(&self) -> DawResult<()> {
        self.generate(GuideScope::Cues)
    }

    fn clear_guide_tracks(&self) -> DawResult<()> {
        let project = ProjectContext::Current;
        let (start, end) = self.song_span(project.clone())?;
        for role in GuideScope::All.roles() {
            if let Some(track) = self.find_track(project.clone(), *role) {
                self.clear_span(project.clone(), &track, start, end);
            }
        }
        Ok(())
    }
}

impl<D: GuideDaw> Guide<D> {
    /// Resolve the song, build the note list, and stamp it.
    pub fn generate(&self, scope: GuideScope) -> DawResult<()> {
        let project = ProjectContext::Current;
        let song = self.current_song()?;
        let timing = GuideSongTiming::from_song(&song);
        let sections = sections_from_song(&song);
        let (start, end) = self.song_span(project.clone())?;

        let mut notes = Vec::new();
        if matches!(scope, GuideScope::Click | GuideScope::All) {
            notes.extend(click_notes(
                &self.tempo_segments(project.clone(), start, end),
                end,
                ClickSubdivision::default(),
            ));
        }
        if matches!(scope, GuideScope::Cues | GuideScope::All) {
            let schedule = CueSchedule::build(&sections, &timing, &ScheduleOptions::default());
            notes.extend(cue_notes(&schedule));
        }

        // Clear first, then write — and only the roles this scope owns.
        for role in scope.roles() {
            let track = self.ensure_track(project.clone(), *role)?;
            self.clear_span(project.clone(), &track, start, end);
        }
        self.stamp(project, &notes, start, end)
    }

    /// Read the project tempo map as the segment list the click grid
    /// wants. Every tempo point is a segment boundary, so a click stamped
    /// through a tempo ramp or a time-signature change stays with the
    /// grid instead of drifting off a single nominal BPM.
    fn tempo_segments(&self, project: ProjectContext, start: f64, end: f64) -> Vec<TempoSegment> {
        let points = self.daw.get_tempo_points(project.clone());
        let mut segments: Vec<TempoSegment> = points
            .iter()
            .filter_map(|point| {
                let at = point.position.time?.as_seconds();
                if at >= end {
                    return None;
                }
                let (num, den) = self
                    .daw
                    .get_time_signature_at(project.clone(), at.max(start));
                Some(TempoSegment {
                    start_seconds: at.max(start),
                    tempo_bpm: point.bpm,
                    time_sig_num: num.max(1) as u32,
                    time_sig_den: den.max(1) as u32,
                })
            })
            .collect();

        // A project with no tempo points still has a tempo — seed one
        // segment from the project default so the click isn't empty.
        if segments.first().map(|s| s.start_seconds) != Some(start) {
            let (num, den) = self.daw.get_time_signature_at(project.clone(), start);
            segments.insert(
                0,
                TempoSegment {
                    start_seconds: start,
                    tempo_bpm: self.daw.get_tempo_at(project, start),
                    time_sig_num: num.max(1) as u32,
                    time_sig_den: den.max(1) as u32,
                },
            );
        }
        segments.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
        segments
    }

    /// One MIDI item per track spanning the song, holding that track's
    /// notes. Positions convert seconds → project quarter-notes, which is
    /// what `Midi::add_notes` expects (`daw_reaper::midi` re-reads
    /// `start_ppq` as a QN position and converts on the way in).
    fn stamp(
        &self,
        project: ProjectContext,
        notes: &[GuideMidiNote],
        start: f64,
        end: f64,
    ) -> DawResult<()> {
        for role in GuideScope::All.roles() {
            let for_role: Vec<&GuideMidiNote> = notes.iter().filter(|n| n.role == *role).collect();
            if for_role.is_empty() {
                continue;
            }
            let track = self.ensure_track(project.clone(), *role)?;
            let location = self
                .daw
                .create_midi_item(project.clone(), track, start, end)
                .ok_or_else(|| {
                    DawError::OperationFailed(format!(
                        "could not create the {} MIDI item",
                        role.name()
                    ))
                })?;
            let creates: Vec<MidiNoteCreate> = for_role
                .iter()
                .map(|note| self.note_create(project.clone(), note))
                .collect();
            self.daw.add_notes(location, creates);
        }
        Ok(())
    }

    fn note_create(&self, project: ProjectContext, note: &GuideMidiNote) -> MidiNoteCreate {
        let qn = |seconds: f64| {
            self.daw
                .time_to_quarter_notes(project.clone(), PositionInSeconds::from_seconds(seconds))
                .quarter_notes
                .as_quarter_notes()
        };
        let start_qn = qn(note.time_seconds);
        let end_qn = qn(note.time_seconds + note.length_seconds);
        MidiNoteCreate {
            channel: 0,
            pitch: note.pitch,
            velocity: note.velocity,
            // `start_ppq` is re-read as a project quarter-note position by
            // the REAPER backend (see `daw_reaper::midi`), which converts
            // it with MIDI_GetPPQPosFromProjQN on the way in.
            start_ppq: start_qn,
            // Length, though, is a raw PPQ delta. 960 ticks per quarter is
            // REAPER's default MIDI resolution.
            length_ppq: ((end_qn - start_qn) * 960.0).max(1.0),
        }
    }

    /// The song's extent. Falls back to the project's own bounds when the
    /// song carries no explicit end.
    fn song_span(&self, project: ProjectContext) -> DawResult<(f64, f64)> {
        let song = self.current_song()?;
        let start = song.start_seconds;
        let end = if song.end_seconds > start {
            song.end_seconds
        } else {
            // No SONGEND: fall back to the last thing in the project, so a
            // half-marked project still gets a usable guide.
            self.daw
                .get_all_items(project)
                .iter()
                .map(|item| item.position.as_seconds() + item.length.as_seconds())
                .fold(start, f64::max)
        };
        if end <= start {
            return Err(DawError::OperationFailed(
                "song has no length — is SONGSTART/SONGEND stamped?".to_string(),
            ));
        }
        Ok((start, end))
    }

    fn current_song(&self) -> DawResult<session_proto::Song> {
        SongBuilder::build_native(ProjectContext::Current)
            .map_err(|err| DawError::OperationFailed(format!("could not build song: {err}")))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                DawError::NotFound(
                    "no song in the current project — stamp SONGSTART/SONGEND markers first"
                        .to_string(),
                )
            })
    }

    fn find_track(&self, project: ProjectContext, role: GuideTrackRole) -> Option<TrackRef> {
        self.daw
            .all(project)
            .into_iter()
            .find(|track| track.name.eq_ignore_ascii_case(role.name()))
            .map(|track| TrackRef::Guid(track.guid))
    }

    /// Find the role's track, creating it if absent. Creating is the
    /// point — this is meant to be one keystroke on a project that has
    /// markers and nothing else.
    fn ensure_track(&self, project: ProjectContext, role: GuideTrackRole) -> DawResult<TrackRef> {
        if let Some(track) = self.find_track(project.clone(), role) {
            return Ok(track);
        }
        let guid = self.daw.add(project, role.name(), None)?;
        Ok(TrackRef::Guid(guid))
    }

    /// Drop every item on `track` that overlaps `[start, end)`.
    fn clear_span(&self, project: ProjectContext, track: &TrackRef, start: f64, end: f64) {
        for item in self.daw.get_items(project.clone(), track.clone()) {
            let item_start = item.position.as_seconds();
            let item_end = item_start + item.length.as_seconds();
            if item_end > start && item_start < end {
                let _ = self.daw.delete_item(
                    project.clone(),
                    daw::service::ItemRef::Guid(item.guid.clone()),
                );
            }
        }
    }
}

/// Register the guide-generation actions with `backend`.
pub fn register_actions<D, B>(backend: &B, daw: D)
where
    D: GuideDaw,
    B: architect::action::ActionBackend + ?Sized,
{
    session_proto::guide::register_guide_actions(backend, std::sync::Arc::new(Guide::new(daw)));
}
