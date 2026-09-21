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
use daw::service::{Markers, ProjectContext, Projects, Regions, TempoMap, Tracks};

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
pub trait ChartDaw: Projects + TransportService + Markers + Regions + TempoMap + Tracks {}
impl<T: Projects + TransportService + Markers + Regions + TempoMap + Tracks> ChartDaw for T {}

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
    let folder = stamped.and_then(|_| super::scaffold::build_keyflow_folder(daw, project));
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
