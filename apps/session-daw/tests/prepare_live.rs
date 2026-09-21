//! A multitrack session prepared end to end on daw-standalone, the way the
//! Session window will: organize it, build the song from its chart, and
//! generate the click and guide.
//!
//! The multitrack brings its own `Click` and `Guide` audio stems. They
//! must survive as muted references in the Guide folder — never written
//! over — while the generated Click / Count / Guide tracks are MIDI,
//! played by the guide instrument, and are what is heard.

use daw::service::{ItemRef, Items, ProjectContext, Takes, TrackRef, Tracks};
use dynamic_template::apply::{organize, DawTarget};
use session::guide::{Guide, GuideScope};
use session::keyflow::from_chart::build_from_chart;
use session::song::SongBuilder;

/// Three stems, two of them a multitrack's own click and guide.
const RPP: &str = r#"<REAPER_PROJECT 0.1 "7.0/test" 0
  TEMPO 90 4 4
  <TRACK {11111111-1111-1111-1111-111111111111}
    NAME "Bass"
    <ITEM
      POSITION 0
      LENGTH 40
      <SOURCE WAVE
        FILE "Media/Bass.wav"
      >
    >
  >
  <TRACK {22222222-2222-2222-2222-222222222222}
    NAME "Click"
    <ITEM
      POSITION 0
      LENGTH 40
      <SOURCE WAVE
        FILE "Media/Click.wav"
      >
    >
  >
  <TRACK {33333333-3333-3333-3333-333333333333}
    NAME "Guide"
    <ITEM
      POSITION 0
      LENGTH 40
      <SOURCE WAVE
        FILE "Media/Guide.wav"
      >
    >
  >
>
"#;

const CHART: &str = "\
Test Song - Nobody
90bpm 4/4 #F
Count 2
In 4
1 5 6 4
vs 4
1 5 6 4
ch 4
4 1 5 6
";

/// Opening a project stands up the window's process-wide engine (the
/// facade, the "current" project), so two tests opening at once step on
/// each other. One at a time.
static ENGINE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_multitrack_is_organized_built_and_guided() {
    let _engine = ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("song.rpp");
    std::fs::write(&file, RPP).expect("write");
    let opened = session_daw::open::open_silent(&file).expect("open");
    let _runtime = session_daw::open::runtime().expect("runtime").enter();
    let daw = opened.daw.clone();
    let project = ProjectContext::Project(opened.project_guid.clone());

    organize(&mut DawTarget::on(daw.clone(), project.clone())).expect("organize");
    build_from_chart(&daw, &project, CHART).expect("chart");

    // The song the guide will be built for: named, from the count-in.
    let songs = SongBuilder::build_on(&daw, ProjectContext::Current).expect("song");
    assert_eq!(songs.len(), 1, "{songs:?}");
    assert_eq!(songs[0].name, "Test Song");

    Guide::new(daw.clone())
        .with_instrument(session_daw::guide_instrument::IDENT)
        .generate(GuideScope::All)
        .expect("generate guide");

    let tracks = Tracks::all(&daw, project.clone());
    if std::env::var_os("FTS_SHOW_TRACKS").is_some() {
        for t in &tracks {
            eprintln!("{:>2} {:>2} {} {}", t.index, t.folder_depth, if t.muted { "M" } else { " " }, t.name);
        }
    }
    let has_audio = |guid: &str| {
        Items::get_items(&daw, project.clone(), TrackRef::Guid(guid.into()))
            .iter()
            .any(|i| {
                Takes::get_active_take(&daw, project.clone(), ItemRef::Guid(i.guid.clone()))
                    .is_some_and(|t| !t.is_midi)
            })
    };
    let item_count = |guid: &str| {
        Items::get_items(&daw, project.clone(), TrackRef::Guid(guid.into())).len()
    };

    // Where each track sits: the running folder depth before it.
    let mut depth = 0i32;
    let mut inside_click_guide: Vec<&str> = Vec::new();
    let mut open_at: Option<i32> = None;
    for t in &tracks {
        if let Some(level) = open_at {
            if depth > level {
                inside_click_guide.push(t.name.as_str());
            } else {
                open_at = None;
            }
        }
        if t.name == "Guide" && t.folder_depth > 0 {
            open_at = Some(depth);
        }
        depth += t.folder_depth;
    }

    for stem in ["Click", "Guide"] {
        let t = tracks
            .iter()
            .find(|t| t.name == stem && has_audio(&t.guid))
            .unwrap_or_else(|| panic!("the {stem} stem is still there, with its audio"));
        assert!(t.muted, "the {stem} stem is muted as a reference");
    }
    for role in ["Click", "Count", "Guide"] {
        let generated = tracks
            .iter()
            .find(|t| t.name == role && t.folder_depth <= 0 && !has_audio(&t.guid))
            .unwrap_or_else(|| panic!("a generated {role} track"));
        assert!(item_count(&generated.guid) > 0, "the generated {role} has MIDI in it");
        assert!(!generated.muted, "the generated {role} plays");
        let chain = daw::service::FxChainContext::Track(generated.guid.clone());
        assert!(
            daw::service::Effects::list(&daw, project.clone(), chain)
                .iter()
                .any(|f| f.name == session_daw::guide_instrument::IDENT),
            "the generated {role} is played by the guide instrument"
        );
    }
    let names = |n: &str| inside_click_guide.iter().filter(|x| **x == n).count();
    assert_eq!(names("Click"), 2, "stem + generated Click in the folder: {inside_click_guide:?}");
    assert_eq!(names("Guide"), 2, "stem + generated Guide in the folder: {inside_click_guide:?}");
    assert_eq!(names("Count"), 1);
    assert_eq!(
        inside_click_guide.len(),
        5,
        "only the click and guide tracks are in the folder — the folder closes: {inside_click_guide:?}"
    );
    let total: i32 = tracks.iter().map(|t| t.folder_depth).sum();
    assert_eq!(total, 0, "every folder in the session closes");

    // And it is heard: render the first bars — the stems are muted and
    // their media is missing here, so any sound is the guide's.
    let renderer = daw::standalone::audio_engine::render::ProjectRenderer::new(
        &daw,
        &opened.project_guid,
        48_000,
    );
    let mut peak = 0.0f32;
    for block in 0..(48_000 * 4 / 512) {
        let out = renderer.render_block(block * 512, 512);
        peak = out.samples.iter().fold(peak, |m, s| m.max(s.abs()));
    }
    assert!(peak > 0.01, "the click and count are audible (peak {peak})");

    // Generating again rewrites the generated tracks, not the stems.
    Guide::new(daw.clone())
        .with_instrument(session_daw::guide_instrument::IDENT)
        .generate(GuideScope::All)
        .expect("regenerate");
    let again = Tracks::all(&daw, project.clone());
    assert_eq!(again.len(), tracks.len(), "no tracks added the second time");
}

/// The real song, prepared the way the window prepares it — for looking
/// at: `FTS_CHART_PROJECT=song.rpp FTS_CHART=song.kf`.
#[test]
fn a_real_session_prepared_when_one_is_given() {
    let _engine = ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (Some(project_file), Some(chart)) = (
        std::env::var_os("FTS_CHART_PROJECT"),
        std::env::var_os("FTS_CHART"),
    ) else {
        return;
    };
    let opened = session_daw::open::open_silent(std::path::Path::new(&project_file)).expect("open");
    session_daw::prepare::Prepare {
        organize: true,
        chart: Some(chart.into()),
        guide: true,
    }
    .run(&opened)
    .expect("prepare");
    let project = ProjectContext::Project(opened.project_guid.clone());
    let mut depth = 0i32;
    for t in Tracks::all(&opened.daw, project) {
        eprintln!(
            "{}{}{}{}",
            "    ".repeat(usize::try_from(depth.max(0)).unwrap_or(0)),
            t.name,
            if t.folder_depth > 0 { "/" } else { "" },
            match (t.muted, t.visible_in_tcp) {
                (true, false) => "  [muted, hidden]",
                (true, true) => "  [muted]",
                (false, false) => "  [hidden]",
                (false, true) => "",
            }
        );
        depth += t.folder_depth;
    }
    assert_eq!(depth, 0, "every folder closes");

    // A slow song clicks in eighths: count the Click track's notes in the
    // first bar after the count-in.
    let project = ProjectContext::Project(opened.project_guid.clone());
    let click = Tracks::all(&opened.daw, project.clone())
        .into_iter()
        .find(|t| t.name == "Click" && t.folder_depth <= 0 && !t.muted)
        .expect("generated click");
    let bpm = daw::service::TempoMap::get_tempo_at(&opened.daw, project.clone(), 0.0);
    let notes: usize = Items::get_items(&opened.daw, project.clone(), TrackRef::Guid(click.guid))
        .iter()
        .map(|item| {
            daw::service::Midi::notes(
                &opened.daw,
                daw::service::MidiTakeLocation::new(
                    project.clone(),
                    ItemRef::Guid(item.guid.clone()),
                    daw::service::TakeRef::Active,
                ),
            )
            .len()
        })
        .sum();
    eprintln!("{bpm} bpm: {notes} click notes");
}
