//! Demo setlist — stamps markers and regions into the current REAPER project.
//!
//! Creates a multi-song setlist with proper ruler lane organization.
//! Each song is a parent region (SONG lane) containing section child regions
//! (SECTIONS lane), with structural markers in the MARKS lane and
//! render bounds in the START/END lane.
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

use daw::Daw;
use session_proto::SessionServiceError;
use session_proto::ruler_lanes::CoreLane;
use tracing::info;

// ── Lane indices (1-based, matching CoreLane) ──────────────────────────────

const SECTIONS_LANE: u32 = CoreLane::Sections.lane_index(); // 1
const MARKS_LANE: u32 = CoreLane::Marks.lane_index(); // 2
const SONG_LANE: u32 = CoreLane::Song.lane_index(); // 3
const START_END_LANE: u32 = CoreLane::StartEnd.lane_index(); // 4

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
pub async fn stamp_demo_setlist(daw: &Daw) -> Result<(), SessionServiceError> {
    let project = daw
        .current_project()
        .await
        .map_err(|e| SessionServiceError::DawError(format!("No current project: {e}")))?;
    stamp_demo_into_project(&project).await
}

/// Stamp demo markers and regions into a specific REAPER project.
///
/// Use this when you already have a `Project` handle (e.g. in tests
/// where each test gets its own isolated project tab).
pub async fn stamp_demo_into_project(project: &daw::Project) -> Result<(), SessionServiceError> {
    info!(
        "Stamping demo setlist markers/regions into project {}",
        project.guid()
    );

    let markers_api = project.markers();
    let regions_api = project.regions();
    let songs = demo_songs();

    let mut total_markers = 0u32;
    let mut total_regions = 0u32;

    for song in &songs {
        // ── SONG-lane parent region (spans entire song) ──────────
        regions_api
            .add_in_lane(song.region_start, song.region_end, song.name, SONG_LANE)
            .await
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        total_regions += 1;

        // ── MARKS-lane structural markers ────────────────────────
        markers_api
            .add_in_lane(song.count_in, "COUNT-IN", MARKS_LANE)
            .await
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        markers_api
            .add_in_lane(song.song_start, "SONGSTART", MARKS_LANE)
            .await
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        markers_api
            .add_in_lane(song.song_end, "SONGEND", MARKS_LANE)
            .await
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        total_markers += 3;

        // ── START/END-lane render bounds ─────────────────────────
        markers_api
            .add_in_lane(song.abs_end, "=END", START_END_LANE)
            .await
            .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
        total_markers += 1;

        // ── SECTIONS-lane section regions ────────────────────────
        for section in &song.sections {
            regions_api
                .add_in_lane(section.start, section.end, section.name, SECTIONS_LANE)
                .await
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
