//! Quantizing and aligning a hit list, and why both are previewed.
//!
//! The detection engines already exist in the expression editor; what
//! was missing is the **write path**. These take a hit list (stretch
//! markers on every mic of a piece — see [`crate::hits`]), work out
//! where each hit should go, and write the result to every mic in one
//! undo step.
//!
//! # Why a plan, and not a verb
//!
//! Every move here is computed first and applied second, so the window
//! can draw what a quantize *would* do before anything moves. That is
//! not a nicety: quantizing drums is destructive to feel, and the
//! difference between 60% and 80% strength is something an engineer
//! judges by looking at it rather than by typing a number and undoing.
//!
//! # Why fills are protected
//!
//! A fill is a run of hits far denser than the groove around it, and
//! its *internal* timing is the performance. Snapping each hit of a
//! fill to the grid straightens exactly the thing that made it a fill.
//! So a fill is moved as one piece — every hit shifted by the same
//! amount, the amount its first hit needed — which puts the fill where
//! the grid wants it and leaves the playing intact.

use crate::hits::Hit;

/// How hard to pull a hit toward the grid.
///
/// A fraction rather than a flag: 1.0 puts a hit exactly on the line,
/// 0.0 leaves it where it was played, and the useful settings are in
/// between — a kit pulled all the way to the grid stops breathing.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Strength(f64);

impl Strength {
    /// A strength, clamped to `0.0..=1.0`.
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Leave everything where it was played.
    pub const NONE: Self = Self(0.0);
    /// Put every hit exactly on the line.
    pub const FULL: Self = Self(1.0);

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

/// The grid a quantize snaps to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Seconds between lines.
    pub interval: f64,
    /// Where the first line sits.
    pub origin: f64,
}

impl Grid {
    /// The line nearest a position.
    #[must_use]
    pub fn nearest(&self, at: f64) -> f64 {
        if self.interval <= 0.0 {
            return at;
        }
        let steps = ((at - self.origin) / self.interval).round();
        self.origin + steps * self.interval
    }
}

/// How a quantize would move one hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    /// The hit's index in the list — the same index on every mic,
    /// because the list was written to all of them at once.
    pub index: usize,
    pub from: f64,
    pub to: f64,
    /// Whether this hit is part of a protected fill, and so moved with
    /// its neighbours rather than onto a line of its own.
    pub in_fill: bool,
}

impl Move {
    /// How far the hit travels. Zero means it is already where it
    /// should be, which is worth knowing: a quantize that moves nothing
    /// should write nothing.
    #[must_use]
    pub fn distance(&self) -> f64 {
        (self.to - self.from).abs()
    }
}

/// Which hits belong to fills.
///
/// A fill is a run of at least `min_run` hits whose gaps are shorter
/// than `density` times the **groove's** spacing — and the groove is
/// taken as the upper quartile of the take's gaps, not the median.
///
/// The median is the obvious choice and it is wrong, for a reason worth
/// keeping: a fill has more hits in it than the bar it sits in, so once
/// a take contains any real fill the median gap IS the fill's own
/// spacing, and nothing is ever dense enough to beat it. The upper
/// quartile still lands on the groove when fills outnumber it.
///
/// Measuring against the take's own playing rather than a fixed number
/// of milliseconds is what lets one setting work on a ballad and on a
/// fast tune.
#[must_use]
pub fn fills(hits: &[Hit], density: f64, min_run: usize) -> Vec<bool> {
    let mut out = vec![false; hits.len()];
    if hits.len() < 2 {
        return out;
    }
    let mut gaps: Vec<f64> = hits.windows(2).map(|w| w[1].at - w[0].at).collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Integer arithmetic, so there is no cast to explain away: the
    // three-quarter index of a list is `len * 3 / 4`, and `min` keeps
    // it in range for a list of one.
    let upper = gaps[(gaps.len() * 3 / 4).min(gaps.len() - 1)];
    if upper <= 0.0 {
        return out;
    }
    let tight = upper * density;

    let mut run_start = 0;
    for i in 0..gaps.len() {
        let close = hits[i + 1].at - hits[i].at < tight;
        if !close {
            mark_run(&mut out, run_start, i, min_run);
            run_start = i + 1;
        }
    }
    mark_run(&mut out, run_start, gaps.len(), min_run);
    out
}

/// Mark `[from, to]` as a fill when it is long enough to be one.
fn mark_run(out: &mut [bool], from: usize, to: usize, min_run: usize) {
    if to.saturating_sub(from) + 1 >= min_run {
        for flag in out.iter_mut().take(to + 1).skip(from) {
            *flag = true;
        }
    }
}

/// What a quantize would do, without doing it.
///
/// Returns a move per hit, in list order. A hit already on its line
/// still gets a `Move` with a zero distance, so a caller can count what
/// would change rather than inferring it from a shorter list.
///
/// r[impl flow.drums.editing.quantize]
#[must_use]
pub fn plan(hits: &[Hit], grid: Grid, strength: Strength, protect_fills: bool) -> Vec<Move> {
    let in_fill = if protect_fills {
        fills(hits, 0.6, 3)
    } else {
        vec![false; hits.len()]
    };

    let mut moves: Vec<Move> = Vec::with_capacity(hits.len());
    let mut i = 0;
    while i < hits.len() {
        if in_fill[i] {
            // The whole fill travels by what its FIRST hit needed, so
            // the playing inside it survives intact.
            let start = i;
            while i < hits.len() && in_fill[i] {
                i += 1;
            }
            let lead = hits[start].at;
            let shift = (grid.nearest(lead) - lead) * strength.get();
            for (offset, hit) in hits[start..i].iter().enumerate() {
                moves.push(Move {
                    index: start + offset,
                    from: hit.at,
                    to: hit.at + shift,
                    in_fill: true,
                });
            }
            continue;
        }
        let hit = hits[i];
        let line = grid.nearest(hit.at);
        moves.push(Move {
            index: i,
            from: hit.at,
            to: hit.at + (line - hit.at) * strength.get(),
            in_fill: false,
        });
        i += 1;
    }
    moves
}

/// What an alignment would do: retime a take's hits onto a reference's.
///
/// Each hit moves to the reference hit nearest it, but only when that
/// one is within `window` — beyond it the two takes are playing
/// different things, and dragging a hit across a beat to the nearest
/// reference is worse than leaving it alone.
///
/// r[impl flow.drums.editing.align-hits]
#[must_use]
pub fn align_to(hits: &[Hit], reference: &[Hit], window: f64) -> Vec<Move> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            let nearest = reference
                .iter()
                .map(|r| r.at)
                .min_by(|a, b| {
                    (a - hit.at)
                        .abs()
                        .partial_cmp(&(b - hit.at).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .filter(|at| (at - hit.at).abs() <= window);
            Move {
                index,
                from: hit.at,
                to: nearest.unwrap_or(hit.at),
                in_fill: false,
            }
        })
        .collect()
}

/// Apply a plan to a hit list.
#[must_use]
pub fn applied(hits: &[Hit], moves: &[Move]) -> Vec<Hit> {
    let mut out = hits.to_vec();
    for m in moves {
        if let Some(hit) = out.get_mut(m.index) {
            hit.at = m.to;
        }
    }
    out
}
