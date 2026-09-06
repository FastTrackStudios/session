//! Opens all 10 of the Rockstars album's real, organized `.RPP` projects in
//! one REAPER instance and proves `SetlistService` builds a real 10-song
//! setlist from them — the actual thing Recording Mode's `load_playlist`
//! does with a real album, not a synthetic single project.
//!
//! Needs `--virtual`: REAPER's flake version doesn't recognize these real
//! projects' newer format ("Project Load Warning" — thousands of unknown
//! elements, likely fixed-lane/comping state from a newer REAPER build)
//! and pops a native modal with no headless-safe suppression. `--virtual`
//! runs a private Xvfb with a background dismisser closing it (and other
//! stray REAPER dialogs) throughout the run — see
//! `TestRunner::run_reaper_tests` / `VirtualDisplay::close_stray_dialogs`.
//!
//! Run via `session-extension-xtask` (needs `nix develop .#reaper-test`):
//!   cargo run -p session-extension-xtask -- --virtual rockstars

use daw::test::daw_test;

const ALBUM_ROOT: &str = "/run/media/AudioHaven/Project/Crescendum-Rockstars";

/// (folder name, `.organized.RPP` file name) — matches the on-disk layout
/// exactly (including "Kornesque "'s trailing space).
const SONGS: &[(&str, &str)] = &[
    (
        "Chained expectations",
        "Chained expectations.organized.RPP",
    ),
    ("empty focus", "empty focus.organized.RPP"),
    ("heavify", "heavify.organized.RPP"),
    ("Intro", "Intro.organized.RPP"),
    ("it knows my name", "it knows my name.organized.RPP"),
    ("Kornesque ", "Kornesque .organized.RPP"),
    ("set in stone", "set in stone.organized.RPP"),
    ("The ballad", "The ballad.organized.RPP"),
    ("the plague", "the plague.organized.RPP"),
    ("unbreakable", "unbreakable.organized.RPP"),
];

async fn connect_setlist_service() -> eyre::Result<session::SetlistServiceClient> {
    let socket = std::env::var("FTS_SOCKET").map_err(|_| {
        eyre::eyre!(
            "FTS_SOCKET not set — this test must run under session-extension-xtask, \
             not `cargo test` directly"
        )
    })?;
    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .map_err(|e| eyre::eyre!("connecting to {socket}: {e}"))?;
    let link = vox_stream::StreamLink::unix(stream);
    let connection = vox::initiator_on(link)
        .establish_connection()
        .await
        .map_err(|e| eyre::eyre!("vox handshake with {socket}: {e:?}"))?;
    connection
        .open_lane::<session::SetlistServiceClient>()
        .await
        .map_err(|e| eyre::eyre!("open SetlistServiceClient lane: {e:?}"))
}

#[daw_test]
async fn rockstars_album_opens_all_ten_songs(ctx: &daw::test::DawTestContext) -> eyre::Result<()> {
    for (folder, file) in SONGS {
        let path = format!("{ALBUM_ROOT}/{folder}/{file}");
        if !std::path::Path::new(&path).exists() {
            return Err(eyre::eyre!(
                "expected album file missing: {path} — is /run/media/AudioHaven mounted?"
            ));
        }
        println!("opening {path} …");
        let started = std::time::Instant::now();
        let project = ctx
            .daw
            .open_project(path.clone())
            .await
            .map_err(|e| eyre::eyre!("opening {path}: {e:?}"))?;
        println!("opened {path} in {:.1}s", started.elapsed().as_secs_f64());

        // Confirm per-track state (the mixer/record-arm status view's data
        // source) is reachable for a real, fully-loaded multitrack
        // project — not just the synthetic single-track fixture the other
        // REAPER test uses. Checked per-song, right after opening it,
        // rather than via `current_project()` later: which project counts
        // as "current" after several more opens and seeks is REAPER's own
        // focus state, not something this test controls.
        let tracks = project.tracks().all().await?;
        assert!(
            !tracks.is_empty(),
            "expected real tracks for {path}, got none"
        );
    }

    let client = connect_setlist_service().await?;
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;

    let setlist = client
        .setlist()
        .await
        .map_err(|e| eyre::eyre!("setlist: {e:?}"))?;

    let names: Vec<_> = setlist.songs.iter().map(|s| s.name.clone()).collect();

    // REAPER always starts with its own blank "Untitled" tab, and the
    // `#[daw_test]` harness's own default tab is another — both are real
    // open projects `build_from_open_projects` correctly includes, not a
    // bug. Filter them out rather than asserting on a count Recording
    // Mode's own `load_playlist` never has to deal with (it never opens
    // an album into a pre-existing blank tab).
    let album_songs: Vec<_> = setlist
        .songs
        .iter()
        .filter(|s| s.name != "Untitled")
        .collect();
    assert_eq!(
        album_songs.len(),
        SONGS.len(),
        "expected all {} Rockstars songs open as one setlist, got {}: {names:?}",
        SONGS.len(),
        album_songs.len(),
    );

    drop(album_songs);

    // Sections hydrate lazily per song — matching real Recording Mode's
    // `load_playlist`, which only seeks the first song after building —
    // so page through every song's first section the way a performer
    // would, then confirm SetlistService now reports it (this is the
    // exact structure the performance view reads from). "empty focus" has
    // no sections at all (a real gap in its source markers, not a bug —
    // confirmed separately via report-sections), so it's skipped.
    for (index, song) in setlist.songs.iter().enumerate() {
        if song.name == "Untitled" || song.name.starts_with("empty focus") {
            continue;
        }
        client
            .seek_to_section(index, 0)
            .await
            .map_err(|e| eyre::eyre!("seek_to_section({index}, 0) for '{}': {e:?}", song.name))?;
    }

    let setlist = client
        .setlist()
        .await
        .map_err(|e| eyre::eyre!("setlist (post-seek): {e:?}"))?;
    for song in &setlist.songs {
        if song.name == "Untitled" || song.name.starts_with("empty focus") {
            continue;
        }
        assert!(
            !song.sections.is_empty(),
            "song '{}' should have sections after seeking to it but has none",
            song.name
        );
    }

    Ok(())
}
