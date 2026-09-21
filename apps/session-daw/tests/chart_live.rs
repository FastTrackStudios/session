//! A keyflow chart builds its song structure into the project the Session
//! window has open — tempo, markers, section regions and the Keyflow
//! folder — on `daw-standalone`, through `session::keyflow::from_chart`.

use std::path::Path;

use daw::service::{Markers, ProjectContext, Regions, TempoMap, Tracks};
use session::keyflow::from_chart::build_from_chart;

/// A project with nothing in it yet — the state a chart is built into.
const EMPTY: &str = "<REAPER_PROJECT 0.1 \"7.0/test\" 0\n  TEMPO 120 4 4\n>\n";

/// Two bars of count-in, then a short song — at 90, so a bar is 8/3 s.
const CHART: &str = "\
Test Song - Nobody
90bpm 4/4 #F
Count 2
In 4
1 5 6 4
vs 8
1 5 6 4 x2
ch 8
4 1 5 6 x2
";

/// Opening a project stands up the window's process-wide engine (the
/// facade, the "current" project), so two tests opening at once step on
/// each other. One at a time.
static ENGINE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn build(project_file: &Path, chart: &str) -> (daw::standalone::Standalone, ProjectContext, session::keyflow::from_chart::ChartBuilt) {
    let opened = session_daw::open::open_silent(project_file).expect("open");
    // The transport engine spawns on the window's runtime; so must we.
    let _runtime = session_daw::open::runtime().expect("engine runtime").enter();
    let project = ProjectContext::Project(opened.project_guid.clone());
    let built = build_from_chart(&opened.daw, &project, chart).expect("build from chart");
    (opened.daw, project, built)
}

#[test]
fn a_chart_builds_tempo_markers_regions_and_the_keyflow_folder() {
    let _engine = ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("empty.rpp");
    std::fs::write(&file, EMPTY).expect("write project");
    let (daw, project, built) = build(&file, CHART);

    assert!((built.tempo_bpm - 90.0).abs() < 1e-9);
    assert_eq!(built.time_sig, (4, 4));
    assert!((TempoMap::get_tempo_at(&daw, project.clone(), 0.0) - 90.0).abs() < 1e-6);

    // Two bars of count-in at 90 in 4/4 is 16/3 s; the song starts there.
    let bar = 4.0 * 60.0 / 90.0;
    assert!((built.song_start_seconds - 2.0 * bar).abs() < 1e-6);

    let markers: Vec<String> = Markers::all(&daw, project.clone())
        .into_iter()
        .map(|m| m.name)
        .collect();
    for want in ["COUNT-IN", "SONGSTART", "SONGEND", "=END"] {
        assert!(
            markers.iter().any(|m| m.eq_ignore_ascii_case(want)),
            "missing {want} marker; have {markers:?}"
        );
    }

    let regions = Regions::all(&daw, project.clone());
    assert!(
        regions.iter().any(|r| r.name == "Test Song"),
        "the SONG-lane region carries the title"
    );
    // Intro, verse and chorus: at least one region each besides the song's.
    assert!(regions.len() > built.sections.min(3), "section regions: {regions:?}");

    let names: Vec<String> = Tracks::all(&daw, project.clone())
        .into_iter()
        .map(|t| t.name)
        .collect();
    let at = names.iter().position(|n| n == "Keyflow").expect("Keyflow folder");
    assert_eq!(&names[at + 1..at + 5], ["KEY", "CHORD", "LINES", "HITS"]);

    // KEY: one key item at the start, read back as the chart's key.
    let changes = session::key::key_changes(&daw, &project);
    assert_eq!(changes.len(), 1, "one key item: {changes:?}");
    assert!(changes[0].seconds.abs() < 1e-6, "at the start");
    assert!(
        session::key::format_key(&changes[0].key).starts_with('F'),
        "in F: {}",
        session::key::format_key(&changes[0].key)
    );

    // CHORD: one MIDI item per chord, after the two count-in bars, named
    // as written and holding notes. The verse's first chord (1 = F) is at
    // bar 6 (2 count + 4 intro) — 16 s at 90.
    let chord_track = Tracks::all(&daw, project.clone())
        .into_iter()
        .find(|t| t.name == "CHORD")
        .expect("CHORD");
    let items = daw::service::Items::get_items(&daw, project.clone(), daw::service::TrackRef::Guid(chord_track.guid));
    // intro 4 bars + verse 4 bars x2 + chorus 4 bars x2, one chord a bar.
    assert_eq!(items.len(), 20, "a chord item per chord");
    let first = items
        .iter()
        .min_by(|a, b| a.position.as_seconds().total_cmp(&b.position.as_seconds()))
        .expect("items");
    assert!((first.position.as_seconds() - 2.0 * bar).abs() < 1e-6, "after the count-in");
    let notes = daw::service::Midi::notes(
        &daw,
        daw::service::MidiTakeLocation::new(
            project.clone(),
            daw::service::ItemRef::Guid(first.guid.clone()),
            daw::service::TakeRef::Active,
        ),
    );
    assert_eq!(notes.len(), 3, "a triad");
    assert_eq!(first.label.as_deref(), Some("1"), "named as the chart writes it");

    // A second build is refused rather than stamping everything twice.
    let again = build_from_chart(&daw, &project, CHART);
    assert!(again.is_err(), "a project with its structure built refuses a rebuild");
}

/// The real song, when a path to it is given — for looking at the result,
/// not for CI: `FTS_CHART_PROJECT=song.rpp FTS_CHART=song.kf`.
#[test]
fn a_real_chart_when_one_is_given() {
    let _engine = ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (Some(project_file), Some(chart)) = (
        std::env::var_os("FTS_CHART_PROJECT"),
        std::env::var_os("FTS_CHART"),
    ) else {
        return;
    };
    let text = std::fs::read_to_string(&chart).expect("chart");
    let (daw, project, built) = build(Path::new(&project_file), &text);
    eprintln!("{built:#?}");
    let mut regions = Regions::all(&daw, project.clone());
    regions.sort_by(|a, b| a.start_seconds().total_cmp(&b.start_seconds()));
    for r in regions {
        eprintln!("  region {:>8.3} – {:>8.3}  lane {:?} color {:?}  {}", r.start_seconds(), r.end_seconds(), r.lane, r.color.map(|c| format!("{c:#08x}")), r.name);
    }
    let mut markers = Markers::all(&daw, project);
    markers.sort_by(|a, b| a.position_seconds().total_cmp(&b.position_seconds()));
    for m in markers {
        eprintln!("  marker {:>8.3}  {}", m.position_seconds(), m.name);
    }
}
