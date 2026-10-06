//! A kit's hits, read off its folder's peaks: where a quantize cuts.
//!
//! The folder already holds every mic's peaks (`load`), folded by piece
//! of the kit (`fold`); a hit is where a piece's envelope jumps. Peaks
//! are 160-sample blocks — under 4 ms — which is finer than a drummer
//! places anything and coarser than any edit needs, so the hits come
//! from what is already loaded rather than a second read of the audio.
//!
//! The pieces are read apart (kick, snare, toms; never the cymbals and
//! rooms, which ring through everything) and their hits merged: a kick
//! and a crash on one beat are one beat, and the cut goes before the
//! earlier of them.

use expression_editor_core::kit::LaneRole;

use super::Folder;
use super::fold::{Grid, GroupBy, SLOTS, fold};

/// The envelope's resolution, in seconds.
pub const COLUMN_SECS: f64 = 0.004;
/// The least gap between two hits on one piece: a buzz roll is not
/// twenty hits.
pub const REFRACTORY_SECS: f64 = 0.06;
/// Hits on different pieces this close are one beat.
pub const MERGE_SECS: f64 = 0.025;
/// A hit is at least this share of its piece's loudest.
const FLOOR: f32 = 0.12;
/// And this many times what the piece was doing just before.
const JUMP: f32 = 2.0;
/// "Just before": this many columns.
const BEFORE: usize = 6;

/// The hits in `take` of `folder` between `from` and `to`, in project
/// seconds, earliest first.
#[must_use]
pub fn hits(folder: &Folder, take: usize, from: f64, to: f64) -> Vec<f64> {
    let span = to - from;
    if span <= 0.0 {
        return Vec::new();
    }
    let columns = crate::num::index((span / COLUMN_SECS).ceil());
    let grid = Grid::over(from, to, columns);
    let by_piece = fold(&folder.children, take, grid, GroupBy::Role);
    let col_secs = grid.secs_per_column();
    let mut all: Vec<f64> = Vec::new();
    for (slot, role) in LaneRole::ALL.iter().enumerate().take(SLOTS) {
        if !role.detects() {
            continue;
        }
        let envelope: Vec<f32> = by_piece
            .columns
            .iter()
            .map(|c| {
                c.slots
                    .get(slot)
                    .copied()
                    .flatten()
                    .map_or(0.0, |(mn, mx)| mn.abs().max(mx.abs()))
            })
            .collect();
        all.extend(
            onsets(&envelope)
                .into_iter()
                .map(|i| crate::num::coord(i).mul_add(col_secs, from)),
        );
    }
    all.sort_by(f64::total_cmp);
    let mut merged: Vec<f64> = Vec::new();
    for t in all {
        if merged.last().is_none_or(|last| t - last > MERGE_SECS) {
            merged.push(t);
        }
    }
    merged
}

/// The columns where `envelope` jumps: over the floor, and `JUMP` times
/// the loudest of the columns just before, at least the refractory gap
/// after the last one.
fn onsets(envelope: &[f32]) -> Vec<usize> {
    let peak = envelope.iter().copied().fold(0.0_f32, f32::max);
    if peak <= 0.0 {
        return Vec::new();
    }
    let floor = peak * FLOOR;
    let gap = crate::num::index((REFRACTORY_SECS / COLUMN_SECS).ceil());
    let mut found: Vec<usize> = Vec::new();
    for (i, &a) in envelope.iter().enumerate() {
        if a < floor
            || found
                .last()
                .is_some_and(|&last| i.saturating_sub(last) < gap)
        {
            continue;
        }
        let before = envelope
            .get(i.saturating_sub(BEFORE)..i)
            .unwrap_or(&[])
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        if a > before * JUMP {
            found.push(i);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::super::fold::{Child, ChildTake, TakePeaks};
    use super::*;

    /// A mono take of `secs` at 48 kHz, 160-sample peaks, silent but for
    /// a decaying hit of `level` at each of `at`.
    fn take(secs: f64, at: &[f64], level: f64) -> TakePeaks {
        let block = 160.0 / 48_000.0;
        let blocks = crate::num::index((secs / block).ceil());
        let pairs: Vec<f64> = (0..blocks)
            .flat_map(|b| {
                let t = crate::num::coord(b) * block;
                let a = at
                    .iter()
                    .filter(|&&h| t >= h)
                    .map(|&h| level * (-(t - h) * 30.0).exp())
                    .fold(0.0_f64, f64::max);
                [-a, a]
            })
            .collect();
        TakePeaks::new(&pairs, 1, 160, 48_000.0)
    }

    fn mic(name: &str, role: LaneRole, peaks: TakePeaks) -> Child {
        Child {
            guid: format!("{{{name}}}"),
            name: name.to_owned(),
            role,
            side: None,
            muted: false,
            hidden: false,
            takes: vec![ChildTake::one(0.0, 2.0, peaks)],
        }
    }

    fn kit(children: Vec<Child>) -> Folder {
        Folder {
            guid: "{Drums}".to_owned(),
            name: "Drums".to_owned(),
            colour: vello::peniko::Color::WHITE,
            group_by: GroupBy::Role,
            children,
            start_secs: 0.0,
            length_secs: 2.0,
            take_count: 1,
        }
    }

    /// The kick's and the snare's hits, each where it was played, the
    /// kick and snare on one beat as one hit.
    #[test]
    fn a_kits_hits_are_where_its_pieces_were_played() {
        let folder = kit(vec![
            mic("In", LaneRole::Kick, take(2.0, &[0.1, 0.6, 1.1], 0.9)),
            mic("Top", LaneRole::Snare, take(2.0, &[0.6, 1.35], 0.7)),
        ]);
        let found = hits(&folder, 0, 0.0, 2.0);
        let near = |t: f64| found.iter().any(|h| (h - t).abs() < 0.01);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(
            near(0.1) && near(0.6) && near(1.1) && near(1.35),
            "{found:?}"
        );
    }

    /// Cymbals ring through everything and are never a cut.
    #[test]
    fn the_cymbals_are_not_hits() {
        let folder = kit(vec![mic("OH", LaneRole::Other, take(2.0, &[0.3], 0.9))]);
        assert!(hits(&folder, 0, 0.0, 2.0).is_empty());
    }
}
