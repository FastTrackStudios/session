//! The shared clock in process: a peer's estimate of the host's clock, and
//! a playhead stamped in it. (Two processes and two machines:
//! `examples/follow.rs`.)

use architect::{LayerRouter, LocalServer, Scope};
use session::Transport;
use session::clock::{
    ClockHost, SessionClockClient, SharedClock, serve, session_clock_rpc_service_descriptor,
};
use session::sync::clock::now_micros_f64;

#[tokio::test]
async fn a_peer_estimates_the_hosts_clock_and_stamps_in_it() {
    let scope = Scope::new();
    let router = LayerRouter::new().with(session_clock_rpc_service_descriptor(), serve(ClockHost));
    let local = LocalServer::serve(router, scope.clone());
    let host: SessionClockClient = local.establish().await.unwrap();
    let clock = SharedClock::default();
    assert!(clock.offset_micros().is_none(), "nothing before a ping");
    assert!(clock.play_state(Transport::stopped(0.0, 0.0)).is_none());
    for _ in 0..8 {
        clock.ping(&host).await.unwrap();
    }
    // One process, one clock: the offset is the estimate's error alone.
    let offset = clock.offset_micros().unwrap();
    assert!(offset.abs() < 2_000.0, "offset {offset} µs");
    assert!(clock.round_trip_micros().unwrap() >= 0.0);

    // A playhead stamped here is restamped in the host's clock, in ms.
    let now = now_micros_f64();
    let play = clock
        .play_state(Transport::playing(12.0, now, 1.0))
        .unwrap();
    assert!(play.playing);
    assert!((play.position - 12.0).abs() < 1e-12);
    assert!((play.at_ms - (now + offset) / 1000.0).abs() < 1e-6);
}
