//! One play button for everyone.

#![allow(clippy::float_cmp, clippy::unwrap_used)]

use session_sync::transport::{
    ClockSync, Command, Follower, LocalTransport, SharedTransport, TransportMode,
};

fn shared(playing: bool, position: f64, at_ms: f64, seq: u64) -> SharedTransport {
    SharedTransport {
        mode: TransportMode::Shared,
        playing,
        position,
        at_ms,
        song: Some("washed".into()),
        seq,
        by: "alice".into(),
    }
}

fn local(playing: bool, position: f64) -> LocalTransport {
    LocalTransport {
        playing,
        position,
        song: Some("washed".into()),
    }
}

#[test]
fn it_round_trips() {
    let s = shared(true, 12.5, 1000.0, 7);
    assert_eq!(SharedTransport::decode(&s.encode()), Some(s));
}

#[test]
fn a_late_joiner_starts_where_the_song_is_now() {
    let mut f = Follower::default();
    // Alice pressed play at 10 s, two seconds ago.
    let cmds = f.step(&shared(true, 10.0, 0.0, 1), &local(false, 0.0), 2000.0);
    assert_eq!(cmds, vec![Command::Play { from: 12.0 }]);
}

#[test]
fn a_stop_stops_everyone_where_it_was_pressed() {
    let mut f = Follower::default();
    let cmds = f.step(&shared(false, 30.0, 5000.0, 2), &local(true, 30.02), 5010.0);
    assert_eq!(cmds, vec![Command::Stop { at: 30.0 }]);
}

#[test]
fn drift_is_corrected_but_jitter_is_not() {
    let mut f = Follower::default();
    let s = shared(true, 0.0, 0.0, 3);
    // First sight of this press: line up exactly.
    assert_eq!(
        f.step(&s, &local(true, 0.99), 1000.0),
        vec![Command::Seek { to: 1.0 }]
    );
    // 10 ms off: leave it alone.
    assert!(f.step(&s, &local(true, 2.01), 2000.0).is_empty());
    // 80 ms off: bring it back.
    assert_eq!(
        f.step(&s, &local(true, 2.92), 3000.0),
        vec![Command::Seek { to: 3.0 }]
    );
}

#[test]
fn a_different_song_is_opened_first() {
    let mut f = Follower::default();
    let here = LocalTransport {
        playing: false,
        position: 0.0,
        song: Some("praise".into()),
    };
    let cmds = f.step(&shared(true, 0.0, 0.0, 4), &here, 0.0);
    assert_eq!(
        cmds,
        vec![
            Command::SwitchSong("washed".into()),
            Command::Play { from: 0.0 }
        ]
    );
}

#[test]
fn independent_mode_asks_nothing() {
    let mut f = Follower::default();
    let s = SharedTransport {
        mode: TransportMode::Independent,
        ..shared(true, 0.0, 0.0, 5)
    };
    assert!(f.step(&s, &local(false, 40.0), 0.0).is_empty());
}

#[test]
fn the_clock_keeps_the_tightest_sample() {
    let mut c = ClockSync::default();
    // Host is 500 ms ahead. A slow, lopsided exchange, then a quick one.
    c.sample(0.0, 600.0, 200.0); // rtt 200, offset 500
    c.sample(1000.0, 1510.0, 1020.0); // rtt 20, offset 500
    c.sample(2000.0, 2600.0, 2100.0); // rtt 100, would say 550
    assert_eq!(c.round_trip(), Some(20.0));
    assert_eq!(c.shared(3000.0), 3500.0);
}
