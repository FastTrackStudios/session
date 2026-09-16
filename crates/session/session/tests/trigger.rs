//! What a trigger render does with a hit list.

use session::hits::Hit;
use session::trigger::{Blend, Note, notes, velocity_of};

/// Velocity comes from how loud the piece actually was at the hit.
///
/// r[verify flow.drums.trigger.from-hits]
#[test]
fn a_louder_hit_triggers_harder() {
    let hits = [0.0, 1.0].map(|at| Hit { at }).to_vec();
    let loud_at_one = |at: f64| if at > 0.5 { 0.9 } else { 0.2 };
    let rendered = notes(&hits, &loud_at_one);
    assert!(
        rendered[1].velocity > rendered[0].velocity,
        "{rendered:?} — the louder hit came out quieter"
    );
}

/// **Silence is still a note.** A MIDI note-on at velocity zero is a
/// note-OFF, so a hit measured at silence would delete itself instead
/// of playing quietly.
#[test]
fn a_silent_hit_still_plays() {
    assert!(velocity_of(0.0) >= 1, "velocity zero is a note-off");
    assert!(
        velocity_of(f64::NAN) >= 1,
        "a bad measurement must not mute"
    );
}

/// Full scale is the top of the range and nothing above it wraps.
#[test]
fn the_range_is_bounded_at_both_ends() {
    assert_eq!(velocity_of(1.0), 127);
    assert_eq!(velocity_of(9.9), 127, "a hot measurement must not wrap");
    assert_eq!(velocity_of(-3.0), 1, "a negative one must not wrap either");
}

/// **The reason this is MIDI.** The notes are derived from the hit
/// list, so a hit slipped in the editor moves its sample with it — for
/// free, because there is nothing kept in step.
///
/// r[verify flow.drums.trigger.from-hits]
#[test]
fn moving_a_hit_moves_its_sample() {
    let steady = |_: f64| 0.7;
    let before = notes(&[Hit { at: 1.0 }], &steady);
    let after = notes(&[Hit { at: 1.25 }], &steady);
    assert!((before[0].at - 1.0).abs() < 1e-9);
    assert!((after[0].at - 1.25).abs() < 1e-9, "the sample stayed put");
}

/// A hit removed takes its sample with it, for the same reason.
///
/// r[verify flow.drums.trigger.from-hits]
#[test]
fn removing_a_hit_removes_its_sample() {
    let steady = |_: f64| 0.7;
    let hits = [0.0, 0.5, 1.0].map(|at| Hit { at }).to_vec();
    assert_eq!(notes(&hits, &steady).len(), 3);
    assert_eq!(notes(&hits[..2], &steady).len(), 2);
}

/// **Velocity is measured, never stored**, so a hit slipped onto a
/// quieter part of the take comes out quieter — which is what anyone
/// would expect and what a stored number would get wrong.
///
/// r[verify flow.drums.trigger.from-hits]
#[test]
fn a_hit_slipped_somewhere_quieter_triggers_softer() {
    let quiet_after_half = |at: f64| if at > 0.5 { 0.1 } else { 0.9 };
    let loud: Vec<Note> = notes(&[Hit { at: 0.25 }], &quiet_after_half);
    let slipped: Vec<Note> = notes(&[Hit { at: 0.75 }], &quiet_after_half);
    assert!(
        slipped[0].velocity < loud[0].velocity,
        "the slipped hit kept its old strength"
    );
}

/// The default blend puts the sample UNDER the mics: a trigger at the
/// same level as the close mic is a new drum, not a reinforcement.
#[test]
fn the_default_blend_is_a_reinforcement() {
    let blend = Blend::default();
    assert!(blend.level < 1.0, "the sample arrives as loud as the mic");
    assert!(!blend.inverted, "polarity is a choice, not a default");
}
