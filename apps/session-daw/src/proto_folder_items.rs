//! PROTOTYPE — folder items. Throwaway code for issue #27; lives on the
//! `prototype/folder-items` branch and nowhere else.
//!
//! The question: how should a folder row's item look and behave when
//! it is summed from its children — the kit's kick, snare and toms each
//! in their role colour layered in one waveform
//! (`flow.drums.comping.folder-item-colours`), cached the way the docked
//! stack's picture is, a muted child dropping out of the sum and a
//! hidden one staying in; and, with the kit in comping, one lane per
//! take on the folder with a comp lane on top
//! (`flow.drums.comping.folder-lanes`).
//!
//! Peaks are SIMULATED here — a small groove rendered per mic as
//! `(min, max)` pairs at the `.reapeaks` level-0 ratio (160 samples per
//! peak at 48 kHz), the same shape `Peaks::take_peaks` returns — so
//! the fold below is written against the real wire contract and a
//! swap to standalone's peaks is a loading change, not a redraw.
//!
//! Rendered by `bench` with `FTS_BENCH_FOLDER_ITEMS=<out.png>`; see
//! `just daw-folder-items`.

use anyrender::{PaintScene, Scene};
use expression_editor_core::kit::LaneRole;
use vello::kurbo::{Affine, BezPath, Line, Rect, Stroke};
use vello::peniko::{Color, Fill};

use crate::arrangement::{Palette, submit_command};
use crate::text::Font;

/// The peak ratio the `.reapeaks` finest level uses.
pub const SAMPLES_PER_PEAK: u32 = 160;
/// The source rate the ratio is measured in.
pub const SAMPLE_RATE: f64 = 48_000.0;
const BPM: f64 = 120.0;
const BARS: usize = 8;

// ────────────────────────────────────────────────────────────────────
// Data — what a folder item is made of
// ────────────────────────────────────────────────────────────────────

/// One take's peaks in take time, as `TakePeakData` carries them:
/// `(min, max)` per peak, linear −1..1.
#[derive(Clone, Debug)]
pub struct TakePeaks {
    pub pairs: Vec<(f32, f32)>,
    pub samples_per_peak: u32,
    pub sample_rate: f64,
}

impl TakePeaks {
    fn secs_per_peak(&self) -> f64 {
        f64::from(self.samples_per_peak) / self.sample_rate
    }

    /// The `(min, max)` over every peak touching `[t0, t1)` — the
    /// window-max rule `ReaPeaks::columns` draws with. `None` when the
    /// window is past the take.
    fn window(&self, t0: f64, t1: f64) -> Option<(f32, f32)> {
        let spp = self.secs_per_peak();
        let from = crate::num::index((t0 / spp).floor());
        let to = crate::num::index((t1 / spp).ceil()).max(from.saturating_add(1));
        let slice = self.pairs.get(from..to.min(self.pairs.len()))?;
        slice
            .iter()
            .copied()
            .reduce(|(lo, hi), (mn, mx)| (lo.min(mn), hi.max(mx)))
    }
}

/// A child of the folder: one mic, with its role and one take of
/// peaks per recorded pass.
#[derive(Clone, Debug)]
pub struct Child {
    pub name: &'static str,
    pub role: LaneRole,
    /// Muted: out of the sum. The folder hears what the mix hears.
    pub muted: bool,
    /// Hidden from the TCP (collapsed under the folder): still in the
    /// sum — hiding is a view, muting is a signal.
    pub hidden: bool,
    pub takes: Vec<TakePeaks>,
}

/// The folder: its children and a revision the cache keys on. The
/// folder owns no audio — its item is a view of the children's.
#[derive(Clone, Debug)]
pub struct Folder {
    pub children: Vec<Child>,
    pub take_count: usize,
    pub length_secs: f64,
    /// Bumped by anything that changes a child's peaks or mute.
    pub revision: u64,
}

impl Folder {
    /// A bit per child, set when it is out of the sum.
    fn mute_mask(&self) -> u64 {
        self.children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.muted)
            .fold(0_u64, |m, (i, _)| m | 1_u64.checked_shl(u32::try_from(i).unwrap_or(63)).unwrap_or(0))
    }

    pub fn set_muted(&mut self, name: &str, muted: bool) {
        if let Some(c) = self.children.iter_mut().find(|c| c.name == name) {
            c.muted = muted;
            self.revision = self.revision.saturating_add(1);
        }
    }

    pub fn set_hidden(&mut self, name: &str, hidden: bool) {
        if let Some(c) = self.children.iter_mut().find(|c| c.name == name) {
            c.hidden = hidden;
            // No revision bump: the sum did not change, only the TCP.
        }
    }
}

/// Where a role sits in a column's role array.
fn role_slot(role: LaneRole) -> usize {
    LaneRole::ALL.iter().position(|r| *r == role).unwrap_or(0)
}

/// One column of the folded picture: per role, the `(min, max)` over
/// every unmuted child of that role — and `None` where the role has
/// no audible child.
#[derive(Clone, Copy, Debug, Default)]
pub struct FoldColumn {
    pub roles: [Option<(f32, f32)>; 4],
}

impl FoldColumn {
    /// The outer envelope: min of the minima, max of the maxima.
    fn outer(&self) -> Option<(f32, f32)> {
        self.roles
            .iter()
            .flatten()
            .copied()
            .reduce(|(lo, hi), (mn, mx)| (lo.min(mn), hi.max(mx)))
    }
}

/// How the children are summed per column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SumRule {
    /// `min(min)` / `max(max)` across children — what REAPER draws for
    /// the same lanes, never narrower than any child.
    MinMax,
    /// Mean of the children's `|peak|`, normalised — the expression
    /// editor's lane rule, here for comparison.
    Mean,
}

/// Fold `take` of every unmuted child onto `cols` columns of
/// `secs_per_col`.
#[must_use]
pub fn fold(folder: &Folder, take: usize, cols: usize, secs_per_col: f64, rule: SumRule) -> Vec<FoldColumn> {
    let mut out = vec![FoldColumn::default(); cols];
    // Per role: how many children fed it, for the mean.
    let mut fed = [0.0_f64; 4];
    for child in folder.children.iter().filter(|c| !c.muted) {
        let Some(peaks) = child.takes.get(take) else { continue };
        let slot = role_slot(child.role);
        if let Some(n) = fed.get_mut(slot) {
            *n += 1.0;
        }
        for (i, col) in out.iter_mut().enumerate() {
            let t0 = crate::num::coord(i) * secs_per_col;
            let Some((mn, mx)) = peaks.window(t0, t0 + secs_per_col) else { continue };
            let Some(cell) = col.roles.get_mut(slot) else { continue };
            *cell = Some(match (*cell, rule) {
                (None, _) => (mn, mx),
                (Some((lo, hi)), SumRule::MinMax) => (lo.min(mn), hi.max(mx)),
                (Some((lo, hi)), SumRule::Mean) => (lo + mn, hi + mx),
            });
        }
    }
    if rule == SumRule::Mean {
        // Divide by the member count, then normalise the loudest column
        // to 1 — `summed_columns` in the editor's stack.
        let mut loudest = 0.0_f32;
        for col in &mut out {
            for (slot, cell) in col.roles.iter_mut().enumerate() {
                let n = fed.get(slot).copied().unwrap_or(1.0).max(1.0);
                if let Some((lo, hi)) = cell {
                    *lo /= n.to_f32();
                    *hi /= n.to_f32();
                    loudest = loudest.max(hi.abs()).max(lo.abs());
                }
            }
        }
        if loudest > 0.0 {
            for col in &mut out {
                for cell in col.roles.iter_mut().flatten() {
                    cell.0 /= loudest;
                    cell.1 /= loudest;
                }
            }
        }
    }
    out
}

/// `f64 -> f32` without an `as`.
trait ToF32 {
    fn to_f32(self) -> f32;
}
impl ToF32 for f64 {
    fn to_f32(self) -> f32 {
        // Round-trips the range a peak ever spans; a `From` impl does
        // not exist in this direction.
        let s = format!("{self:.6}");
        s.parse().unwrap_or(0.0)
    }
}

// ────────────────────────────────────────────────────────────────────
// Cache — the picture kept until what it depends on changes
// ────────────────────────────────────────────────────────────────────

/// Everything the folded picture depends on. Same idea as the docked
/// stack's `ViewKey`: a frame compares the key and replays the scene
/// when nothing changed, and rebuilds when something did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PictureKey {
    pub revision: u64,
    pub mute_mask: u64,
    pub take: usize,
    /// The zoom, bucketed to whole columns per second — the fold is
    /// on a column grid, so a zoom that moves the grid needs a refold,
    /// and one that does not can reuse it.
    pub cols_per_sec: u32,
    pub style: Style,
    pub rule: SumRule,
}

#[derive(Default)]
pub struct PictureCache {
    entries: Vec<(PictureKey, Scene)>,
    pub hits: usize,
    pub misses: usize,
}

impl PictureCache {
    /// The scene for `key`, built by `build` when it is not there.
    pub fn get_or_build(&mut self, key: PictureKey, build: impl FnOnce() -> Scene) -> &Scene {
        let found = self.entries.iter().position(|(k, _)| *k == key);
        if found.is_some() {
            self.hits = self.hits.saturating_add(1);
        } else {
            self.misses = self.misses.saturating_add(1);
            self.entries.push((key, build()));
        }
        let i = found.unwrap_or_else(|| self.entries.len().saturating_sub(1));
        self.entries.get(i).map_or_else(|| unreachable_scene(), |(_, s)| s)
    }

    /// Drop every picture of a folder whose children changed.
    pub fn invalidate(&mut self, revision: u64) {
        self.entries.retain(|(k, _)| k.revision == revision);
    }
}

fn unreachable_scene() -> &'static Scene {
    static EMPTY: std::sync::OnceLock<Scene> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Scene::new)
}

// ────────────────────────────────────────────────────────────────────
// Drawing — the variants under comparison
// ────────────────────────────────────────────────────────────────────

/// How the per-role folds are put in one item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// One envelope, the outer min/max, in the track colour. The
    /// baseline: what REAPER draws.
    Grey,
    /// Every role's own envelope, back to front in a fixed order
    /// (other, toms, snare, kick), translucent.
    Layered,
    /// Every role's own envelope, opaque, the loudest role of each
    /// column drawn first so a quieter role always shows inside it.
    Sorted,
    /// The outer envelope, coloured per column by the loudest role.
    Winner,
    /// Each role's height stacked on the last — a sum, not a max — so
    /// the item's height is the kit's total energy.
    Stacked,
    /// The pieces: `Other` (hats, cymbals, rooms — heard, not a
    /// piece) as the outer envelope in the dim track colour, then
    /// toms, snare and kick opaque on top of it, kick last.
    Pieces,
    /// `Pieces`, with a bleed gate: a piece's column is drawn only
    /// where its fold is at least `BLEED_GATE` of the column's outer
    /// envelope, so a kick hit heard in the snare mics does not paint
    /// a snare-coloured shadow under every kick.
    Gated,
}

/// The fraction of the outer envelope a piece must reach to be
/// painted in `Style::Gated`.
const BLEED_GATE: f32 = 0.45;

impl Style {
    pub const ALL: [Self; 7] = [Self::Grey, Self::Layered, Self::Sorted, Self::Winner, Self::Stacked, Self::Pieces, Self::Gated];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Grey => "grey — outer min/max, track colour",
            Self::Layered => "layered — role envelopes back to front, translucent",
            Self::Sorted => "sorted — per column, loudest role behind, opaque",
            Self::Winner => "winner — one envelope, column takes the loudest role's colour",
            Self::Stacked => "stacked — roles' heights added, a sum not a max",
            Self::Pieces => "pieces — Other dim in the track colour, toms/snare/kick opaque on it",
            Self::Gated => "gated — pieces, drawn only where the piece is 45% of the outer envelope",
        }
    }
}

fn hex(s: &str) -> Color {
    let byte = |i: usize| {
        s.get(i..i.saturating_add(2))
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .unwrap_or(0)
    };
    Color::from_rgb8(byte(1), byte(3), byte(5))
}

fn role_color(role: LaneRole) -> Color {
    hex(role.color())
}

/// The mirrored envelope of one role across `cols`, as one closed path.
fn envelope(cols: &[FoldColumn], pick: impl Fn(&FoldColumn) -> Option<(f32, f32)>, x0: f64, col_w: f64, mid: f64, half: f64) -> BezPath {
    let mut path = BezPath::new();
    let at = |i: usize| crate::num::coord(i).mul_add(col_w, x0);
    let amp = |v: f32| f64::from(v).clamp(-1.0, 1.0) * half;
    path.move_to((x0, mid));
    for (i, col) in cols.iter().enumerate() {
        let hi = pick(col).map_or(0.0, |(_, hi)| amp(hi));
        path.line_to((at(i), mid - hi));
        path.line_to((at(i.saturating_add(1)), mid - hi));
    }
    for (i, col) in cols.iter().enumerate().rev() {
        let lo = pick(col).map_or(0.0, |(lo, _)| amp(lo));
        path.line_to((at(i.saturating_add(1)), mid - lo));
        path.line_to((at(i), mid - lo));
    }
    path.close_path();
    path
}

fn span(cell: Option<(f32, f32)>) -> f32 {
    cell.map_or(0.0, |(lo, hi)| hi - lo)
}

/// Draw one folder item of `cols` into `scene` at `(x0, top)`, `w × h`.
/// Where an item is drawn: its left edge, top, width and height.
#[derive(Clone, Copy)]
struct Box {
    x0: f64,
    top: f64,
    width: f64,
    height: f64,
}

fn draw_item(scene: &mut Scene, cols: &[FoldColumn], style: Style, track: Color, at: Box) {
    let Box { x0, top, width, height } = at;
    let col_w = width / crate::num::coord(cols.len().max(1));
    let mid = top + height / 2.0;
    let half = height / 2.0 - 1.0;
    let fill = |scene: &mut Scene, color: Color, path: &BezPath| {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, path);
    };
    // The body, dimmed, as the arrangement draws every item.
    scene.fill(Fill::NonZero, Affine::IDENTITY, track.multiply_alpha(0.22), None, &Rect::new(x0, top, x0 + width, top + height));
    match style {
        Style::Grey => {
            let path = envelope(cols, FoldColumn::outer, x0, col_w, mid, half);
            fill(scene, track, &path);
        }
        Style::Layered => {
            for role in LaneRole::ALL {
                let slot = role_slot(role);
                let path = envelope(cols, |c| c.roles.get(slot).copied().flatten(), x0, col_w, mid, half);
                fill(scene, role_color(role).multiply_alpha(0.78), &path);
            }
        }
        Style::Sorted => {
            // Per column the roles are drawn loudest first, so each
            // column is its own little stack of opaque rectangles.
            for (i, col) in cols.iter().enumerate() {
                let mut order: Vec<usize> = (0..4).collect();
                order.sort_by(|a, b| {
                    let sa = span(col.roles.get(*a).copied().flatten());
                    let sb = span(col.roles.get(*b).copied().flatten());
                    sb.total_cmp(&sa)
                });
                let x = crate::num::coord(i).mul_add(col_w, x0);
                for slot in order {
                    let Some((lo, hi)) = col.roles.get(slot).copied().flatten() else { continue };
                    let role = LaneRole::ALL.get(slot).copied().unwrap_or(LaneRole::Other);
                    let rect = Rect::new(x, f64::from(hi).mul_add(-half, mid), x + col_w, f64::from(lo).mul_add(-half, mid));
                    scene.fill(Fill::NonZero, Affine::IDENTITY, role_color(role), None, &rect);
                }
            }
        }
        Style::Winner => {
            for (i, col) in cols.iter().enumerate() {
                let Some((lo, hi)) = col.outer() else { continue };
                let winner = (0..4)
                    .max_by(|a, b| span(col.roles.get(*a).copied().flatten()).total_cmp(&span(col.roles.get(*b).copied().flatten())))
                    .and_then(|s| LaneRole::ALL.get(s).copied())
                    .unwrap_or(LaneRole::Other);
                let x = crate::num::coord(i).mul_add(col_w, x0);
                let rect = Rect::new(x, f64::from(hi).mul_add(-half, mid), x + col_w, f64::from(lo).mul_add(-half, mid));
                scene.fill(Fill::NonZero, Affine::IDENTITY, role_color(winner), None, &rect);
            }
        }
        Style::Pieces | Style::Gated => {
            let outer = envelope(cols, FoldColumn::outer, x0, col_w, mid, half);
            fill(scene, track.multiply_alpha(0.45), &outer);
            let gate = if style == Style::Gated { BLEED_GATE } else { 0.0 };
            for role in [LaneRole::Toms, LaneRole::Snare, LaneRole::Kick] {
                let slot = role_slot(role);
                let pick = |c: &FoldColumn| {
                    let cell = c.roles.get(slot).copied().flatten()?;
                    let limit = span(c.outer()) * gate;
                    (span(Some(cell)) >= limit && span(Some(cell)) > 0.02).then_some(cell)
                };
                let path = envelope(cols, pick, x0, col_w, mid, half);
                fill(scene, role_color(role).multiply_alpha(0.92), &path);
            }
        }
        Style::Stacked => {
            // Heights added, kick at the centre, other at the rim;
            // scaled so a column never leaves the lane.
            let total = |c: &FoldColumn| c.roles.iter().map(|cell| f64::from(span(*cell))).sum::<f64>();
            let loudest = cols.iter().map(total).fold(0.0_f64, f64::max).max(1e-6);
            for (i, col) in cols.iter().enumerate() {
                let x = crate::num::coord(i).mul_add(col_w, x0);
                let mut acc = 0.0_f64;
                for role in LaneRole::ALL.iter().rev() {
                    let s = f64::from(span(col.roles.get(role_slot(*role)).copied().flatten())) / loudest;
                    let (a, b) = (acc, acc + s);
                    acc = b;
                    let rect = Rect::new(x, b.mul_add(-half, mid), x + col_w, a.mul_add(-half, mid));
                    scene.fill(Fill::NonZero, Affine::IDENTITY, role_color(*role), None, &rect);
                    let rect = Rect::new(x, a.mul_add(half, mid), x + col_w, b.mul_add(half, mid));
                    scene.fill(Fill::NonZero, Affine::IDENTITY, role_color(*role), None, &rect);
                }
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// The comp — one lane per take, a comp lane on top
// ────────────────────────────────────────────────────────────────────

/// One region of the comp: `take` between `from` and `to` seconds.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub take: usize,
    pub from: f64,
    pub to: f64,
}

pub struct Comp {
    pub regions: Vec<Region>,
    /// Boundary crossfade, seconds — the session-wide default.
    pub crossfade: f64,
}

// ────────────────────────────────────────────────────────────────────
// Simulation — a groove per mic, per take
// ────────────────────────────────────────────────────────────────────

struct Hit {
    at: f64,
    vel: f64,
}

/// The kit's pieces, as sources each mic hears.
struct Groove {
    kick: Vec<Hit>,
    snare: Vec<Hit>,
    hat: Vec<Hit>,
    crash: Vec<Hit>,
    toms: [Vec<Hit>; 3],
}

fn hash(a: usize, b: usize) -> f64 {
    let x = a.wrapping_mul(0x9E37_79B9).wrapping_add(b.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0x27D4_EB2F);
    crate::num::coord(x % 10_007) / 10_007.0
}

impl Groove {
    fn tom(&self, i: usize) -> &[Hit] {
        self.toms.get(i).map_or(&[], Vec::as_slice)
    }

    fn play(take: usize) -> Self {
        let beat = 60.0 / BPM;
        let mut kick = Vec::new();
        let mut snare = Vec::new();
        let mut hat = Vec::new();
        let mut crash = Vec::new();
        let mut toms: [Vec<Hit>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for bar in 0..BARS {
            let t = crate::num::coord(bar) * 4.0 * beat;
            let vary = |k: usize| 0.15_f64.mul_add(hash(take.saturating_add(1).saturating_mul(31), bar.saturating_mul(7).saturating_add(k)), 0.85);
            // Kick on 1 and 3; the "and of 3" in even bars; take 2
            // pushes an extra one on 4& in bars 3 and 7, take 3 drops
            // the kick of bar 6 beat 3 — a take you would comp around.
            kick.push(Hit { at: t, vel: vary(1) });
            if !(take == 2 && bar == 5) {
                kick.push(Hit { at: 2.0_f64.mul_add(beat, t), vel: vary(2) });
            }
            if bar % 2 == 1 {
                kick.push(Hit { at: 2.5_f64.mul_add(beat, t), vel: 0.7 * vary(3) });
            }
            if take == 1 && (bar == 2 || bar == 6) {
                kick.push(Hit { at: 3.5_f64.mul_add(beat, t), vel: 0.8 });
            }
            // Snare on 2 and 4; ghosts in take 3.
            snare.push(Hit { at: t + beat, vel: vary(4) });
            snare.push(Hit { at: 3.0_f64.mul_add(beat, t), vel: vary(5) });
            if take == 2 {
                snare.push(Hit { at: 1.75_f64.mul_add(beat, t), vel: 0.25 });
                snare.push(Hit { at: 3.75_f64.mul_add(beat, t), vel: 0.25 });
            }
            // Hats in eighths, a crash on bars 1 and 5.
            for k in 0..8 {
                let v = if k % 2 == 0 { 0.55 } else { 0.35 };
                hat.push(Hit { at: (crate::num::coord(k) * 0.5).mul_add(beat, t), vel: v * vary(k.saturating_add(10)) });
            }
            if bar % 4 == 0 {
                crash.push(Hit { at: t, vel: 0.9 });
            }
            // A fill on the last beat of bars 4 and 8; take 2 fills
            // bar 8 over two beats, take 3 has no fill in bar 4.
            let fills = match take {
                1 if bar == 7 => vec![(2.0, 0), (2.25, 0), (2.5, 1), (2.75, 1), (3.0, 2), (3.25, 2), (3.5, 2), (3.75, 2)],
                2 if bar == 3 => Vec::new(),
                _ if bar == 3 || bar == 7 => vec![(3.0, 0), (3.25, 1), (3.5, 2), (3.75, 2)],
                _ => Vec::new(),
            };
            for (b, tom) in fills.into_iter().map(|(b, tom): (f64, usize)| (b, tom)) {
                if let Some(list) = toms.get_mut(tom) {
                    list.push(Hit { at: b.mul_add(beat, t), vel: 0.9 });
                }
            }
        }
        Self { kick, snare, hat, crash, toms }
    }

    /// Peaks of one mic: every source it hears, each at a gain and a
    /// decay, rendered per block as an asymmetric `(min, max)`.
    fn render(sources: &[(&[Hit], f64, f64)], mic: usize) -> TakePeaks {
        let spp = f64::from(SAMPLES_PER_PEAK) / SAMPLE_RATE;
        let secs = crate::num::coord(BARS) * 4.0 * 60.0 / BPM;
        let blocks = crate::num::index((secs / spp).ceil());
        let mut pairs = Vec::with_capacity(blocks);
        let mut cursors = vec![0_usize; sources.len()];
        for b in 0..blocks {
            let t = crate::num::coord(b) * spp;
            let mut env = 0.003_f64;
            for (k, (hits, gain, decay)) in sources.iter().enumerate() {
                let Some(cur) = cursors.get_mut(k) else { continue };
                while hits.get(cur.saturating_add(1)).is_some_and(|h| h.at <= t) {
                    *cur = cur.saturating_add(1);
                }
                for h in hits.iter().take(cur.saturating_add(1)).rev().take(3) {
                    let dt = t - h.at;
                    if dt >= 0.0 {
                        env += gain * h.vel * (-dt / decay).exp();
                    }
                }
            }
            let env = env.min(1.0);
            // A waveform's halves are not mirror images: the low side
            // rides a slow wobble so the pair order is visibly used.
            let wobble = 0.35_f64.mul_add(hash(mic, b), 0.55);
            pairs.push(((-env * wobble).to_f32(), env.to_f32()));
        }
        TakePeaks { pairs, samples_per_peak: SAMPLES_PER_PEAK, sample_rate: SAMPLE_RATE }
    }
}

/// What one mic hears of a groove: sources with a gain and a decay.
type Pick = dyn Fn(&Groove) -> Vec<(&[Hit], f64, f64)>;

/// The kit: twelve mics, three takes each.
#[must_use]
pub fn kit() -> Folder {
    use LaneRole as R;
    let takes = 3;
    let grooves: Vec<Groove> = (0..takes).map(Groove::play).collect();
    let mic = |i: usize, name: &'static str, role: R, pick: &Pick| Child {
        name,
        role,
        muted: false,
        hidden: false,
        takes: grooves.iter().map(|g| Groove::render(&pick(g), i)).collect(),
    };
    let children = vec![
        mic(0, "Kick In", R::Kick, &|g| vec![(g.kick.as_slice(), 1.0, 0.12), (g.snare.as_slice(), 0.12, 0.1)]),
        mic(1, "Kick Out", R::Kick, &|g| vec![(g.kick.as_slice(), 0.8, 0.25), (g.tom(2), 0.2, 0.3)]),
        mic(2, "Kick Trig", R::Kick, &|g| vec![(g.kick.as_slice(), 1.0, 0.03)]),
        mic(3, "Snare Top", R::Snare, &|g| vec![(g.snare.as_slice(), 1.0, 0.15), (g.hat.as_slice(), 0.18, 0.04), (g.kick.as_slice(), 0.1, 0.1)]),
        mic(4, "Snare Bottom", R::Snare, &|g| vec![(g.snare.as_slice(), 0.9, 0.2), (g.kick.as_slice(), 0.15, 0.1)]),
        mic(5, "Tom 1", R::Toms, &|g| vec![(g.tom(0), 1.0, 0.3), (g.snare.as_slice(), 0.2, 0.1)]),
        mic(6, "Tom 2", R::Toms, &|g| vec![(g.tom(1), 1.0, 0.35), (g.snare.as_slice(), 0.15, 0.1)]),
        mic(7, "Floor Tom", R::Toms, &|g| vec![(g.tom(2), 1.0, 0.45), (g.kick.as_slice(), 0.2, 0.15)]),
        mic(8, "HH", R::Other, &|g| vec![(g.hat.as_slice(), 0.9, 0.05), (g.snare.as_slice(), 0.3, 0.1)]),
        mic(9, "OH L", R::Other, &|g| vec![(g.hat.as_slice(), 0.5, 0.08), (g.crash.as_slice(), 1.0, 1.2), (g.snare.as_slice(), 0.5, 0.15), (g.kick.as_slice(), 0.3, 0.1)]),
        mic(10, "OH R", R::Other, &|g| vec![(g.hat.as_slice(), 0.4, 0.08), (g.crash.as_slice(), 0.9, 1.3), (g.snare.as_slice(), 0.5, 0.15), (g.tom(0), 0.4, 0.3)]),
        mic(11, "Room", R::Other, &|g| vec![(g.kick.as_slice(), 0.5, 0.3), (g.snare.as_slice(), 0.6, 0.35), (g.crash.as_slice(), 0.6, 1.5), (g.hat.as_slice(), 0.2, 0.1)]),
    ];
    Folder {
        children,
        take_count: takes,
        length_secs: crate::num::coord(BARS) * 4.0 * 60.0 / BPM,
        revision: 1,
    }
}

// ────────────────────────────────────────────────────────────────────
// The shots
// ────────────────────────────────────────────────────────────────────

const GUTTER: f64 = 430.0;
const PAD: f64 = 12.0;
const LABEL: f32 = 12.0;

/// One row of the sheet: which take, how it is folded and drawn, and
/// the two lines of label beside it.
#[derive(Clone, Copy)]
struct RowSpec<'a> {
    take: usize,
    style: Style,
    rule: SumRule,
    h: f64,
    name: &'a str,
    note: &'a str,
    window: (f64, f64),
}

struct Sheet<'a> {
    palette: &'a Palette,
    font: &'a Font,
    width: f64,
    y: f64,
    cache: PictureCache,
}

impl Sheet<'_> {
    fn heading(&mut self, painter: &mut impl PaintScene, text: &str) {
        self.y += PAD;
        crate::tcp::glyphs(painter, self.font, self.palette.accent, text, PAD, self.y + 14.0, 14.0);
        self.y += 24.0;
    }

    fn label(&self, painter: &mut impl PaintScene, text: &str, dim: bool, y: f64) {
        let color = if dim { self.palette.text_dim } else { self.palette.text };
        crate::tcp::glyphs(painter, self.font, color, text, PAD, y, LABEL);
    }

    /// One folder row: the item across the lane, folded at this zoom.
    fn row(&mut self, painter: &mut impl PaintScene, folder: &Folder, spec: RowSpec<'_>) {
        let RowSpec { take, style, rule, h, name, note, window } = spec;
        let lane_w = self.width - GUTTER - PAD;
        let (t0, t1) = window;
        let cols = crate::num::index(lane_w.floor());
        let secs_per_col = (t1 - t0) / crate::num::coord(cols.max(1));
        let cols_per_sec = u32::try_from(crate::num::index((1.0 / secs_per_col).round())).unwrap_or(0);
        let key = PictureKey { revision: folder.revision, mute_mask: folder.mute_mask(), take, cols_per_sec, style, rule };
        let top = self.y;
        // The lane: stripe and divider.
        painter.fill(Fill::NonZero, Affine::IDENTITY, self.palette.row_a, None, &Rect::new(GUTTER, top, self.width, top + h));
        painter.fill(Fill::NonZero, Affine::IDENTITY, self.palette.divider, None, &Rect::new(0.0, top + h, self.width, top + h + 1.0));
        // Bar lines.
        let bar_len = 4.0 * 60.0 / BPM;
        for bar in (0..=BARS).map(|b| crate::num::coord(b) * bar_len) {
            if bar >= t0 && bar <= t1 {
                let x = (bar - t0) / secs_per_col + GUTTER;
                painter.fill(Fill::NonZero, Affine::IDENTITY, self.palette.grid, None, &Rect::new(x, top, x + 1.0, top + h));
            }
        }
        let track = self.palette.accent;
        // Recorded in *window* space (0..cols, 0..h), replayed at the
        // lane's origin: the folded picture is the cache's unit.
        let picture = self.cache.get_or_build(key, || {
            let mut scene = Scene::new();
            // A shift in the window is a shift in the fold's origin;
            // `fold` works from 0 so the offset is applied by folding
            // a longer span and skipping — cheap enough here.
            let skip = crate::num::index((t0 / secs_per_col).round());
            let all = fold(folder, take, cols.saturating_add(skip), secs_per_col, rule);
            let cols_in = all.get(skip..).unwrap_or(&[]);
            draw_item(&mut scene, cols_in, style, track, Box { x0: 0.0, top: 3.0, width: lane_w, height: h - 6.0 });
            scene
        });
        let at = Affine::translate((GUTTER, top));
        for cmd in &picture.commands {
            submit_command(painter, cmd, at);
        }
        self.label(painter, name, false, top + 16.0);
        self.label(painter, note, true, top + 32.0);
        self.y += h + 1.0;
    }

    /// The folder in comping: a comp lane on top, then one lane per take.
    fn comp(&mut self, painter: &mut impl PaintScene, folder: &Folder, comp: &Comp, style: Style, h: f64, window: (f64, f64)) {
        let (t0, t1) = window;
        let lane_w = self.width - GUTTER - PAD;
        let px = |t: f64| ((t - t0) / (t1 - t0)).mul_add(lane_w, GUTTER);
        // The take lanes are drawn first; the comp lane row is reserved
        // above them and drawn from regions of the takes' pictures.
        let comp_top = self.y;
        self.y += h + 1.0;
        let mut take_tops = Vec::new();
        for take in 0..folder.take_count {
            take_tops.push(self.y);
            let name = format!("Take {}", take.saturating_add(1));
            self.row(painter, folder, RowSpec { take, style, rule: SumRule::MinMax, h, name: &name, note: "one lane per take, the take's folder item", window });
        }
        // Dim the parts of each take lane the comp did not choose, and
        // outline the chosen regions.
        for (take, top) in take_tops.iter().enumerate() {
            let mut chosen: Vec<(f64, f64)> = comp.regions.iter().filter(|r| r.take == take).map(|r| (r.from, r.to)).collect();
            chosen.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut cursor = t0;
            let shade = self.palette.surface.multiply_alpha(0.55);
            for (from, to) in &chosen {
                if *from > cursor {
                    painter.fill(Fill::NonZero, Affine::IDENTITY, shade, None, &Rect::new(px(cursor), *top, px(*from), top + h));
                }
                cursor = cursor.max(*to);
                painter.stroke(&Stroke::new(1.5), Affine::IDENTITY, self.palette.text, None, &Rect::new(px(*from), top + 1.0, px(*to), top + h - 1.0));
            }
            if cursor < t1 {
                painter.fill(Fill::NonZero, Affine::IDENTITY, shade, None, &Rect::new(px(cursor), *top, px(t1), top + h));
            }
        }
        // The comp lane: each region is the chosen take's picture,
        // clipped to the region; crossfades at the boundaries.
        painter.fill(Fill::NonZero, Affine::IDENTITY, self.palette.row_b, None, &Rect::new(GUTTER, comp_top, self.width, comp_top + h));
        painter.fill(Fill::NonZero, Affine::IDENTITY, self.palette.divider, None, &Rect::new(0.0, comp_top + h, self.width, comp_top + h + 1.0));
        let cols = crate::num::index(lane_w.floor());
        let secs_per_col = (t1 - t0) / crate::num::coord(cols.max(1));
        let track = self.palette.accent;
        for r in &comp.regions {
            let from = r.from.max(t0);
            let to = r.to.min(t1);
            if to <= from {
                continue;
            }
            let c0 = crate::num::index(((from - t0) / secs_per_col).floor());
            let c1 = crate::num::index(((to - t0) / secs_per_col).ceil());
            let skip = crate::num::index((t0 / secs_per_col).round());
            let all = fold(folder, r.take, cols.saturating_add(skip), secs_per_col, SumRule::MinMax);
            let slice = all.get(c0.saturating_add(skip)..c1.saturating_add(skip).min(all.len())).unwrap_or(&[]);
            let mut scene = Scene::new();
            draw_item(&mut scene, slice, style, track, Box { x0: 0.0, top: 3.0, width: crate::num::coord(slice.len()), height: h - 6.0 });
            let at = Affine::translate((crate::num::coord(c0) + GUTTER, comp_top));
            for cmd in &scene.commands {
                submit_command(painter, cmd, at);
            }
            let tag = format!("T{}", r.take.saturating_add(1));
            crate::tcp::glyphs(painter, self.font, self.palette.text, &tag, px(from) + 4.0, comp_top + 14.0, 10.0);
        }
        // Crossfades: a darkened wedge either side of each boundary.
        let mut bounds: Vec<f64> = comp.regions.iter().map(|r| r.from).filter(|f| *f > t0 && *f < t1).collect();
        bounds.sort_by(f64::total_cmp);
        for b in bounds {
            let (xa, xb) = (px(b - comp.crossfade / 2.0), px(b + comp.crossfade / 2.0));
            let mut wedge = BezPath::new();
            wedge.move_to((xa, comp_top + 3.0));
            wedge.line_to((xb, comp_top + 3.0));
            wedge.line_to((xa, comp_top + h - 3.0));
            wedge.close_path();
            painter.fill(Fill::NonZero, Affine::IDENTITY, Color::from_rgba8(0, 0, 0, 0x60), None, &wedge);
            painter.stroke(&Stroke::new(1.0), Affine::IDENTITY, self.palette.text, None, &Line::new((xa, comp_top + h - 3.0), (xb, comp_top + 3.0)));
            painter.stroke(&Stroke::new(1.0), Affine::IDENTITY, self.palette.text_dim, None, &Line::new((xa, comp_top + 3.0), (xb, comp_top + h - 3.0)));
        }
        self.label(painter, "Drums  (comp)", false, comp_top + 16.0);
        self.label(painter, "the comp lane: regions chosen from the take lanes", true, comp_top + 32.0);
    }
}

/// The findings sheet: sum rules and colour styles, mute against hide,
/// and the comp view. Returns the cache's hit/miss count so the run
/// can say what was rebuilt.
pub fn shot(painter: &mut impl PaintScene, palette: &Palette, font: &Font, width: f64, height: f64, zoom: (f64, f64)) -> (usize, usize) {
    painter.fill(Fill::NonZero, Affine::IDENTITY, palette.surface, None, &Rect::new(0.0, 0.0, width, height));
    let mut folder = kit();
    let mut sheet = Sheet { palette, font, width, y: 0.0, cache: PictureCache::default() };
    let h = 96.0;

    sheet.heading(painter, "1. Sum rule and colouring — take 1, the same eight bars");
    sheet.row(painter, &folder, RowSpec { take: 0, style: Style::Grey, rule: SumRule::Mean, h, name: "mean (editor's lane rule)", note: "children averaged then normalised — bleed everywhere, hits flattened", window: zoom });
    for style in Style::ALL {
        sheet.row(painter, &folder, RowSpec { take: 0, style, rule: SumRule::MinMax, h, name: "min/max fold", note: style.label(), window: zoom });
    }

    sheet.heading(painter, "2. Mute vs hide — pieces style; the cache says which of these were rebuilt");
    sheet.row(painter, &folder, RowSpec { take: 0, style: Style::Pieces, rule: SumRule::MinMax, h, name: "all children", note: "same key as above: a cache HIT, replayed", window: zoom });
    folder.set_hidden("Tom 1", true);
    folder.set_hidden("Tom 2", true);
    folder.set_hidden("Floor Tom", true);
    sheet.row(painter, &folder, RowSpec { take: 0, style: Style::Pieces, rule: SumRule::MinMax, h, name: "toms HIDDEN", note: "identical picture — hiding is the TCP's business; a HIT, no refold", window: zoom });
    folder.set_muted("Snare Top", true);
    folder.set_muted("Snare Bottom", true);
    sheet.row(painter, &folder, RowSpec { take: 0, style: Style::Pieces, rule: SumRule::MinMax, h, name: "snare MUTED", note: "the snare drops out of the sum; revision bumped — a MISS, refolded", window: zoom });
    sheet.cache.invalidate(folder.revision);
    folder.set_muted("Snare Top", false);
    folder.set_muted("Snare Bottom", false);
    folder.set_hidden("Tom 1", false);
    folder.set_hidden("Tom 2", false);
    folder.set_hidden("Floor Tom", false);

    sheet.heading(painter, "3. Comping — one lane per take on the folder, the comp lane on top");
    let comp = Comp {
        regions: vec![
            Region { take: 0, from: 0.0, to: 4.0 },
            Region { take: 1, from: 4.0, to: 10.0 },
            Region { take: 2, from: 10.0, to: 13.0 },
            Region { take: 1, from: 13.0, to: folder.length_secs },
        ],
        crossfade: 0.12,
    };
    sheet.comp(painter, &folder, &comp, Style::Pieces, h, zoom);

    (sheet.cache.hits, sheet.cache.misses)
}
