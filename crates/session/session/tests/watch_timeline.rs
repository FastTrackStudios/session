//! The Session watch app's beat grid, from a real (standalone) project:
//! the demo setlist's first song, read by `Guide::watch_timeline` — the
//! same song, span and tempo map `Guide::generate` stamps the Click track
//! from.
//!
//! Run: cargo test -p session --test watch_timeline

use daw_proto::ProjectInfo;
use daw_standalone::sync::Standalone;
use session::guide::Guide;
use session::setlist::service::demo::stamp_demo_setlist_with;
use session_proto::watch::WatchAccent;

// The standalone engine spawns onto a runtime as projects change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_demo_songs_grid_has_its_count_in_bars_and_sections() {
    let standalone = Standalone::new();
    standalone.seed_project(ProjectInfo { guid: "demo-proj".into(), name: "Demo".into(), path: String::new() });
    stamp_demo_setlist_with(&standalone).expect("stamp the demo setlist");

    let grid = Guide::new(standalone).watch_timeline().expect("a grid for the first song");
    assert!(grid.beats.len() > 100, "a whole song of beats: {}", grid.beats.len());
    assert!(!grid.sections.is_empty(), "the song's sections");
    let first_bar = grid.beats.iter().find(|b| b.bar == 1).expect("a bar 1");
    assert_eq!(first_bar.beat, 1);
    assert_eq!(first_bar.accent, WatchAccent::Downbeat);
    assert!(
        grid.beats.iter().any(|b| b.accent == WatchAccent::CountIn),
        "the demo songs count in"
    );
    assert!(grid.beats.iter().all(|b| b.bpm > 0.0));
}
