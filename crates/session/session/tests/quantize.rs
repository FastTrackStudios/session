//! What quantizing and aligning a hit list do, and what they refuse to.

use session::hits::Hit;
use session::quantize::{Grid, Strength, align_to, applied, fills, plan};

/// A groove on eighths at 120 bpm, played a little late.
fn groove() -> Vec<Hit> {
    [0.02, 0.26, 0.51, 0.77, 1.02].map(|at| Hit { at }).to_vec()
}

fn eighths() -> Grid {
    Grid {
        interval: 0.25,
        origin: 0.0,
    }
}

/// Full strength puts every hit exactly on its line.
///
/// r[verify flow.drums.editing.quantize]
#[test]
fn full_strength_lands_on_the_grid() {
    let moves = plan(&groove(), eighths(), Strength::FULL, false);
    let after = applied(&groove(), &moves);
    for hit in &after {
        let line = eighths().nearest(hit.at);
        assert!((hit.at - line).abs() < 1e-9, "{hit:?} is off the grid");
    }
}

/// **Strength is a fraction, not a flag.** A kit pulled all the way to
/// the grid stops breathing, so the useful settings move a hit part of
/// the way — and half strength must land halfway, not somewhere.
///
/// r[verify flow.drums.editing.quantize]
#[test]
fn half_strength_moves_a_hit_halfway() {
    let hits = vec![Hit { at: 0.10 }];
    let moves = plan(&hits, eighths(), Strength::new(0.5), false);
    // The nearest line is 0.0, so half of a 0.10 pull is 0.05.
    assert!((moves[0].to - 0.05).abs() < 1e-9, "{:?}", moves[0]);
}

/// Zero strength moves nothing, which is what makes a preview safe to
/// leave open.
#[test]
fn zero_strength_moves_nothing() {
    let moves = plan(&groove(), eighths(), Strength::NONE, false);
    assert!(moves.iter().all(|m| m.distance() < 1e-9), "{moves:?}");
}

/// **The rule fills exist for.** A fill's internal timing IS the
/// performance; snapping each of its hits to the grid straightens the
/// thing that made it a fill. So the fill travels as one piece and its
/// gaps come out unchanged.
///
/// r[verify flow.drums.editing.quantize]
#[test]
fn a_protected_fill_keeps_its_internal_timing() {
    // A groove, then a burst of sixteenths played late.
    let mut hits = vec![Hit { at: 0.0 }, Hit { at: 0.5 }];
    for i in 0..6 {
        hits.push(Hit {
            at: 1.03 + f64::from(i) * 0.06,
        });
    }
    let before: Vec<f64> = hits[2..].windows(2).map(|w| w[1].at - w[0].at).collect();

    let moves = plan(&hits, eighths(), Strength::FULL, true);
    let after = applied(&hits, &moves);
    let gaps: Vec<f64> = after[2..].windows(2).map(|w| w[1].at - w[0].at).collect();

    for (a, b) in before.iter().zip(&gaps) {
        assert!(
            (a - b).abs() < 1e-9,
            "the fill was straightened: {before:?} became {gaps:?}"
        );
    }
    assert!(
        moves[2..].iter().all(|m| m.in_fill),
        "the burst was not recognised as a fill"
    );
}

/// Without protection the same fill IS straightened — the negative
/// control, so the test above is proving the flag and not an accident
/// of the numbers.
#[test]
fn without_protection_a_fill_is_straightened() {
    let mut hits = vec![Hit { at: 0.0 }, Hit { at: 0.5 }];
    for i in 0..6 {
        hits.push(Hit {
            at: 1.03 + f64::from(i) * 0.06,
        });
    }
    let moves = plan(&hits, eighths(), Strength::FULL, false);
    let after = applied(&hits, &moves);
    let gaps: Vec<f64> = after[2..].windows(2).map(|w| w[1].at - w[0].at).collect();
    assert!(
        gaps.iter().any(|g| g.abs() < 1e-9),
        "unprotected, the fill should collapse onto lines: {gaps:?}"
    );
}

/// A fill is measured against the take's OWN median gap, so the same
/// setting works on a ballad and on a fast tune.
#[test]
fn a_fill_is_relative_to_the_takes_own_tempo() {
    let slow: Vec<Hit> = [0.0, 1.0, 2.0, 2.1, 2.2, 2.3].map(|at| Hit { at }).to_vec();
    let fast: Vec<Hit> = [0.0, 0.1, 0.2, 0.21, 0.22, 0.23]
        .map(|at| Hit { at })
        .to_vec();
    assert!(fills(&slow, 0.6, 3)[3], "the slow tune's burst is a fill");
    assert!(fills(&fast, 0.6, 3)[3], "the fast tune's burst is too");
}

/// Alignment moves a hit to the reference hit nearest it.
///
/// r[verify flow.drums.editing.align-hits]
#[test]
fn alignment_pulls_a_take_onto_its_reference() {
    let take = vec![Hit { at: 0.04 }, Hit { at: 0.53 }];
    let reference = vec![Hit { at: 0.0 }, Hit { at: 0.5 }];
    let after = applied(&take, &align_to(&take, &reference, 0.1));
    assert!((after[0].at - 0.0).abs() < 1e-9);
    assert!((after[1].at - 0.5).abs() < 1e-9);
}

/// **And refuses to, past the window.** Beyond it the two takes are
/// playing different things, and dragging a hit across a beat onto the
/// nearest reference is worse than leaving it where it was played.
///
/// r[verify flow.drums.editing.align-hits]
#[test]
fn alignment_leaves_a_hit_alone_when_the_reference_is_far() {
    let take = vec![Hit { at: 0.40 }];
    let reference = vec![Hit { at: 0.0 }, Hit { at: 1.0 }];
    let moves = align_to(&take, &reference, 0.1);
    assert!(
        moves[0].distance() < 1e-9,
        "a hit was dragged across a beat: {:?}",
        moves[0]
    );
}
