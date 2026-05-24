//! Demo setlist — stamps markers and regions into the current REAPER project.
//!
//! Creates a multi-song setlist with proper ruler lane organization.
//! Each song is a parent region (SONG lane) containing section child regions
//! (SECTIONS lane), with structural markers in the MARKS lane and
//! render bounds in the MARKS lane.
//!
//! Layout (3 songs, ~12 minutes total):
//!
//! ```text
//! Song 1: "Great Is Thy Faithfulness" (0s–120s)
//!   COUNT-IN → SONGSTART → Intro → Verse 1 → Chorus → Verse 2 → Chorus → Outro → SONGEND
//!
//! Song 2: "Build My Life" (130s–270s)
//!   COUNT-IN → SONGSTART → Intro → Verse 1 → Pre-Chorus → Chorus → Bridge → Chorus → SONGEND
//!
//! Song 3: "Way Maker" (280s–430s)
//!   COUNT-IN → SONGSTART → Intro → Verse 1 → Chorus → Verse 2 → Chorus → Bridge → Chorus → Tag → SONGEND
//! ```

#![allow(dead_code)]

use daw::rpc;
use daw::service::{Markers, ProjectContext, Projects, Regions};
use session_proto::SessionServiceError;
use session_proto::ruler_lanes::CoreLane;
use tracing::info;

// ── Lane indices (1-based, matching CoreLane) ──────────────────────────────

const SECTIONS_LANE: u32 = CoreLane::Sections.lane_index(); // 2
const MARKS_LANE: u32 = CoreLane::Marks.lane_index(); // 3
const SONG_LANE: u32 = CoreLane::Song.lane_index(); // 1

// ── Types ──────────────────────────────────────────────────────────────────

struct DemoSong {
    /// Song name (used for the SONG-lane parent region)
    name: &'static str,
    /// Absolute start of the song region (includes count-in)
    region_start: f64,
    /// Absolute end of the song region
    region_end: f64,
    /// Position of COUNT-IN marker (before SONGSTART)
    count_in: f64,
    /// Position of SONGSTART marker
    song_start: f64,
    /// Position of SONGEND marker
    song_end: f64,
    /// Position of =END marker (render tail)
    abs_end: f64,
    /// Sections within the song
    sections: Vec<DemoSection>,
}

struct DemoSection {
    name: &'static str,
    start: f64,
    end: f64,
}

/// Stamp demo markers and regions into the current REAPER project.
///
/// This is a free function that takes a `&Daw` so it works both with
/// the global singleton and with a locally-held `Daw` instance (e.g.
/// from `daw_extension_runtime::connect()`).
pub async fn stamp_demo_setlist() -> Result<(), SessionServiceError> {
    Err(SessionServiceError::DawError(
        "stamp_demo_setlist requires a native DAW backend; use stamp_demo_setlist_with".to_string(),
    ))
}

pub fn stamp_demo_setlist_with<D>(daw: &D) -> Result<(), SessionServiceError>
where
    D: Markers + Projects + Regions,
{
    let project = daw
        .current()
        .ok_or_else(|| SessionServiceError::DawError("No current project".to_string()))?;
    stamp_demo_into_project_native(daw, ProjectContext::Project(project.guid))
}

/// Stamp demo markers and regions into a specific REAPER project.
///
/// Use this when you already have a `Project` handle (e.g. in tests
/// where each test gets its own isolated project tab).
pub async fn stamp_demo_into_project(project: &rpc::Project) -> Result<(), SessionServiceError> {
    let _ = project;
    Err(SessionServiceError::DawError(
        "stamp_demo_into_project requires a native DAW backend; use stamp_demo_into_project_native"
            .to_string(),
    ))
}

/// Native sync version for in-process extension/session paths.
pub fn stamp_demo_into_project_native<D>(
    daw: &D,
    project: ProjectContext,
) -> Result<(), SessionServiceError>
where
    D: Markers + Regions,
{
    info!(
        "Stamping demo setlist markers/regions into project {}",
        match &project {
            ProjectContext::Current => "current",
            ProjectContext::Project(guid) => guid,
        }
    );

    let songs = demo_songs();

    let mut total_markers = 0u32;
    let mut total_regions = 0u32;

    for song in &songs {
        // ── SONG-lane parent region (spans entire song) ──────────
        //
        // `Regions::add` drops the region into the project's default
        // region lane — which may or may not be SECTIONS depending on
        // project state. Pin to SONG_LANE explicitly so the song
        // bound shows up where the rest of the stack expects it.
        let song_region_id = Regions::add(
            daw,
            project.clone(),
            song.region_start,
            song.region_end,
            song.name,
        )
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        Regions::set_lane(daw, project.clone(), song_region_id, Some(SONG_LANE))
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        total_regions += 1;

        // ── SONG-lane bounds + MARKS-lane structural markers ─────
        //
        // SONGSTART / SONGEND live on SONG (alongside the parent
        // region they bracket). COUNT-IN and =END are render /
        // playback structural cues — MARKS lane per the convention
        // (see session_proto::ruler_lanes::classify_marker_lane).
        place_marker(daw, &project, song.song_start, "SONGSTART", SONG_LANE)?;
        place_marker(daw, &project, song.song_end, "SONGEND", SONG_LANE)?;
        place_marker(daw, &project, song.count_in, "COUNT-IN", MARKS_LANE)?;
        place_marker(daw, &project, song.abs_end, "=END", MARKS_LANE)?;
        total_markers += 4;

        // ── SECTIONS-lane section regions ────────────────────────
        //
        // SECTIONS has flags=8 (default region lane) so these *should*
        // land on it automatically, but make it explicit so the demo
        // is robust against projects where the default has been
        // changed by hand or by another extension.
        for section in &song.sections {
            let id = Regions::add(
                daw,
                project.clone(),
                section.start,
                section.end,
                section.name,
            )
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
            Regions::set_lane(daw, project.clone(), id, Some(SECTIONS_LANE))
                .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
            total_regions += 1;
        }
    }

    info!(
        "  Stamped {} markers, {} regions ({} songs)",
        total_markers,
        total_regions,
        songs.len()
    );
    info!("Demo markers/regions stamped successfully");
    Ok(())
}

/// Add a marker and pin it to the requested lane. Folds the
/// add-then-set_lane pair callers want every time into one fallible
/// step so the demo body reads as data.
fn place_marker<D>(
    daw: &D,
    project: &ProjectContext,
    position: f64,
    name: &str,
    lane: u32,
) -> Result<(), SessionServiceError>
where
    D: Markers,
{
    let id = Markers::add(daw, project.clone(), position, name)
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
    Markers::set_lane(daw, project.clone(), id, Some(lane))
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
    Ok(())
}

/// Build the demo setlist: 3 worship songs with realistic structure.
fn demo_songs() -> Vec<DemoSong> {
    vec![
        // ── Song 1: "Great Is Thy Faithfulness" ──────────────────
        // 120 BPM, 4/4 — classic hymn arrangement
        DemoSong {
            name: "Great Is Thy Faithfulness",
            region_start: 0.0,
            region_end: 120.0,
            count_in: 0.0,
            song_start: 4.0,
            song_end: 116.0,
            abs_end: 120.0,
            sections: vec![
                DemoSection {
                    name: "Intro",
                    start: 4.0,
                    end: 20.0,
                },
                DemoSection {
                    name: "Verse 1",
                    start: 20.0,
                    end: 44.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 44.0,
                    end: 68.0,
                },
                DemoSection {
                    name: "Verse 2",
                    start: 68.0,
                    end: 92.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 92.0,
                    end: 112.0,
                },
                DemoSection {
                    name: "Outro",
                    start: 112.0,
                    end: 116.0,
                },
            ],
        },
        // ── Song 2: "Build My Life" ──────────────────────────────
        // 68 BPM, 4/4 — modern worship, slower tempo
        DemoSong {
            name: "Build My Life",
            region_start: 130.0,
            region_end: 270.0,
            count_in: 130.0,
            song_start: 134.0,
            song_end: 266.0,
            abs_end: 270.0,
            sections: vec![
                DemoSection {
                    name: "Intro",
                    start: 134.0,
                    end: 152.0,
                },
                DemoSection {
                    name: "Verse 1",
                    start: 152.0,
                    end: 178.0,
                },
                DemoSection {
                    name: "Pre-Chorus",
                    start: 178.0,
                    end: 192.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 192.0,
                    end: 218.0,
                },
                DemoSection {
                    name: "Bridge",
                    start: 218.0,
                    end: 240.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 240.0,
                    end: 266.0,
                },
            ],
        },
        // ── Song 3: "Way Maker" ──────────────────────────────────
        // 72 BPM, 4/4 — builds dynamically, longer song
        DemoSong {
            name: "Way Maker",
            region_start: 280.0,
            region_end: 430.0,
            count_in: 280.0,
            song_start: 284.0,
            song_end: 426.0,
            abs_end: 430.0,
            sections: vec![
                DemoSection {
                    name: "Intro",
                    start: 284.0,
                    end: 300.0,
                },
                DemoSection {
                    name: "Verse 1",
                    start: 300.0,
                    end: 320.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 320.0,
                    end: 344.0,
                },
                DemoSection {
                    name: "Verse 2",
                    start: 344.0,
                    end: 364.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 364.0,
                    end: 388.0,
                },
                DemoSection {
                    name: "Bridge",
                    start: 388.0,
                    end: 408.0,
                },
                DemoSection {
                    name: "Chorus",
                    start: 408.0,
                    end: 422.0,
                },
                DemoSection {
                    name: "Tag",
                    start: 422.0,
                    end: 426.0,
                },
            ],
        },
    ]
}
