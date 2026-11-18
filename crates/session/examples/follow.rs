//! Following a playhead across two real clocks: two processes, or two
//! machines.
//!
//! ```text
//! follow lead 0.0.0.0:4141                 # serves the clock and presence, plays a loop
//! follow follow ws://HOST:4141/vox 30      # follows for 30 s, then reports
//! ```
//!
//! The leader serves `session::clock::SessionClock` and Architect's
//! presence (a `DocRegistry`'s) over vox on a WebSocket
//! (`architect::host::EngineHost`), joins its own presence as peer
//! `lead`, and plays a loop: play from 0 at 1 s, seek to 42 s at 5 s, stop
//! at 8 s, every 10 s. It is the session host, so its clock is the shared
//! clock and its `PlayState::at_ms` is stamped in it.
//!
//! The follower dials it, pings the clock ten times a second
//! (`SharedClock::run`), reads `lead/play` from its presence, and draws
//! the playhead with `sync::Tracker` every 16 ms. It reports the clock
//! estimate and how far its drawn playhead was from the leader's.
//!
//! **Measuring it.** Each process publishes its clock's epoch in wall-clock
//! microseconds (`<peer>/epoch`), so the true offset between their clocks
//! is the difference of the epochs. On one machine both wall clocks are
//! the same clock and that truth is exact (to a few microseconds of
//! reading it). Across two machines it is only as good as the two wall
//! clocks' agreement (NTP: typically a millisecond or two), so there the
//! report bounds the error by the round trip instead.

// A measuring tool, not library code: it panics on a failed dial, counts
// and indexes its own samples, and loops on the clock.
#![allow(
    clippy::expect_used,
    clippy::suboptimal_flops,
    clippy::arithmetic_side_effects,
    clippy::while_float,
    clippy::as_conversions,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use architect::LayerRouter;
use architect::connect::{Endpoint, pool};
use architect::host::EngineHost;
use crdt::CrdtDoc;
use crdt::DocRegistry;
use crdt::presence::{Here, PeerState};
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, PresencePeer, doc_presence_service_descriptor,
};
use session::Transport;
use session::clock::{
    ClockHost, SessionClockClient, SharedClock, serve as serve_clock,
    session_clock_rpc_service_descriptor,
};
use session::doc::transport_of;
use session::sync::Tracker;
use session::sync::clock::now_micros_f64;
use uuid::Uuid;

/// The one presence channel both join: a fixed document id.
const DOC: Uuid = Uuid::from_u128(0x5e55_1011_0000_4000_8000_0000_0000_0001);

/// This process's clock epoch in wall-clock microseconds.
fn epoch_wall_micros() -> f64 {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1e6;
    wall - now_micros_f64()
}

async fn join(url: &str, me: &str) -> PresencePeer {
    let client: DocPresenceClient = pool()
        .client(&Endpoint::url(url))
        .await
        .expect("dial presence");
    let (peer, mut driver) = PresencePeer::new(DOC, 30_000);
    tokio::spawn(async move {
        let _ = driver.run(&client).await;
    });
    peer.set(&format!("{me}/epoch"), epoch_wall_micros());
    peer
}

async fn lead(bind: &str) {
    let registry = DocRegistry::new(|_| Box::pin(async { Ok(CrdtDoc::ephemeral()) }));
    let router = LayerRouter::new()
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(registry),
        )
        .with(
            session_clock_rpc_service_descriptor(),
            serve_clock(ClockHost),
        );
    let bind = bind.to_owned();
    tokio::spawn(async move {
        EngineHost::ungated(router, bind)
            .serve()
            .await
            .expect("serve");
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let port = std::env::args()
        .nth(2)
        .and_then(|b| b.rsplit(':').next().map(str::to_owned));
    let url = format!(
        "ws://127.0.0.1:{}/vox",
        port.unwrap_or_else(|| "4141".into())
    );
    let peer = join(&url, "lead").await;
    let here = Here::new(peer.clone(), "lead", PeerState::new("lead", "Leader"));
    println!(
        "lead: serving on {url}; epoch {:.0} µs",
        epoch_wall_micros()
    );
    // The host's clock is the shared clock: its stamps need no offset.
    let mut cycle = 0_u32;
    loop {
        let start = now_micros_f64();
        for (at, playing, position, what) in [
            (1.0, true, 0.0, "play"),
            (5.0, true, 42.0, "seek"),
            (8.0, false, 45.0, "stop"),
        ] {
            let wait = start + at * 1e6 - now_micros_f64();
            if wait > 0.0 {
                tokio::time::sleep(Duration::from_secs_f64(wait / 1e6)).await;
            }
            let now = now_micros_f64();
            here.play(&crdt::presence::PlayState {
                playing,
                position,
                at_ms: now / 1000.0,
                rate: 1.0,
            });
            println!("lead: cycle {cycle} {what} at {position} s");
        }
        let rest = start + 10e6 - now_micros_f64();
        tokio::time::sleep(Duration::from_secs_f64(rest.max(0.0) / 1e6)).await;
        cycle += 1;
    }
}

#[allow(clippy::too_many_lines)]
async fn follow(url: &str, seconds: f64) {
    let clock = SharedClock::default();
    let host: SessionClockClient = pool()
        .client(&Endpoint::url(url))
        .await
        .expect("dial clock");
    let pinger = clock.clone();
    tokio::spawn(async move {
        let _ = pinger.run(&host).await;
    });
    let peer = join(url, "follow").await;
    let mut tracker = Tracker::default();
    let mut errors: Vec<f64> = Vec::new();
    let mut offset_errors: Vec<f64> = Vec::new();
    let mut events = 0_u32;
    let mut last: Option<Transport> = None;
    let begin = now_micros_f64();
    let my_epoch = epoch_wall_micros();
    while now_micros_f64() - begin < seconds * 1e6 {
        tokio::time::sleep(Duration::from_millis(16)).await;
        let states = peer.states();
        let (Some(offset), Some(leader), Some(lead_epoch)) = (
            clock.offset_micros(),
            states.get("lead/play").and_then(transport_of),
            states
                .get("lead/epoch")
                .and_then(|v| v.clone().into_double().ok()),
        ) else {
            continue;
        };
        if last != Some(leader) {
            events += 1;
            last = Some(leader);
        }
        tracker.observe(leader);
        let now = now_micros_f64();
        let shown = tracker.position(now, offset).unwrap_or(0.0);
        // The truth: the leader's playhead at this instant of its clock,
        // which is this clock plus the true offset (shared − local = the
        // follower's epoch − the leader's, on the wall clock).
        let true_offset = my_epoch - lead_epoch;
        let truth = leader.at(now + true_offset);
        // Skip the 150 ms after a leader's move, while its message is in
        // flight (a follower cannot know a seek before it hears it).
        if now + true_offset - leader.host_micros > 150_000.0 && now - begin > 3e6 {
            errors.push((shown - truth).abs() * 1e6);
            offset_errors.push((offset - true_offset).abs());
        }
    }
    let rtt = clock.round_trip_micros().unwrap_or(f64::NAN);
    let stat = |v: &mut Vec<f64>| -> (f64, f64, f64) {
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            return (f64::NAN, f64::NAN, f64::NAN);
        }
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        (at(0.5), at(0.99), v[v.len() - 1])
    };
    let n = errors.len();
    let (e50, e99, emax) = stat(&mut errors);
    let (o50, o99, omax) = stat(&mut offset_errors);
    println!("follow: {n} frames checked, {events} leader messages heard");
    println!("follow: round trip {rtt:.0} µs (half of it bounds the estimate's error)");
    println!("follow: clock offset error  p50 {o50:.0} µs, p99 {o99:.0} µs, max {omax:.0} µs");
    println!("follow: playhead error      p50 {e50:.0} µs, p99 {e99:.0} µs, max {emax:.0} µs");
    println!(
        "follow: (truth from wall-clock epochs: exact on one machine, NTP-bounded across two)"
    );
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("lead") => lead(args.get(2).map_or("0.0.0.0:4141", String::as_str)).await,
        Some("follow") => {
            let url = args
                .get(2)
                .map_or("ws://127.0.0.1:4141/vox", String::as_str);
            let seconds = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(30.0);
            follow(url, seconds).await;
        }
        _ => eprintln!("usage: follow lead [BIND] | follow follow [WS_URL] [SECONDS]"),
    }
}
