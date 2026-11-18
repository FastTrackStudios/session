//! The session's shared clock (features `vox` and `sync`).
//!
//! Following a playhead on another machine needs one clock both sides'
//! stamps are in. It is the session host's: the host serves
//! [`SessionClock::now`] (its `sync::clock`) beside the session and its
//! presence; each peer pings it ten times a second ([`SharedClock::run`])
//! and feeds a [`ClockEstimator`] (NTP's four stamps, the fastest round
//! trips trusted); then a playhead is published with its instant in the
//! shared clock ([`SharedClock::play_state`]), and a follower passes the
//! estimate's offset to [`crate::sync::Tracker::position`].
//!
//! This is the smallest version of what Architect's presence should carry
//! (`docs/design/session.md` §7.5): Architect's `PlayState::at_ms` says it
//! is in "the clock the two sides share", but its presence stamps each
//! process's own clock. Until the presence driver pings a host clock
//! itself, a session host serves this one and stamps `at_ms` with it.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::sync::ClockEstimator;
use crate::sync::clock::now_micros_f64;
use crate::transport::Transport;

/// The host's clock, as a service.
#[architect::rpc]
pub trait SessionClock {
    /// The host's monotonic clock now, microseconds.
    async fn now(&self) -> f64;
}

/// The host's side: its own `sync::clock`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClockHost;

impl SessionClock for ClockHost {
    #[allow(clippy::unused_async_trait_impl)]
    async fn now(&self) -> f64 {
        now_micros_f64()
    }
}

/// A peer's estimate of the host's clock. Cloning shares it.
#[derive(Debug, Clone, Default)]
pub struct SharedClock {
    estimator: Arc<Mutex<ClockEstimator>>,
}

/// How often a peer pings the host's clock.
pub const PING_EVERY: Duration = Duration::from_millis(100);

impl SharedClock {
    /// The host's clock minus this process's, microseconds; `None` before
    /// the first ping returns. Zero on the host itself.
    #[must_use]
    pub fn offset_micros(&self) -> Option<f64> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .offset_micros()
    }

    /// The shortest recent round trip to the host, microseconds: half of
    /// it bounds the estimate's error.
    #[must_use]
    pub fn round_trip_micros(&self) -> Option<f64> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .round_trip_micros()
    }

    /// The shared clock now, microseconds; `None` before the first ping.
    #[must_use]
    pub fn now_micros(&self) -> Option<f64> {
        self.offset_micros().map(|o| now_micros_f64() + o)
    }

    /// One exchange with the host: this clock before, the host's during,
    /// this clock after. A handler that only reads a clock takes no
    /// measurable time, so the host's two middle stamps are one.
    ///
    /// # Errors
    ///
    /// The call failed.
    pub async fn ping(&self, host: &SessionClockClient) -> Result<(), String> {
        let t1 = now_micros_f64();
        let there = host.now().await.map_err(|e| format!("{e:?}"))?;
        let t4 = now_micros_f64();
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(t1, there, there, t4);
        Ok(())
    }

    /// Pings the host every [`PING_EVERY`] until a ping fails (the
    /// connection closed): spawn it beside the presence driver, and again
    /// on reconnect.
    ///
    /// # Errors
    ///
    /// The ping that failed.
    pub async fn run(&self, host: &SessionClockClient) -> Result<(), String> {
        loop {
            self.ping(host).await?;
            architect::platform::sleep(PING_EVERY).await;
        }
    }

    /// `transport`, stamped in this process's clock, restamped in the
    /// shared clock; `None` before the first ping.
    #[must_use]
    pub fn to_shared(&self, transport: Transport) -> Option<Transport> {
        self.offset_micros().map(|o| transport.shifted(o))
    }

    /// The presence `PlayState` for a playhead stamped in this process's
    /// clock: its instant in the shared clock, milliseconds, as
    /// `PlayState::at_ms` says it is. `None` before the first ping.
    #[must_use]
    pub fn play_state(&self, transport: Transport) -> Option<crdt::presence::PlayState> {
        self.to_shared(transport)
            .map(|t| crdt::presence::PlayState {
                playing: t.is_playing,
                position: t.playhead_seconds,
                at_ms: t.host_micros / 1000.0,
                rate: t.playrate,
            })
    }
}
