//! Presence: what each peer says about itself, and how the others draw it.

#![allow(clippy::unwrap_used, clippy::float_cmp)]

use session_sync::presence::{
    INTERPOLATION_DELAY_MS, PLAY, POINTER, PeerState, PlayState, Pointer, PointerTrail, Roster,
    STATE, Throttle, color_for, key,
};

#[test]
fn state_round_trips() {
    let s = PeerState {
        name: "Cody".into(),
        color: 0x44_aa_ff,
        song: Some("holy-forever".into()),
        view: "arrangement".into(),
        edit_cursor: Some(12.5),
        time_selection: Some((8.0, 16.0)),
        selected_tracks: vec!["kick".into()],
        selected_items: vec!["i1".into(), "i2".into()],
        chart_caret: Some((vec![1, 2, 3], vec![4, 5])),
    };
    assert_eq!(PeerState::decode(&s.encode()), Some(s));
}

#[test]
fn a_pointer_is_in_timeline_coordinates_and_round_trips() {
    let p = Pointer::Timeline {
        at: 3.25,
        track: Some("bass".into()),
    };
    assert_eq!(Pointer::decode(&p.encode(1000.0)), Some((p, 1000.0)));
    let c = Pointer::Chart { x: 0.5, y: 0.25 };
    assert_eq!(Pointer::decode(&c.encode(5.0)), Some((c, 5.0)));
}

#[test]
fn a_play_cursor_is_extrapolated_not_streamed() {
    let play = PlayState {
        playing: true,
        position: 10.0,
        at_ms: 1000.0,
        rate: 1.0,
    };
    assert_eq!(play.position_at(3000.0), 12.0);
    let stopped = PlayState {
        playing: false,
        ..play
    };
    assert_eq!(stopped.position_at(3000.0), 10.0);
    assert_eq!(PlayState::decode(&play.encode()), Some(play));
}

#[test]
fn the_throttle_limits_the_rate_but_keeps_the_last_value() {
    let mut t = Throttle::new(33.0);
    assert_eq!(t.offer(1, 0.0), Some(1));
    assert_eq!(t.offer(2, 10.0), None);
    assert_eq!(t.offer(3, 20.0), None);
    assert_eq!(t.flush(25.0), None, "not yet");
    assert_eq!(t.flush(40.0), Some(3), "the latest, not the first held");
    assert_eq!(t.flush(80.0), None, "nothing left");
}

#[test]
fn a_remote_pointer_glides_between_samples() {
    let mut trail = PointerTrail::default();
    // Sender clock is 5 s ahead of ours; samples 50 ms apart.
    trail.push(
        Pointer::Timeline {
            at: 0.0,
            track: None,
        },
        5000.0,
        0.0,
    );
    trail.push(
        Pointer::Timeline {
            at: 1.0,
            track: None,
        },
        5050.0,
        50.0,
    );
    // Drawn INTERPOLATION_DELAY_MS behind: at local 50 + 100 - 25 we are
    // halfway between the two samples.
    let now = 50.0 + INTERPOLATION_DELAY_MS - 25.0;
    assert_eq!(
        trail.at(now),
        Some(Pointer::Timeline {
            at: 0.5,
            track: None
        })
    );
    // After the last sample, it rests there.
    assert_eq!(
        trail.at(10_000.0),
        Some(Pointer::Timeline {
            at: 1.0,
            track: None
        })
    );
}

#[test]
fn the_roster_tracks_others_and_forgets_those_who_leave() {
    let mut roster = Roster::default();
    let alice = PeerState {
        name: "Alice".into(),
        ..PeerState::default()
    };
    roster.apply("me", &key("alice", STATE), Some(&alice.encode()), 0.0);
    roster.apply(
        "me",
        &key("me", STATE),
        Some(&PeerState::default().encode()),
        0.0,
    );
    let p = Pointer::Timeline {
        at: 2.0,
        track: None,
    };
    roster.apply("me", &key("alice", POINTER), Some(&p.encode(0.0)), 0.0);
    let play = PlayState {
        playing: true,
        position: 1.0,
        at_ms: 0.0,
        rate: 1.0,
    };
    roster.apply("me", &key("alice", PLAY), Some(&play.encode()), 0.0);

    assert_eq!(roster.peers.len(), 1, "my own entries are not a peer");
    let a = &roster.peers["alice"];
    assert_eq!(a.state.as_ref().unwrap().name, "Alice");
    assert_eq!(a.play, Some(play));
    assert!(a.trail.at(1000.0).is_some());

    roster.apply("me", &key("alice", STATE), None, 0.0);
    assert!(roster.peers.is_empty());
}

#[test]
fn colours_are_stable_and_differ() {
    assert_eq!(color_for("alice"), color_for("alice"));
    assert_ne!(color_for("alice"), color_for("bob"));
}

#[test]
fn a_session_wide_entry_is_not_a_peer() {
    let mut roster = Roster::default();
    let t = session_sync::transport::SharedTransport {
        mode: session_sync::transport::TransportMode::Shared,
        playing: true,
        position: 0.0,
        at_ms: 0.0,
        song: None,
        seq: 1,
        by: "alice".into(),
    };
    roster.apply("me", session_sync::transport::KEY, Some(&t.encode()), 0.0);
    assert!(roster.peers.is_empty());
}

#[test]
fn a_pointer_over_a_panel_is_where_in_that_panel() {
    let p = Pointer::Region { region: "lyrics".into(), x: 0.25, y: 0.75 };
    assert_eq!(Pointer::decode(&p.encode(9.0)), Some((p, 9.0)));
    let mut trail = PointerTrail::default();
    trail.push(Pointer::Region { region: "chart".into(), x: 0.0, y: 0.0 }, 0.0, 0.0);
    trail.push(Pointer::Region { region: "chart".into(), x: 1.0, y: 0.5 }, 100.0, 100.0);
    assert_eq!(
        trail.at(50.0 + INTERPOLATION_DELAY_MS),
        Some(Pointer::Region { region: "chart".into(), x: 0.5, y: 0.25 })
    );
    // Across panels it jumps: halfway between the chart and the mixer is
    // on neither.
    trail.push(Pointer::Region { region: "panels".into(), x: 0.9, y: 0.9 }, 200.0, 200.0);
    assert_eq!(
        trail.at(150.0 + INTERPOLATION_DELAY_MS),
        Some(Pointer::Region { region: "panels".into(), x: 0.9, y: 0.9 })
    );
}
