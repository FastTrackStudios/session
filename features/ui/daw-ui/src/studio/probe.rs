//! The window's diagnostic switches, in one place.
//!
//! These exist because this window's performance work kept turning on
//! questions that only a measurement could answer, and every one of them
//! wanted the window built slightly differently. They are scaffolding,
//! not features: each one makes the window WRONG in a specific, useful
//! way so that a number can be attributed to one thing.
//!
//! ```sh
//! FTS_STUDIO_PROBE=tcp-only          # no ruler, no lanes
//! FTS_STUDIO_PROBE=no-clock          # no playhead animation, no evals
//! FTS_STUDIO_PROBE=no-sticky         # unpin the TCP and the ruler
//! FTS_STUDIO_PROBE=no-sync           # no ControlSync, no MeterFeed
//! FTS_STUDIO_PROBE=no-transport      # no transport subscription
//! FTS_STUDIO_PROBE=build:0           # bare shell, then 1,2,3… add back
//! FTS_STUDIO_PROBE=items:px          # literal px, not calc(var())
//! FTS_STUDIO_PROBE=items:bare        # no label span, no containment
//! FTS_STUDIO_PROBE=lanes:plain       # lanes exactly as the control page
//! FTS_STUDIO_PROBE=lanes:nogrid      # no gradient grid
//! FTS_STUDIO_PROBE=lanes:noflex      # block layout, not flex column
//! FTS_STUDIO_PROBE=lanes:nohead      # no playhead layer
//! FTS_STUDIO_PROBE=row:none          # empty track rows
//! FTS_STUDIO_PROBE=row:fader         # one control family per row
//! FTS_STUDIO_PROBE=tcp-only,row:knob # combined, comma-separated
//! ```
//!
//! Read once at mount, never re-read: a window that changed shape
//! halfway through a measurement would measure neither shape.

use std::collections::HashSet;

/// The one environment variable. Comma-separated switches.
pub const ENV: &str = "FTS_STUDIO_PROBE";

/// What was asked for, resolved once.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Probes(HashSet<String>);

impl Probes {
    /// Read the environment. Call from a hook so it is resolved once per
    /// window rather than per render.
    pub fn from_env() -> Self {
        Self(
            std::env::var(ENV)
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        )
    }

    pub fn has(&self, switch: &str) -> bool {
        self.0.contains(switch)
    }

    /// Whether ANY probe is set — worth saying out loud, because a
    /// deliberately-wrong window that nobody remembers asking for is a
    /// day of confusing measurements.
    pub fn any(&self) -> bool {
        !self.0.is_empty()
    }

    /// The `build:<n>` level, if one was asked for.
    ///
    /// Subtracting pieces from a window that is already slow can hide an
    /// interaction — two things that are only expensive together look
    /// innocent when you remove either one. Building UP from a bare
    /// shell finds the first addition that costs something, which is a
    /// different and more trustworthy question.
    ///
    /// | level | adds |
    /// |---|---|
    /// | 0 | a plain scroller of empty rows — no grid, no sticky |
    /// | 1 | the real grid: sticky TCP column, sticky ruler slot |
    /// | 2 | real `TrackRow`s in the column |
    /// | 3 | the ruler |
    /// | 4 | the lanes (items) |
    /// | 5 | `ControlSync` + `MeterFeed` |
    /// | 6 | the playhead clock |
    /// | 7 | the transport subscription + bar |
    pub fn build_level(&self) -> Option<u8> {
        self.0
            .iter()
            .find_map(|s| s.strip_prefix("build:"))
            .and_then(|n| n.parse().ok())
    }

    /// Whether a piece at `level` should be mounted. Everything is on
    /// when no `build:` was asked for.
    pub fn at_least(&self, level: u8) -> bool {
        self.build_level().is_none_or(|asked| asked >= level)
    }

    /// The `row:<family>` selection, if one was asked for.
    pub fn row(&self) -> Option<&str> {
        self.0
            .iter()
            .find_map(|s| s.strip_prefix("row:"))
    }

    /// The zoom the `items:px` probe positions at. Fixed, because that
    /// probe gives up on zooming without a re-render — it exists to
    /// measure, not to ship.
    pub fn pps_hint(&self) -> f64 {
        super::clock::DEFAULT_PPS
    }

    /// Announce what the window has been built as. Called once.
    pub fn announce(&self) {
        if self.any() {
            let mut switches: Vec<&str> = self.0.iter().map(String::as_str).collect();
            switches.sort_unstable();
            tracing::warn!(
                probes = switches.join(","),
                "studio built with diagnostic probes — this window is deliberately not the real one"
            );
        }
    }
}
