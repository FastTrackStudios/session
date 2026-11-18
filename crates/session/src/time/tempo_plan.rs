//! A tempo map compiled for the audio thread: [`TempoPlan`].
//!
//! [`TempoMapEngine`] walks every point on every question, and its bar and
//! beat conversions build a list of meter changes each time (an allocation
//! per call), so an engine cannot ask it from the audio thread. A plan is
//! the same map precomputed once, off the audio thread
//! ([`crate::timeline::TempoMap::plan`]): a span per tempo segment (its
//! start in seconds and in quarter notes, its two points) and a span per
//! meter (its start in quarter notes, the bar it starts). Every question is
//! then a binary search over those slices and the engine's own segment
//! arithmetic: no allocation, no lock, no walk. Ramps (linear, bezier) are
//! integrated as the engine integrates them, so the plan plays the
//! session's tempo, not a stepped copy, and agrees with the engine (the
//! tests hold it to that on many random maps).

use super::{
    EPSILON, MusicalCursor, TempoMapEngine, TempoPoint, TempoShape, TimeSignature,
    integrate_between_points, integrate_hold_tempo_segment, signature_metric,
    solve_segment_seconds,
};

/// One tempo segment: from `from_seconds` to `to_seconds` (the last runs
/// on for ever), starting `from_quarter_notes` into the song.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoSpan {
    from_seconds: f64,
    to_seconds: f64,
    from_quarter_notes: f64,
    start: TempoPoint,
    /// The point the tempo moves towards; `None`: it holds `start`'s.
    end: Option<TempoPoint>,
}

impl TempoSpan {
    /// Where it starts, seconds.
    #[must_use]
    pub const fn from_seconds(&self) -> f64 {
        self.from_seconds
    }

    /// Where it ends, seconds; infinite for the last.
    #[must_use]
    pub const fn to_seconds(&self) -> f64 {
        self.to_seconds
    }

    /// How far into the song it starts, quarter notes.
    #[must_use]
    pub const fn from_quarter_notes(&self) -> f64 {
        self.from_quarter_notes
    }

    /// The tempo at its start, BPM.
    #[must_use]
    pub const fn bpm_start(&self) -> f64 {
        self.start.bpm
    }

    /// The tempo it reaches at its end, BPM (its start's when it holds).
    #[must_use]
    pub fn bpm_end(&self) -> f64 {
        match self.shape() {
            TempoShape::Hold => self.start.bpm,
            TempoShape::Linear | TempoShape::Bezier => {
                self.end.as_ref().map_or(self.start.bpm, |e| e.bpm)
            }
        }
    }

    /// How the tempo moves across it.
    #[must_use]
    pub fn shape(&self) -> TempoShape {
        if self.end.is_none() {
            return TempoShape::Hold;
        }
        TempoShape::from_reaper_shape(self.start.shape)
    }

    fn quarter_notes_at(&self, seconds: f64) -> f64 {
        let to = seconds.min(self.to_seconds);
        self.from_quarter_notes
            + self.end.as_ref().map_or_else(
                || integrate_hold_tempo_segment(self.start.bpm, self.from_seconds, to),
                |end| integrate_between_points(&self.start, end, self.from_seconds, to),
            )
    }

    fn seconds_at(&self, quarter_notes: f64) -> f64 {
        let into = (quarter_notes - self.from_quarter_notes).max(0.0);
        self.end.as_ref().map_or_else(
            || self.from_seconds + into / (self.start.bpm.max(f64::EPSILON) / 60.0),
            |end| solve_segment_seconds(&self.start, end, self.from_seconds, self.to_seconds, into),
        )
    }

    fn bpm_at(&self, seconds: f64) -> f64 {
        let Some(end) = &self.end else {
            return self.start.bpm;
        };
        let start_seconds = self.start.position_seconds();
        let length = end.position_seconds() - start_seconds;
        if length <= EPSILON {
            return self.start.bpm;
        }
        let u = ((seconds - start_seconds) / length).clamp(0.0, 1.0);
        let along = match self.shape() {
            TempoShape::Hold => 0.0,
            TempoShape::Linear => u,
            TempoShape::Bezier => bezier_bias(u, self.start.bezier_tension.unwrap_or(0.0)),
        };
        (end.bpm - self.start.bpm).mul_add(along, self.start.bpm)
    }
}

/// How far along a bezier ramp the tempo is at `u` (0..1 of the ramp):
/// the derivative of the engine's `bezier_shape_antiderivative`.
fn bezier_bias(u: f64, tension: f64) -> f64 {
    let t = tension.clamp(-0.999_999, 0.999_999);
    let a = ((1.0 + t.abs()) / (1.0 - t.abs()))
        .powf(0.85)
        .clamp(1.0, 12.0);
    if t >= 0.0 {
        u.powf(a)
    } else {
        1.0 - (1.0 - u).powf(a)
    }
}

/// One meter: from `from_quarter_notes` on, bars counted from where this
/// one starts.
#[derive(Debug, Clone, Copy)]
struct MeterSpan {
    from_quarter_notes: f64,
    /// The bar and beat state where it starts (its meter applied).
    cursor: MusicalCursor,
}

/// A tempo map compiled for realtime use. See the module docs.
#[derive(Debug, Clone)]
pub struct TempoPlan {
    spans: Vec<TempoSpan>,
    meters: Vec<MeterSpan>,
}

impl TempoPlan {
    /// The plan of `engine`'s map (default tempo `default_bpm`, meter
    /// `default_meter`).
    #[must_use]
    pub fn compile(
        engine: &TempoMapEngine,
        default_bpm: f64,
        default_meter: TimeSignature,
    ) -> Self {
        Self {
            spans: tempo_spans(engine.points(), default_bpm.max(f64::EPSILON)),
            meters: meter_spans(engine, default_meter),
        }
    }

    /// The tempo segments, in order: what a ruler draws ramps from.
    #[must_use]
    pub fn spans(&self) -> &[TempoSpan] {
        &self.spans
    }

    fn span_at_seconds(&self, seconds: f64) -> Option<&TempoSpan> {
        let after = self.spans.partition_point(|s| s.from_seconds <= seconds);
        self.spans.get(after.saturating_sub(1))
    }

    /// Quarter notes from the start at `seconds`.
    #[must_use]
    pub fn seconds_to_quarter_notes(&self, seconds: f64) -> f64 {
        if seconds <= 0.0 {
            return 0.0;
        }
        self.span_at_seconds(seconds)
            .map_or(0.0, |span| span.quarter_notes_at(seconds))
    }

    /// Seconds from the start at `quarter_notes`.
    #[must_use]
    pub fn quarter_notes_to_seconds(&self, quarter_notes: f64) -> f64 {
        if quarter_notes <= 0.0 {
            return 0.0;
        }
        let after = self
            .spans
            .partition_point(|s| s.from_quarter_notes + EPSILON < quarter_notes);
        self.spans
            .get(after.saturating_sub(1))
            .map_or(0.0, |span| span.seconds_at(quarter_notes))
    }

    /// The tempo at `seconds`, BPM: where a ramp is by then, not the
    /// step value of the point before it (`TempoMapEngine::tempo_at`).
    #[must_use]
    pub fn bpm_at(&self, seconds: f64) -> f64 {
        self.span_at_seconds(seconds.max(0.0))
            .map_or(0.0, |span| span.bpm_at(seconds))
    }

    /// `quarter_notes` as bar, beat (both 1-based, the beat in the meter's
    /// unit) and how far into the beat.
    #[must_use]
    pub fn bar_beat(&self, quarter_notes: f64) -> (i32, i32, f64) {
        if quarter_notes <= 0.0 {
            return (1, 1, 0.0);
        }
        let after = self
            .meters
            .partition_point(|m| m.from_quarter_notes + EPSILON < quarter_notes);
        let Some(meter) = self.meters.get(after.saturating_sub(1)) else {
            return (1, 1, 0.0);
        };
        let mut cursor = meter.cursor;
        cursor.advance(quarter_notes - meter.from_quarter_notes);
        cursor.as_parts()
    }

    /// Bar `bar`, beat `beat` (1-based) and `fraction` of it, in quarter
    /// notes from the start.
    #[must_use]
    pub fn bar_beat_to_quarter_notes(&self, bar: i32, beat: i32, fraction: f64) -> f64 {
        let target_bar = bar.saturating_sub(1).max(0);
        let into_beats = f64::from(beat.saturating_sub(1).max(0)) + fraction.max(0.0);
        let after = self
            .meters
            .partition_point(|m| m.cursor.measure <= target_bar);
        let Some(meter) = self.meters.get(after.saturating_sub(1)) else {
            return 0.0;
        };
        let cursor = meter.cursor;
        let bars_in = f64::from(target_bar.saturating_sub(cursor.measure));
        let from_start = bars_in.mul_add(
            cursor.quarter_notes_per_measure,
            into_beats * cursor.quarter_notes_per_display_beat,
        ) - cursor.quarter_offset;
        let at = meter.from_quarter_notes + from_start.max(0.0);
        // A position past this meter's end (the rest of a bar a meter
        // change cut short) is where the next meter starts.
        self.meters
            .get(after)
            .map_or(at, |next| at.min(next.from_quarter_notes))
    }
}

fn tempo_spans(points: &[TempoPoint], default_bpm: f64) -> Vec<TempoSpan> {
    let held = |from: f64, to: f64, quarter_notes: f64, bpm: f64| TempoSpan {
        from_seconds: from,
        to_seconds: to,
        from_quarter_notes: quarter_notes,
        start: TempoPoint::from_seconds(from, bpm),
        end: None,
    };
    let Some(first) = points.first() else {
        return vec![held(0.0, f64::INFINITY, 0.0, default_bpm)];
    };
    let mut spans = Vec::with_capacity(points.len().saturating_add(1));
    let first_seconds = first.position_seconds().max(0.0);
    let mut quarter_notes = if first_seconds > 0.0 {
        spans.push(held(0.0, first_seconds, 0.0, default_bpm));
        first_seconds * default_bpm / 60.0
    } else {
        0.0
    };
    for (index, start) in points.iter().enumerate() {
        let next = points.get(index.saturating_add(1));
        let from = start.position_seconds().max(0.0);
        let to = next.map_or(f64::INFINITY, TempoPoint::position_seconds);
        // Two points at one place: the later one is the tempo there.
        if to <= from {
            continue;
        }
        spans.push(TempoSpan {
            from_seconds: from,
            to_seconds: to,
            from_quarter_notes: quarter_notes,
            start: start.clone(),
            end: next.cloned(),
        });
        if let Some(end) = next {
            quarter_notes += integrate_between_points(start, end, from, to);
        }
    }
    spans
}

/// The meters, walked as the engine walks them: a change mid-bar starts a
/// new bar (`MusicalCursor::apply_time_signature`).
fn meter_spans(engine: &TempoMapEngine, default_meter: TimeSignature) -> Vec<MeterSpan> {
    let mut cursor = MusicalCursor::new(signature_metric(default_meter));
    let mut spans = vec![MeterSpan {
        from_quarter_notes: 0.0,
        cursor,
    }];
    let mut previous = 0.0;
    for (at, metric) in engine.time_signature_changes_qn() {
        if at <= previous + EPSILON {
            // At the start (or where the last change was): it is the
            // meter from there.
            cursor.apply_time_signature(metric);
            if let Some(last) = spans.last_mut() {
                last.cursor = cursor;
            }
            continue;
        }
        cursor.advance(at - previous);
        cursor.apply_time_signature(metric);
        previous = at;
        spans.push(MeterSpan {
            from_quarter_notes: at,
            cursor,
        });
    }
    spans
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::indexing_slicing
)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::*;
    use crate::time::{Position, PositionInSeconds};

    /// Counts this thread's allocations, so a test can say a stretch of
    /// code made none.
    struct Counting;

    thread_local! {
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    // SAFETY: forwards to the system allocator unchanged; only counts.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
            // SAFETY: the caller's contract, passed on.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller's contract, passed on.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static COUNTING: Counting = Counting;

    fn allocations() -> usize {
        ALLOCATIONS.with(Cell::get)
    }

    /// A deterministic pseudo-random stream (no dependency, same maps on
    /// every run).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f64) / ((1_u64 << 53) as f64)
        }

        fn below(&mut self, n: usize) -> usize {
            ((self.next() * n as f64) as usize).min(n - 1)
        }
    }

    fn point(seconds: f64, bpm: f64, shape: i32) -> TempoPoint {
        TempoPoint {
            position: Position::from_time(PositionInSeconds::from_seconds(seconds)),
            bpm,
            shape: Some(shape),
            ..TempoPoint::default()
        }
    }

    fn random_map(rng: &mut Lcg) -> (TempoMapEngine, f64, TimeSignature) {
        let meters = [(4, 4), (3, 4), (6, 8), (7, 8), (5, 4), (2, 2)];
        let default_bpm = rng.next().mul_add(200.0, 40.0);
        let (n, d) = meters[rng.below(meters.len())];
        let default_meter = TimeSignature::new(n, d);
        let mut points = Vec::new();
        let mut at = rng.next() * 4.0;
        for _ in 0..rng.below(9) {
            let shape = [0, 1, 5][rng.below(3)];
            let mut p = point(at, rng.next().mul_add(200.0, 40.0), shape);
            if shape == 5 {
                p.bezier_tension = Some(rng.next().mul_add(2.0, -1.0));
            }
            if rng.below(3) == 0 {
                let (n, d) = meters[rng.below(meters.len())];
                p.time_signature = Some(TimeSignature::new(n, d));
            }
            points.push(p);
            // Sometimes two points at one place.
            at += if rng.below(8) == 0 {
                0.0
            } else {
                rng.next() * 20.0
            };
        }
        (
            TempoMapEngine::new(default_bpm, default_meter, points),
            default_bpm,
            default_meter,
        )
    }

    #[test]
    fn the_plan_agrees_with_the_engine_on_random_maps() {
        let mut rng = Lcg(0x5e55_1013);
        for map in 0..400 {
            let (engine, bpm, meter) = random_map(&mut rng);
            let plan = TempoPlan::compile(&engine, bpm, meter);
            for _ in 0..60 {
                let seconds = rng.next() * 180.0;
                let qn = engine.seconds_to_quarter_notes(seconds);
                let planned = plan.seconds_to_quarter_notes(seconds);
                assert!(
                    (qn - planned).abs() < 1e-9,
                    "map {map}: {seconds} s: engine {qn} qn, plan {planned}"
                );
                let back = engine.quarter_notes_to_seconds(qn);
                let planned_back = plan.quarter_notes_to_seconds(qn);
                assert!(
                    (back - planned_back).abs() < 1e-6,
                    "map {map}: {qn} qn: engine {back} s, plan {planned_back}"
                );
                assert!(
                    (planned_back - seconds).abs() < 1e-6,
                    "map {map}: {seconds} s round trip: {planned_back}"
                );
                let (bar, beat, fraction) = engine.quarter_notes_to_musical(qn);
                let (pbar, pbeat, pfraction) = plan.bar_beat(qn);
                assert!(
                    (bar, beat) == (pbar, pbeat) && (fraction - pfraction).abs() < 1e-9,
                    "map {map}: {qn} qn: engine {bar}.{beat}+{fraction}, plan {pbar}.{pbeat}+{pfraction}"
                );
                let to_qn = engine.musical_to_quarter_notes(bar, beat, fraction);
                let planned_qn = plan.bar_beat_to_quarter_notes(bar, beat, fraction);
                assert!(
                    (to_qn - planned_qn).abs() < 1e-9,
                    "map {map}: {bar}.{beat}+{fraction}: engine {to_qn} qn, plan {planned_qn}"
                );
            }
        }
    }

    #[test]
    fn a_ramp_is_a_ramp() {
        let engine = TempoMapEngine::new(
            60.0,
            TimeSignature::new(4, 4),
            vec![point(0.0, 60.0, 0), point(10.0, 120.0, 1)],
        );
        let plan = TempoPlan::compile(&engine, 60.0, TimeSignature::new(4, 4));
        assert!((plan.seconds_to_quarter_notes(10.0) - 15.0).abs() < 1e-9);
        assert!((plan.bpm_at(5.0) - 90.0).abs() < 1e-9);
        assert!((plan.bpm_at(20.0) - 120.0).abs() < 1e-9);
        let ramp = &plan.spans()[0];
        assert_eq!(ramp.shape(), TempoShape::Linear);
        assert_eq!((ramp.bpm_start(), ramp.bpm_end()), (60.0, 120.0));
        assert_eq!(plan.spans()[1].shape(), TempoShape::Hold);
    }

    #[test]
    fn a_question_allocates_nothing() {
        let mut rng = Lcg(7);
        let (engine, bpm, meter) = random_map(&mut rng);
        let plan = TempoPlan::compile(&engine, bpm, meter);
        let before = allocations();
        let mut sum = 0.0;
        for i in 0..10_000 {
            let seconds = f64::from(i) * 0.017;
            let qn = plan.seconds_to_quarter_notes(seconds);
            sum += plan.quarter_notes_to_seconds(qn) + plan.bpm_at(seconds);
            let (bar, beat, fraction) = plan.bar_beat(qn);
            sum += plan.bar_beat_to_quarter_notes(bar, beat, fraction);
        }
        assert_eq!(allocations(), before, "a plan's question allocated");
        assert!(sum.is_finite());
        // The engine's bar and beat conversion, for contrast, does.
        let before = allocations();
        let _ = engine.quarter_notes_to_musical(10.0);
        assert!(
            allocations() > before || engine.points().iter().all(|p| p.time_signature.is_none()),
            "the engine's meter walk was expected to allocate"
        );
    }
}
