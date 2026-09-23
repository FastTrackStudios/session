//! A prepared session saved as `.session` opens again as the same session,
//! through the window's own open path: tracks and folders, the chart's
//! markers and regions, the tempo, and the guide tracks' instruments.

use std::path::Path;

use daw::service::{Effects, FxChainContext, Markers, ProjectContext, Regions, TempoMap, Tracks};

const EMPTY: &str = "<REAPER_PROJECT 0.1 \"7.0/test\" 0\n  TEMPO 120 4 4\n>\n";

const CHART: &str = "\
Saved Song - Nobody
90bpm 4/4 #F
Count 2
In 4
1 5 6 4
vs 8
1 5 6 4 x2
";

/// One engine per process (the facade binds to the first opened), so this
/// file holds one test that opens twice through two processes' worth of
/// state — the second open is read straight from the saved text.
#[test]
fn a_saved_session_opens_as_it_was_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rpp = dir.path().join("Saved Song.RPP");
    std::fs::write(&rpp, EMPTY).expect("write project");

    let opened = session_daw::open::open_silent(&rpp).expect("open");
    let _runtime = session_daw::open::runtime().expect("engine runtime").enter();
    session_daw::prepare::apply_chart(&opened.daw, &opened.project_guid, CHART, true).expect("prepare");
    let saved = rpp.with_extension("session");
    session_daw::session_file::save_session(&opened.daw, &opened.project_guid, &saved).expect("save");
    let before = snapshot(&opened.daw, &ProjectContext::Project(opened.project_guid.clone()));

    // Opened again, into a fresh engine, from the saved text.
    let again = daw::standalone::Standalone::new();
    session_daw::guide_instrument::install(
        &again,
        session_daw::guide_instrument::Library::Folder(session_daw::guide_instrument::samples_dir()),
    );
    let text = session_daw::open::project_text(&saved).expect("read the session").text;
    let loaded = daw::standalone::project_loader::load_rpp_text(&again, "Saved Song", &saved.to_string_lossy(), &text)
        .expect("load");
    let after = snapshot(&again, &ProjectContext::Project(loaded.project_guid.clone()));

    assert_eq!(before, after);
    assert!(
        before.fx.iter().any(|(track, fx)| track == "Click" && fx.iter().any(|f| f.starts_with("fts.guide"))),
        "the click plays through the guide instrument: {:?}",
        before.fx
    );
    assert!(Path::new(&saved).join("objects").is_dir());
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    tracks: Vec<(String, Option<String>, bool)>,
    markers: Vec<(String, i64)>,
    regions: Vec<(String, i64, i64, Option<u32>)>,
    tempo: i64,
    fx: Vec<(String, Vec<String>)>,
}

fn snapshot(daw: &daw::standalone::Standalone, project: &ProjectContext) -> Snapshot {
    let ms = |s: f64| (s * 1000.0).round() as i64;
    let tracks = Tracks::all(daw, project.clone());
    let name_of = |guid: &str| tracks.iter().find(|t| t.guid == guid).map(|t| t.name.clone());
    Snapshot {
        tracks: tracks
            .iter()
            .map(|t| (t.name.clone(), t.parent_guid.as_deref().and_then(name_of), t.is_folder))
            .collect(),
        markers: Markers::all(daw, project.clone())
            .into_iter()
            .map(|m| (m.name, ms(m.position.seconds().unwrap_or(0.0))))
            .collect(),
        // By lane and time: a save renumbers regions, and `Regions::all`
        // lists them by number, which is not part of what was saved.
        regions: {
            let mut regions: Vec<_> = Regions::all(daw, project.clone())
                .into_iter()
                .map(|r| (r.name, ms(r.time_range.start_seconds()), ms(r.time_range.end_seconds()), r.lane))
                .collect();
            regions.sort_by_key(|(_, start, end, lane)| (*lane, *start, *end));
            regions
        },
        tempo: ms(TempoMap::get_tempo_at(daw, project.clone(), 0.0)),
        fx: tracks
            .iter()
            .map(|t| {
                let chain = Effects::list(daw, project.clone(), FxChainContext::Track(t.guid.clone()));
                (t.name.clone(), chain.into_iter().map(|f| f.name).collect())
            })
            .filter(|(_, fx): &(String, Vec<String>)| !fx.is_empty())
            .collect(),
    }
}
