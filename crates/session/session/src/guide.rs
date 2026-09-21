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
    Effects, FxChainContext, ItemRef, Items, Markers, Midi, MidiNoteCreate, PositionConversion,
    PositionInSeconds, ProjectContext, Projects, Regions, Takes, TempoMap, TrackRef, Tracks,
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
    const fn roles(self) -> &'static [GuideTrackRole] {
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

/// The template's group folder for click and guide material — the
/// `Guide/` a session opens with at the top. (The CLICK + GUIDE BUS is
/// its bus: routing, fed by sends, not where the tracks live.)
const CLICK_GUIDE_FOLDER: &str = "Guide";

/// Serves [`session_proto::guide::GuideActions`] against a DAW backend.
pub struct Guide<D> {
    daw: D,
    /// The instrument that plays the generated notes, by FX name —
    /// `fts.guide` on daw-standalone (the native guide engine), the FTS
    /// Guide plugin on a host that loads it. `None` writes the notes only.
    instrument: Option<String>,
}

impl<D> Guide<D> {
    pub const fn new(daw: D) -> Self {
        Self {
            daw,
            instrument: None,
        }
    }

    /// Put `fx` on every track this generates, so its notes are heard.
    #[must_use]
    pub fn with_instrument(mut self, fx: impl Into<String>) -> Self {
        self.instrument = Some(fx.into());
        self
    }
}

/// The backend capabilities guide generation needs.
pub trait GuideDaw:
    Projects
    + Tracks
    + Items
    + Takes
    + Effects
    + Markers
    + Regions
    + TempoMap
    + PositionConversion
    + Midi
    + Send
    + Sync
    + 'static
{
}

impl<T> GuideDaw for T where
    T: Projects
        + Tracks
        + Items
        + Takes
        + Effects
        + Markers
        + Regions
        + TempoMap
        + PositionConversion
        + Midi
        + Send
        + Sync
        + 'static
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
                self.clear_span(&project, &track, start, end);
            }
        }
        Ok(())
    }
}

impl<D: GuideDaw> Guide<D> {
    /// Resolve the song, build the note list, and stamp it.
    ///
    /// # Errors
    ///
    /// Returns an error if the song cannot be resolved, the song has no length (missing SONGSTART/SONGEND markers),
    /// or if a required track cannot be created or found.
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

        // A multitrack's own click and guide are audio stems that share
        // these names. They are kept — muted, as a reference beside the
        // generated tracks in the Guide folder — and never written over:
        // `find_track` only answers with tracks that carry no audio. The
        // generated tracks, played by the guide instrument, are the ones
        // heard.
        self.mute_stems(&project);

        // Clear first, then write — and only the roles this scope owns.
        for role in scope.roles() {
            let track = self.ensure_track(project.clone(), *role)?;
            self.clear_span(&project, &track, start, end);
            self.ensure_instrument(&project, &track);
        }
        self.stamp(&project, &notes, start, end)?;
        self.file_into_click_guide_folder(&project)
    }

    /// Put every click and guide track — the multitrack's muted stems and
    /// the generated ones — inside the CLICK + GUIDE folder, so the
    /// reference and its replacement sit together. On a session the
    /// organizer found existing tracks in, the bus is a flat track fed by
    /// sends; this makes it the folder. Idempotent: tracks already inside
    /// are left where they are.
    fn file_into_click_guide_folder(&self, project: &ProjectContext) -> DawResult<()> {
        let roles = [
            GuideTrackRole::Click,
            GuideTrackRole::Loop,
            GuideTrackRole::Count,
            GuideTrackRole::Guide,
        ];
        let is_guide_track = |name: &str| {
            roles
                .iter()
                .any(|role| name.trim().eq_ignore_ascii_case(role.name()))
        };

        // The folder, not a track that shares its name: the generated
        // Guide track is called "Guide" too.
        let folder = match Tracks::all(&self.daw, project.clone())
            .into_iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(CLICK_GUIDE_FOLDER) && t.folder_depth > 0)
        {
            Some(folder) => folder.guid,
            None => Tracks::add(&self.daw, project.clone(), CLICK_GUIDE_FOLDER, None)?,
        };

        // Only top-level, plain tracks move: one inside another folder
        // belongs to it, and one carrying folder structure would strand
        // what it holds — the organizer's gather rule.
        let all = Tracks::all(&self.daw, project.clone());
        let mut running = 0i32;
        let mut moving = Vec::new();
        for track in &all {
            if track.guid != folder
                && running == 0
                && track.folder_depth == 0
                && is_guide_track(&track.name)
            {
                moving.push(track.guid.clone());
            }
            running = running.saturating_add(track.folder_depth);
        }
        if moving.is_empty() {
            return Ok(());
        }
        let before = Tracks::all(&self.daw, project.clone());
        let Some(folder_index) = before.iter().position(|t| t.guid == folder) else {
            return Ok(());
        };
        let was_folder = before[folder_index].folder_depth > 0;

        Tracks::clear_selection(&self.daw, project.clone())?;
        for guid in &moving {
            Tracks::set_selected(&self.daw, project.clone(), TrackRef::Guid(guid.clone()), true)?;
        }
        Tracks::reorder_selected(
            &self.daw,
            project.clone(),
            u32::try_from(folder_index + 1).unwrap_or(u32::MAX),
            daw_proto::ReorderTracksBehavior::MakeChildOfPreviousTrack,
        )?;
        Tracks::clear_selection(&self.daw, project.clone())?;

        // A bus that was not a folder had nothing to close. The moved
        // tracks now sit right after it, so the last of them closes the
        // new folder — or everything below would fall into it.
        if !was_folder {
            let after = Tracks::all(&self.daw, project.clone());
            if let Some(folder_at) = after.iter().position(|t| t.guid == folder)
                && let Some(last) = after.get(folder_at + moving.len())
            {
                Tracks::set_folder_depth(
                    &self.daw,
                    project.clone(),
                    TrackRef::Guid(last.guid.clone()),
                    last.folder_depth - 1,
                )?;
            }
        }
        Ok(())
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
                    time_sig_num: u32::try_from(num.max(1)).unwrap_or(4),
                    time_sig_den: u32::try_from(den.max(1)).unwrap_or(4),
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
                    time_sig_num: u32::try_from(num.max(1)).unwrap_or(4),
                    time_sig_den: u32::try_from(den.max(1)).unwrap_or(4),
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
        project: &ProjectContext,
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
                .map(|note| self.note_create(project, note))
                .collect();
            self.daw.add_notes(location, creates);
        }
        Ok(())
    }

    fn note_create(&self, project: &ProjectContext, note: &GuideMidiNote) -> MidiNoteCreate {
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

    /// The song the guide is built for, read from this backend's own
    /// markers and regions — not REAPER's: this used to call
    /// `SongBuilder::build_native`, which only exists with the `reaper`
    /// feature, so guide generation failed on `daw-standalone`.
    fn current_song(&self) -> DawResult<session_proto::Song> {
        SongBuilder::build_on(&self.daw, ProjectContext::Current)
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
        Tracks::all(&self.daw, project.clone())
            .into_iter()
            .filter(|track| track.name.eq_ignore_ascii_case(role.name()))
            // Not a folder: the `Guide` group folder carries the role's
            // name and no audio, and is not a track to write notes into.
            .filter(|track| track.folder_depth <= 0)
            .find(|track| !self.has_audio(&project, &track.guid))
            .map(|track| TrackRef::Guid(track.guid))
    }

    /// Whether a track carries recorded audio — which makes a track named
    /// "Click" or "Guide" a multitrack's stem, not the generator's own.
    fn has_audio(&self, project: &ProjectContext, track_guid: &str) -> bool {
        Items::get_items(&self.daw, project.clone(), TrackRef::Guid(track_guid.to_owned()))
            .iter()
            .any(|item| {
                Takes::get_active_take(&self.daw, project.clone(), ItemRef::Guid(item.guid.clone()))
                    .is_some_and(|take| !take.is_midi)
            })
    }

    /// Mute every audio stem named for a guide role (Click, Count, Guide):
    /// the generated tracks replace them, and both playing would double
    /// the click.
    fn mute_stems(&self, project: &ProjectContext) {
        let roles = [GuideTrackRole::Click, GuideTrackRole::Count, GuideTrackRole::Guide];
        for track in Tracks::all(&self.daw, project.clone()) {
            let named_for_a_role = roles
                .iter()
                .any(|role| track.name.trim().eq_ignore_ascii_case(role.name()));
            if named_for_a_role
                && track.folder_depth <= 0
                && !track.muted
                && self.has_audio(project, &track.guid)
            {
                let _ = Tracks::set_muted(&self.daw, project.clone(), TrackRef::Guid(track.guid), true);
            }
        }
    }

    /// Put the instrument on a generated track, once.
    fn ensure_instrument(&self, project: &ProjectContext, track: &TrackRef) {
        let (Some(fx), TrackRef::Guid(guid)) = (&self.instrument, track) else {
            return;
        };
        let chain = FxChainContext::Track(guid.clone());
        let present = Effects::list(&self.daw, project.clone(), chain.clone())
            .iter()
            .any(|f| f.name == *fx);
        if !present && Effects::add(&self.daw, project.clone(), chain, fx).is_none() {
            tracing::warn!(fx, track = guid, "guide: could not add the instrument");
        }
    }

    /// Find the role's track, creating it if absent. Creating is the
    /// point — this is meant to be one keystroke on a project that has
    /// markers and nothing else.
    fn ensure_track(&self, project: ProjectContext, role: GuideTrackRole) -> DawResult<TrackRef> {
        if let Some(track) = self.find_track(project.clone(), role) {
            return Ok(track);
        }
        // Inside the CLICK + GUIDE folder when the session has one (the
        // organizer builds it), as its first child — which keeps the
        // folder's own close on its last child intact.
        let at = Tracks::all(&self.daw, project.clone())
            .into_iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(CLICK_GUIDE_FOLDER) && t.folder_depth > 0)
            .map(|folder| folder.index + 1);
        let guid = Tracks::add(&self.daw, project, role.name(), at)?;
        Ok(TrackRef::Guid(guid))
    }

    /// Drop every item on `track` that overlaps `[start, end)`.
    fn clear_span(&self, project: &ProjectContext, track: &TrackRef, start: f64, end: f64) {
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
