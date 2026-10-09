//! The golden mix's shape, built at prepare: each drum piece's gate keyed
//! from its trigger, Kick's and Snare's returns beside a SUM of their
//! mics, the vocal effects fed from one Input — and the FTS processors
//! on them (`mix_chains`).

#![cfg(feature = "native")]

use std::sync::Arc;

use daw::service::{Effects, FxChainContext, ProjectContext, Routing, TrackRef, Tracks};
use daw_proto::routing::SendMode;

const SONG: &str = r#"<REAPER_PROJECT 0.1 "7.75/linux-x86_64" 1790015830 0
  TEMPO 120 4 4 0
  SAMPLERATE 48000 0 0
  <TRACK {A0000000-0000-0000-0000-000000000001}
    NAME Drums
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-000000000002}
    NAME Kick
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-000000000003}
    NAME In
  >
  <TRACK {A0000000-0000-0000-0000-000000000004}
    NAME Trig
    MUTESOLO 1 0 0
    ISBUS 2 -1
  >
  <TRACK {A0000000-0000-0000-0000-000000000005}
    NAME Snare
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-000000000006}
    NAME Top
  >
  <TRACK {A0000000-0000-0000-0000-000000000007}
    NAME Trig
    MUTESOLO 1 0 0
    ISBUS 2 -1
  >
  <TRACK {A0000000-0000-0000-0000-000000000008}
    NAME Toms
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-000000000009}
    NAME T1
  >
  <TRACK {A0000000-0000-0000-0000-00000000000A}
    NAME Trig
    MUTESOLO 1 0 0
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-00000000000B}
    NAME "T1 Trig"
    MUTESOLO 1 0 0
    ISBUS 2 -2
  >
  <TRACK {A0000000-0000-0000-0000-00000000000C}
    NAME Cymbals
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-00000000000D}
    NAME OHs
    ISBUS 2 -2
  >
  <TRACK {A0000000-0000-0000-0000-00000000000E}
    NAME Vocals
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-00000000000F}
    NAME Main
    ISBUS 1 1
  >
  <TRACK {A0000000-0000-0000-0000-000000000010}
    NAME "Lead Vocal"
    ISBUS 2 -2
  >
>
"#;

/// The names of `parent`'s children, in order.
fn children(tracks: &[daw_proto::Track], parent: &daw_proto::Track) -> Vec<String> {
    tracks
        .iter()
        .filter(|t| t.parent_guid.as_deref() == Some(parent.guid.as_str()))
        .map(|t| t.name.clone())
        .collect()
}

/// The track named `name` under a parent named `parent`.
fn find<'a>(tracks: &'a [daw_proto::Track], parent: &str, name: &str) -> &'a daw_proto::Track {
    tracks
        .iter()
        .find(|t| {
            t.name == name
                && t.parent_guid
                    .as_deref()
                    .and_then(|p| tracks.iter().find(|u| u.guid == p))
                    .is_some_and(|p| p.name == parent)
        })
        .unwrap_or_else(|| panic!("{parent}/{name}"))
}

#[test]
fn prepare_builds_the_golden_mix() {
    let daw = daw::standalone::Standalone::new();
    daw.set_fx_factory(Arc::new(builtin_fx::BuiltinFx));
    let loaded =
        daw::standalone::project_loader::load_rpp_text(&daw, "song", "/tmp/song.RPP", SONG)
            .expect("the song");
    let project = ProjectContext::Project(loaded.project_guid);
    let mut target = dynamic_template::apply::DawTarget::on(daw.clone(), project.clone());

    assert_eq!(
        target.key_drum_gates().expect("keyed"),
        ["Kick", "Snare", "Toms"]
    );
    assert_eq!(
        target.add_piece_verbs().expect("verbs"),
        ["Kick Verb", "Snare NonLin"]
    );
    assert_eq!(target.add_vocal_fx().expect("vocals").len(), 6);
    let chains = session_daw::mix_chains::add(&daw, &project).expect("chains");

    let tracks = Tracks::all(&daw, project.clone());
    let kick = find(&tracks, "Drums", "Kick");
    assert_eq!(children(&tracks, kick), ["Verb", "SUM"]);
    let sum = find(&tracks, "Kick", "SUM");
    assert_eq!(children(&tracks, sum), ["In", "Trig"]);

    // The trigger keys the piece: pre-FX onto 3/4, unheard but alive.
    let trig = find(&tracks, "SUM", "Trig");
    let sends = Routing::sends(&daw, project.clone(), TrackRef::Guid(trig.guid.clone()));
    let key = sends.first().expect("the key send");
    assert_eq!(key.dest_track_guid.as_deref(), Some(kick.guid.as_str()));
    assert_eq!(
        (key.send_mode, key.dest_channels.start_channel),
        (SendMode::PreFx, 2)
    );
    assert!(!trig.muted, "a muted track's sends are muted");
    assert!(!Routing::parent_send_enabled(
        &daw,
        project.clone(),
        TrackRef::Guid(trig.guid.clone())
    ));
    // A folder of tom triggers keys Toms, every trigger in it unmuted.
    assert!(!find(&tracks, "Trig", "T1 Trig").muted);
    // No trigger, no key.
    let cymbals = find(&tracks, "Drums", "Cymbals");
    assert!(
        Routing::receives(&daw, project.clone(), TrackRef::Guid(cymbals.guid.clone())).is_empty()
    );

    let vocal_fx = find(&tracks, "Vocals", "FX");
    assert_eq!(
        children(&tracks, vocal_fx),
        ["Input", "Long Verb", "Short Verb", "Slap", "Oct Down"]
    );
    let input = find(&tracks, "FX", "Input");
    let main = find(&tracks, "Vocals", "Main");
    let fed = Routing::receives(&daw, project.clone(), TrackRef::Guid(input.guid.clone()));
    assert_eq!(
        fed.first().and_then(|r| r.dest_track_guid.as_deref()),
        Some(main.guid.as_str())
    );

    // The processors: the gate first and on, the returns set, no tone on
    // a trigger.
    let fx = |track: &daw_proto::Track| {
        Effects::list(
            &daw,
            project.clone(),
            FxChainContext::Track(track.guid.clone()),
        )
    };
    let on_kick = fx(kick);
    assert_eq!(
        on_kick.first().map(|f| (f.plugin_name.as_str(), f.enabled)),
        Some(("FTS Gate", true))
    );
    assert_eq!(
        fx(find(&tracks, "Kick", "Verb"))
            .first()
            .map(|f| f.plugin_name.clone())
            .as_deref(),
        Some("FTS Reverb")
    );
    assert_eq!(
        fx(find(&tracks, "FX", "Slap"))
            .first()
            .map(|f| f.plugin_name.clone())
            .as_deref(),
        Some("FTS Delay")
    );
    assert_eq!(
        fx(find(&tracks, "FX", "Oct Down"))
            .first()
            .map(|f| f.plugin_name.clone())
            .as_deref(),
        Some("FTS Pitch")
    );
    assert!(fx(trig).is_empty(), "a trigger is a key, not a channel");
    assert!(
        fx(cymbals).iter().all(|f| f.plugin_name != "FTS Gate"),
        "Cymbals: no key, no gate"
    );
    assert_eq!(chains.gates.len(), 3);
    assert!(chains.unknown.is_empty(), "{:?}", chains.unknown);

    // Again: nothing more.
    assert!(target.key_drum_gates().expect("keyed").is_empty());
    assert!(target.add_piece_verbs().expect("verbs").is_empty());
    assert!(target.add_vocal_fx().expect("vocals").is_empty());
    assert_eq!(Tracks::all(&daw, project).len(), tracks.len());
}
