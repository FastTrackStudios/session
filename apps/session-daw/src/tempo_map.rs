//! Tempo mapping: moving a bar line to where the music actually is.
//!
//! The workflow REAPER's `tempo-map` overlay drives, brought here so it
//! can be done in the window — and, in time, in a browser against a
//! video. You play the song, and at each downbeat you put the nearest
//! bar line where you hear it. The tempo is whatever makes that true.
//!
//! This is the arithmetic, with nothing in it that knows about a
//! pointer or a DAW. Moving a bar line is a question about two numbers
//! — how long a stretch of music takes now and how long it should take
//! — and everything else is presentation.
//!
//! **The three levels of constraint** are the ones the REAPER overlay
//! binds to `g`, `Shift+g` and `Alt+g`, and the difference between them
//! is what STAYS PUT:
//!
//! - [`Anchor::Nothing`] changes the tempo back at the last tempo
//!   marker, so everything between it and the line you moved stretches
//!   with it. What you want on a first pass through a song with one
//!   tempo: there is nothing behind you to protect yet.
//! - [`Anchor::MeasureBefore`] puts a new tempo marker at the previous
//!   bar line, so only that one bar stretches. What you want once you
//!   have mapped the bars behind you and do not want them moving.
//! - [`Anchor::BothSides`] does that and also fixes the bar AFTER, by
//!   giving it a tempo that lands the following line where it already
//!   was. What you want when you are correcting one bar in the middle
//!   of a map that is otherwise right.

/// What a move is not allowed to disturb.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Anchor {
    /// Nothing behind is protected: the tempo changes at the last
    /// marker and everything after it moves.
    #[default]
    Nothing,
    /// The bar line before stays where it is.
    MeasureBefore,
    /// The bars before AND after both stay where they are.
    BothSides,
}

/// A tempo to write at a time, adding a marker there if none exists.
///
/// "The stretch starting here now runs at this tempo" — which is what a
/// tempo marker means, and the only thing a tempo edit ever says.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Set {
    pub at: f64,
    pub bpm: f64,
}

/// What a project needs to say for a bar line to be moved.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Move {
    /// The bar line being moved, where it is now.
    pub line: f64,
    /// Where it should be.
    pub to: f64,
    /// The bar line before it, and the tempo in force there.
    pub previous_line: f64,
    /// The bar line after it, if there is one — needed only to anchor
    /// the far side.
    pub next_line: Option<f64>,
    /// The last tempo marker at or before `previous_line`, and its
    /// tempo.
    pub marker: Set,
}

/// The smallest tempo a move may produce.
///
/// A line dragged onto or past the one before it would ask for an
/// infinite tempo, and a line dragged a long way right asks for one
/// near zero. Both are a slip of the hand rather than a request, and
/// refusing is kinder than writing a tempo map nobody can undo by eye.
const FLOOR: f64 = 1.0;

/// The largest.
const CEILING: f64 = 960.0;

/// What to write so that `line` lands on `to`.
///
/// `None` when the move is impossible or absurd — a line cannot be
/// dragged onto the one before it, and the tempo that would be needed
/// is refused rather than clamped, because a clamped tempo silently
/// puts the line somewhere other than where you asked.
#[must_use]
pub fn align(m: Move, anchor: Anchor) -> Option<Vec<Set>> {
    // The stretch being retimed, and what it should become.
    let (from, was, becomes) = match anchor {
        // Everything since the last marker stretches together.
        Anchor::Nothing => (m.marker.at, m.line - m.marker.at, m.to - m.marker.at),
        // Only the bar before the line stretches.
        Anchor::MeasureBefore | Anchor::BothSides => (
            m.previous_line,
            m.line - m.previous_line,
            m.to - m.previous_line,
        ),
    };
    if was <= 0.0 || becomes <= 0.0 {
        return None;
    }
    // Time and tempo are inverse: a stretch that has to take longer
    // must run slower, by exactly the ratio of the two durations.
    let bpm = m.marker.bpm * was / becomes;
    let bpm = sane(bpm)?;
    let mut sets = vec![Set { at: from, bpm }];

    if anchor == Anchor::BothSides {
        // The bar after has to end where it already ended, and it now
        // starts somewhere else — so it gets its own tempo.
        let next = m.next_line?;
        let (was, becomes) = (next - m.line, next - m.to);
        if was <= 0.0 || becomes <= 0.0 {
            return None;
        }
        sets.push(Set {
            at: m.to,
            bpm: sane(m.marker.bpm * was / becomes)?,
        });
    }
    Some(sets)
}

/// A tempo, or nothing if it is not one.
fn sane(bpm: f64) -> Option<f64> {
    (bpm.is_finite() && (FLOOR..=CEILING).contains(&bpm)).then_some(bpm)
}

/// The bar line nearest a time, from a walked timeline.
///
/// Nearest rather than previous: you click where the downbeat IS, and
/// the line you meant is whichever one is closest to that — before it
/// if you were early, after it if you were late.
#[must_use]
pub fn nearest_line(beats: &[crate::ruler::Beat], at: f64) -> Option<crate::ruler::Beat> {
    beats
        .iter()
        .filter(|beat| beat.is_downbeat())
        .min_by(|a, b| (a.at - at).abs().total_cmp(&(b.at - at).abs()))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::{Anchor, Move, Set, align};

    /// A bar at 120 that should have taken twice as long runs at 60.
    ///
    /// The whole idea in one line: time and tempo are inverse, so a
    /// stretch that has to take twice as long runs at half the tempo.
    #[test]
    fn a_bar_that_takes_twice_as_long_runs_half_as_fast() {
        let moved = align(
            Move {
                line: 2.0,
                to: 4.0,
                previous_line: 0.0,
                next_line: None,
                marker: Set {
                    at: 0.0,
                    bpm: 120.0,
                },
            },
            Anchor::Nothing,
        )
        .expect("a legal move");
        assert_eq!(moved.len(), 1);
        assert!(
            (moved[0].bpm - 60.0).abs() < 1e-9,
            "expected 60, got {}",
            moved[0].bpm
        );
        assert!((moved[0].at - 0.0).abs() < 1e-9, "written at the marker");
    }

    /// Anchoring the bar before writes the tempo THERE, not back at the
    /// marker — which is what stops the bars you already mapped from
    /// moving under you.
    #[test]
    fn anchoring_the_measure_before_writes_a_marker_there() {
        let m = Move {
            line: 10.0,
            to: 11.0,
            previous_line: 8.0,
            next_line: Some(12.0),
            marker: Set {
                at: 0.0,
                bpm: 120.0,
            },
        };
        let loose = align(m, Anchor::Nothing).expect("legal");
        let tight = align(m, Anchor::MeasureBefore).expect("legal");
        assert!(
            (loose[0].at - 0.0).abs() < 1e-9,
            "loose writes at the marker"
        );
        assert!(
            (tight[0].at - 8.0).abs() < 1e-9,
            "tight writes at the bar before"
        );
        // And they are different tempos, because they are stretching
        // different amounts of music.
        assert!((loose[0].bpm - tight[0].bpm).abs() > 1.0);
    }

    /// Anchoring both sides leaves the bar after where it was.
    ///
    /// The one that matters when correcting a single bar in the middle
    /// of a finished map: without the second tempo the rest of the song
    /// slides by however much you moved this line.
    #[test]
    fn anchoring_both_sides_puts_the_next_line_back() {
        let m = Move {
            line: 10.0,
            to: 11.0,
            previous_line: 8.0,
            next_line: Some(12.0),
            marker: Set {
                at: 0.0,
                bpm: 120.0,
            },
        };
        let sets = align(m, Anchor::BothSides).expect("legal");
        assert_eq!(sets.len(), 2, "both sides needs two tempos: {sets:?}");
        assert!(
            (sets[1].at - 11.0).abs() < 1e-9,
            "the second starts at the moved line"
        );
        // The property, checked rather than gestured at: play the
        // second tempo forward and the next line has to land back on
        // 12. Asserting the tempos are "about right" proves nothing;
        // this proves the line is where it was.
        let bar_after = 4.0 * 60.0 / sets[1].bpm;
        assert!(
            (sets[1].at + bar_after - 12.0).abs() < 1e-6,
            "the next line landed at {}, not 12",
            sets[1].at + bar_after
        );
        assert!(sets[1].bpm > sets[0].bpm, "a shorter bar runs faster");
    }

    /// A line cannot be dragged onto the one before it.
    ///
    /// The tempo that would be needed is infinite. Refused rather than
    /// clamped: a clamped tempo puts the line somewhere other than
    /// where you asked, and silently.
    #[test]
    fn a_line_cannot_land_on_the_one_before_it() {
        let m = Move {
            line: 2.0,
            to: 0.0,
            previous_line: 0.0,
            next_line: None,
            marker: Set {
                at: 0.0,
                bpm: 120.0,
            },
        };
        assert!(align(m, Anchor::Nothing).is_none());
        assert!(align(Move { to: -1.0, ..m }, Anchor::Nothing).is_none());
    }

    /// An absurd tempo is refused, both ways.
    #[test]
    fn an_absurd_tempo_is_refused() {
        // Dragged almost onto the previous line: far too fast.
        let fast = Move {
            line: 2.0,
            to: 0.000_1,
            previous_line: 0.0,
            next_line: None,
            marker: Set {
                at: 0.0,
                bpm: 120.0,
            },
        };
        assert!(align(fast, Anchor::Nothing).is_none());
        // Dragged a very long way right: far too slow.
        let slow = Move {
            to: 100_000.0,
            ..fast
        };
        assert!(align(slow, Anchor::Nothing).is_none());
    }

    /// Moving a line to where it already is changes nothing.
    ///
    /// The no-op has to be exact, because tempo mapping is a hundred
    /// small corrections and one that drifts on a null move would
    /// accumulate.
    #[test]
    fn a_move_to_where_it_already_is_keeps_the_tempo() {
        let sets = align(
            Move {
                line: 4.0,
                to: 4.0,
                previous_line: 2.0,
                next_line: None,
                marker: Set {
                    at: 0.0,
                    bpm: 137.5,
                },
            },
            Anchor::Nothing,
        )
        .expect("legal");
        assert!(
            (sets[0].bpm - 137.5).abs() < 1e-9,
            "a null move changed the tempo to {}",
            sets[0].bpm
        );
    }
}
