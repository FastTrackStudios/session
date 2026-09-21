//! Build a song's structure into a project from its keyflow chart.
//!
//! One call for what a chart can say about a session before anything is
//! recorded against it: the project tempo and meter, the SONG-lane region,
//! the COUNT-IN / SONGSTART / SONGEND / END markers, one coloured region
//! per section on the SECTIONS lane, and the Keyflow folder (KEY / CHORD /
//! LINES / HITS) the generator writes into.
//!
//! Generic over the `daw::service` traits, so it runs on REAPER and on the
//! in-process `daw-standalone` engine the Session window drives. The
//! pieces are the existing ones — [`chart_to_layout`] for the timeline,
//! [`stamp_song_with_default_tempo_native`] for tempo, markers and regions,
//! and the scaffold's folder builder — composed without the scaffold's
//! text-input dialog, which standalone has no UI for.
//!
//! The chart is laid out from zero: a leading count-in section starts at
//! 0 s and the song follows. Multitracks bounced from a click that starts
//! at zero line up with that without being moved.

use daw::service::transport::service::Transport as TransportService;
use daw::service::{
    ItemRef, Items, Markers, Midi, MidiNoteCreate, PositionConversion, PositionInSeconds,
    ProjectContext, Projects, Regions, TempoMap, TrackRef, Tracks,
};

use crate::setlist::chart_import::{ChartLayout, chart_to_layout};
use crate::setlist::service::demo::{chart_layout_to_demo_song, stamp_song_with_default_tempo_native};

/// What [`build_from_chart`] put into the project.
#[derive(Debug, Clone)]
pub struct ChartBuilt {
    pub title: String,
    pub tempo_bpm: f64,
    pub time_sig: (u32, u32),
    pub sections: usize,
    /// Where the song starts after the count-in, and where it ends.
    pub song_start_seconds: f64,
    pub song_end_seconds: f64,
}

/// The backend surface [`build_from_chart`] needs.
pub trait ChartDaw:
    Projects + TransportService + Markers + Regions + TempoMap + Tracks + Items + Midi + PositionConversion
{
}
impl<T> ChartDaw for T where
    T: Projects
        + TransportService
        + Markers
        + Regions
        + TempoMap
        + Tracks
        + Items
        + Midi
        + PositionConversion
{
}

/// Build the chart's structure into `project`.
///
/// Refuses a project that already has a `Keyflow` track: a second run
/// would stamp every marker and region twice, and there is no way to tell
/// the chart's regions from ones a person drew by hand to undo that.
///
/// The edit cursor is put back where it was — the marker inserts move it.
///
/// # Errors
///
/// The chart does not parse, the project already has its structure, or a
/// backend call fails.
pub fn build_from_chart<D: ChartDaw>(
    daw: &D,
    project: &ProjectContext,
    chart_text: &str,
) -> eyre::Result<ChartBuilt> {
    let layout: ChartLayout = chart_to_layout(chart_text)?;
    let already = Tracks::all(daw, project.clone())
        .iter()
        .any(|t| t.name.trim().eq_ignore_ascii_case("Keyflow"));
    if already {
        eyre::bail!("this project already has a Keyflow folder; its chart structure is built");
    }

    let title = layout
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Song".to_string());
    // `DemoSong` names its region with a `&'static str` (it was written for
    // fixtures). One short string per chart built is a leak worth not
    // widening that type for.
    let name: &'static str = Box::leak(title.clone().into_boxed_str());
    let song = chart_layout_to_demo_song(name, &layout);

    let cursor = TransportService::get_position(daw, project.clone());
    daw.begin_undo_block(project.clone(), "Build song from chart");
    // SONG / SECTIONS / MARKS, named and flagged, before anything lands
    // on them — what the insert actions do in REAPER.
    super::actions::ensure_core_lanes(daw);
    let stamped = stamp_song_with_default_tempo_native(daw, project, &song)
        .map_err(|e| eyre::eyre!("{e}"));
    let folder = stamped
        .and_then(|_| super::scaffold::build_keyflow_folder(daw, project))
        .and_then(|()| stamp_keyflow_tracks(daw, project, chart_text, &layout));
    daw.end_undo_block(project.clone(), "Build song from chart", None);
    let _ = TransportService::set_position(daw, project.clone(), cursor);
    folder?;

    Ok(ChartBuilt {
        title,
        tempo_bpm: layout.tempo_bpm,
        time_sig: (layout.time_sig_num, layout.time_sig_den),
        sections: layout.sections.len(),
        song_start_seconds: layout.song_start_seconds,
        song_end_seconds: layout.song_end_seconds,
    })
}

/// The Keyflow folder's content from the chart: the KEY track gets one
/// key item at the start (a label the rest of session reads back —
/// `crate::key`), and CHORD one MIDI item per chord, named with the
/// chord as written and holding its voicing. LINES and HITS stay empty;
/// the chart says nothing about them yet.
fn stamp_keyflow_tracks<D: ChartDaw>(
    daw: &D,
    project: &ProjectContext,
    chart_text: &str,
    layout: &ChartLayout,
) -> eyre::Result<()> {
    let chart = keyflow::text::chart::parse_chart(chart_text).map_err(|e| eyre::eyre!("chart: {e}"))?;
    if let Some(key) = &chart.initial_key {
        crate::key::set_key_at(daw, project.clone(), 0.0, key)?;
    }

    let Some(chord_track) = Tracks::all(daw, project.clone())
        .into_iter()
        .find(|t| t.name.trim().eq_ignore_ascii_case("CHORD") && t.folder_depth <= 0)
    else {
        return Ok(());
    };
    // The chart is laid out from zero at one tempo (`chart_to_layout`):
    // a beat is a quarter at `tempo_bpm`, a bar is `time_sig_num` of them.
    let beat = 60.0 / layout.tempo_bpm.max(1.0);
    let bar = beat * f64::from(layout.time_sig_num.max(1));
    let qn = |seconds: f64| {
        daw.time_to_quarter_notes(project.clone(), PositionInSeconds::from_seconds(seconds))
            .quarter_notes
            .as_quarter_notes()
    };
    for chord in super::generate::voicings(&chart, CHORD_OCTAVE) {
        #[expect(clippy::cast_precision_loss, reason = "a bar count")]
        let start = (chord.measure as f64).mul_add(bar, chord.beat * beat);
        let end = start + chord.beats * beat;
        let Some(location) = daw.create_midi_item(
            project.clone(),
            TrackRef::Guid(chord_track.guid.clone()),
            start,
            end,
        ) else {
            eyre::bail!("could not create a chord item at {start:.3} s");
        };
        let item: ItemRef = location.item.clone();
        let notes = chord
            .pitches
            .iter()
            .map(|&pitch| MidiNoteCreate {
                channel: 0,
                pitch,
                velocity: CHORD_VELOCITY,
                // A project quarter-note position, as the guide writes
                // them (see `Guide::note_create`); length in 960-PPQ ticks.
                start_ppq: qn(start),
                length_ppq: (chord.beats * 960.0).max(1.0),
            })
            .collect();
        daw.add_notes(location, notes);
        Items::set_label(daw, project.clone(), item, &chord.symbol)?;
    }
    Ok(())
}

/// Where the chord track's voicings sit: a reference to read, out of the
/// way of anything played.
const CHORD_OCTAVE: i32 = 4;
const CHORD_VELOCITY: u8 = 85;
