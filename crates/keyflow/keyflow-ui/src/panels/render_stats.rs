//! Rendering statistics and performance tracking for chart panels.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::RenderStats;

// ── FPS Tracker (used by ChartView) ─────────────────────────────────

const FPS_WINDOW_SIZE: usize = 100;

/// Sliding-window FPS tracker for chart rendering performance.
pub struct FpsTracker {
    count: usize,
    sum: u64,
    min: u64,
    max: u64,
    samples: VecDeque<u64>,
}

impl Default for FpsTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FpsTracker {
    pub fn new() -> Self {
        Self {
            count: 0,
            sum: 0,
            min: u64::MAX,
            max: u64::MIN,
            samples: VecDeque::with_capacity(FPS_WINDOW_SIZE),
        }
    }

    pub fn add_sample(&mut self, frame_time_us: u64) {
        let oldest = if self.count < FPS_WINDOW_SIZE {
            self.count += 1;
            None
        } else {
            self.samples.pop_front()
        };
        self.sum += frame_time_us;
        self.samples.push_back(frame_time_us);
        if let Some(oldest) = oldest {
            self.sum -= oldest;
        }
        self.min = self.min.min(frame_time_us);
        self.max = self.max.max(frame_time_us);
    }

    pub fn snapshot(&self) -> RenderStats {
        if self.count == 0 {
            return RenderStats::default();
        }
        let frame_time_ms = (self.sum as f64 / self.count as f64) * 0.001;
        let fps = 1000.0 / frame_time_ms;
        RenderStats {
            fps,
            frame_time_ms,
            frame_time_min_ms: self.min as f64 * 0.001,
            frame_time_max_ms: self.max as f64 * 0.001,
        }
    }
}

// ── Performance Preview helpers (used by ChartPreviewPanel) ─────────

/// Cache key for detecting when the static scene layer needs rebuilding.
#[derive(Clone, Copy)]
pub struct PerfStaticSceneKey {
    pub generation: u64,
    pub width: f64,
    pub height: f64,
    pub tx: f64,
    pub ty: f64,
    pub scale: f64,
}

impl PerfStaticSceneKey {
    pub fn approx_eq(self, other: Self) -> bool {
        const EPS: f64 = 0.001;
        self.generation == other.generation
            && (self.width - other.width).abs() <= EPS
            && (self.height - other.height).abs() <= EPS
            && (self.tx - other.tx).abs() <= EPS
            && (self.ty - other.ty).abs() <= EPS
            && (self.scale - other.scale).abs() <= EPS
    }
}

/// Tracks cursor motion for smooth interpolation between DAW transport packets.
#[derive(Default)]
pub struct PerfCursorMotionState {
    pub last_sample_tick: Option<i64>,
    pub last_sample_time: Option<Instant>,
    pub velocity_ticks_per_sec: f64,
}

/// Accumulates per-frame render timing for periodic log output.
pub struct PerfRenderAccumulator {
    window_started: Instant,
    static_rebuilds: u64,
    static_build_ms: f64,
    overlay_ms: f64,
    frame_samples_ms: Vec<f64>,
}

impl Default for PerfRenderAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfRenderAccumulator {
    pub fn new() -> Self {
        Self {
            window_started: Instant::now(),
            static_rebuilds: 0,
            static_build_ms: 0.0,
            overlay_ms: 0.0,
            frame_samples_ms: Vec::with_capacity(1024),
        }
    }

    pub fn record(&mut self, static_ms: f64, overlay_ms: f64, frame_ms: f64, static_rebuilt: bool) {
        if static_rebuilt {
            self.static_rebuilds += 1;
            self.static_build_ms += static_ms;
        }
        self.overlay_ms += overlay_ms;
        self.frame_samples_ms.push(frame_ms);
    }

    pub fn maybe_flush_log(&mut self) -> Option<(u64, f64, f64, f64, u64, f64)> {
        if self.window_started.elapsed() < Duration::from_secs(5)
            || self.frame_samples_ms.is_empty()
        {
            return None;
        }

        let frames = self.frame_samples_ms.len() as u64;
        let avg_frame_ms = self.frame_samples_ms.iter().sum::<f64>() / frames as f64;
        let mut sorted = self.frame_samples_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_idx = ((sorted.len() as f64 * 0.95).floor() as usize).min(sorted.len() - 1);
        let p95_ms = sorted[p95_idx];
        let avg_overlay_ms = self.overlay_ms / frames as f64;
        let static_rebuilds = self.static_rebuilds;
        let avg_static_ms = if static_rebuilds > 0 {
            self.static_build_ms / static_rebuilds as f64
        } else {
            0.0
        };

        self.window_started = Instant::now();
        self.static_rebuilds = 0;
        self.static_build_ms = 0.0;
        self.overlay_ms = 0.0;
        self.frame_samples_ms.clear();

        Some((
            frames,
            avg_frame_ms,
            p95_ms,
            avg_overlay_ms,
            static_rebuilds,
            avg_static_ms,
        ))
    }
}
