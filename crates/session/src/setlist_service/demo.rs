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
use daw::service::transport::service::Transport as TransportService;
use daw::service::{Markers, ProjectContext, Projects, Regions, TempoMap};
use session_proto::SessionServiceError;
use session_proto::ruler_lanes::CoreLane;
use tracing::info;

use crate::keyflow_actions::{self, KeyflowAction, MarkerKind, SectionKind};

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
    /// Section kind — drives both the inserted region's name (via
    /// keyflow_actions' abbreviation table) and which template colour
    /// the keyflow insert action picks. We carry the kind itself
    /// rather than a string so the demo can dispatch through the
    /// existing `insert_<kind>_region` actions and inherit all their
    /// lane / colour / carving logic.
    kind: SectionKind,
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
    D: Projects + TransportService + Markers + Regions + TempoMap,
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
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    info!(
        "Stamping demo setlist markers/regions into project {}",
        match &project {
            ProjectContext::Current => "current",
            ProjectContext::Project(guid) => guid,
        }
    );

    // Every keyflow_actions::dispatch call below operates on
    // `ProjectContext::Current`. If the caller asked for a different
    // project we have to bring it to the front first.
    if let ProjectContext::Project(guid) = &project {
        if !daw.select(guid) {
            return Err(SessionServiceError::DawError(format!(
                "could not focus project {guid} before stamping demo"
            )));
        }
    }

    let songs = demo_songs();

    let mut total_markers = 0u32;
    let mut total_regions = 0u32;

    for song in &songs {
        // ── SONG-lane parent region (the song-bounded named region) ──
        //
        // No keyflow action exists for "insert song-name region into
        // SONG lane" yet (the per-section inserts only target
        // SECTIONS). Do this one by hand: add via the trait, pin to
        // SONG lane via the new Regions::set_lane. Everything else is
        // a chain of existing actions whose ensure-lane / set-lane /
        // colour-pick logic we don't want to duplicate.
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

        // ── Structural markers — chain the existing actions ──────
        //
        // Each `insert_<kind>_marker` action:
        //   1. ensures the right CoreLane exists
        //   2. drops the marker at the *edit cursor*
        //   3. assigns the lane via `classify_marker_lane(kind.name())`
        //   4. picks the conventional colour
        // …so we just have to move the cursor and let the action do
        // its thing. That keeps lane / colour / classification logic
        // in one place (keyflow_actions) — this demo just composes.
        place_marker_via_action(daw, &project, song.count_in, MarkerKind::CountIn)?;
        place_marker_via_action(daw, &project, song.song_start, MarkerKind::SongStart)?;
        place_marker_via_action(daw, &project, song.song_end, MarkerKind::SongEnd)?;
        place_marker_via_action(daw, &project, song.abs_end, MarkerKind::End)?;
        total_markers += 4;

        // ── Section regions — chain the per-section inserts ──────
        //
        // `insert_<section>_region` takes its bounds from either the
        // current time selection or the edit cursor + default 2
        // measures. We have exact bounds for every section, so set
        // the time selection precisely and dispatch. The action
        // carves overlaps, colours per section type, and lands in
        // SECTIONS lane — all of which we'd otherwise re-implement.
        for section in &song.sections {
            place_section_via_action(daw, &project, section)?;
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

/// Move the edit cursor to `position`, then dispatch the matching
/// `insert_<kind>_marker` keyflow action. The action handles
/// lane / colour / convention — the helper just bridges between
/// "we know exactly where it goes" and "the action reads the cursor".
fn place_marker_via_action<D>(
    daw: &D,
    project: &ProjectContext,
    position: f64,
    kind: MarkerKind,
) -> Result<(), SessionServiceError>
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    TransportService::set_position(daw, project.clone(), position)
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
    keyflow_actions::dispatch(daw, KeyflowAction::InsertMarker(kind));
    Ok(())
}

/// Set the project's time selection to the section's bounds, then
/// dispatch `insert_<section>_region`. Keyflow's
/// `infer_insert_bounds` consumes the time selection (clearing it
/// afterwards) so this is the only way to feed precise bounds
/// without rewriting the insert path.
fn place_section_via_action<D>(
    daw: &D,
    project: &ProjectContext,
    section: &DemoSection,
) -> Result<(), SessionServiceError>
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    TransportService::set_position(daw, project.clone(), section.start)
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
    TransportService::set_time_selection(daw, project.clone(), section.start, section.end)
        .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;
    keyflow_actions::dispatch(daw, KeyflowAction::InsertSection(section.kind));
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
                    kind: SectionKind::Intro,
                    start: 4.0,
                    end: 20.0,
                },
                DemoSection {
                    kind: SectionKind::Verse,
                    start: 20.0,
                    end: 44.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 44.0,
                    end: 68.0,
                },
                DemoSection {
                    kind: SectionKind::Verse,
                    start: 68.0,
                    end: 92.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 92.0,
                    end: 112.0,
                },
                DemoSection {
                    kind: SectionKind::Outro,
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
                    kind: SectionKind::Intro,
                    start: 134.0,
                    end: 152.0,
                },
                DemoSection {
                    kind: SectionKind::Verse,
                    start: 152.0,
                    end: 178.0,
                },
                DemoSection {
                    kind: SectionKind::PreChorus,
                    start: 178.0,
                    end: 192.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 192.0,
                    end: 218.0,
                },
                DemoSection {
                    kind: SectionKind::Bridge,
                    start: 218.0,
                    end: 240.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
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
                    kind: SectionKind::Intro,
                    start: 284.0,
                    end: 300.0,
                },
                DemoSection {
                    kind: SectionKind::Verse,
                    start: 300.0,
                    end: 320.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 320.0,
                    end: 344.0,
                },
                DemoSection {
                    kind: SectionKind::Verse,
                    start: 344.0,
                    end: 364.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 364.0,
                    end: 388.0,
                },
                DemoSection {
                    kind: SectionKind::Bridge,
                    start: 388.0,
                    end: 408.0,
                },
                DemoSection {
                    kind: SectionKind::Chorus,
                    start: 408.0,
                    end: 422.0,
                },
                // Closest existing variant — SectionKind has no Tag yet;
                // SectionKind::Outro keeps the convention coherent (tag
                // is a structural close to the song) without requiring
                // a session-wide enum addition.
                DemoSection {
                    kind: SectionKind::Outro,
                    start: 422.0,
                    end: 426.0,
                },
            ],
        },
    ]
}
