//! The drum part as written, held up to the part as played: which note
//! each hit is.
//!
//! A drummer sits ahead of the click or behind it for a whole section —
//! a pre-chorus pushed 40 ms early, a breakdown laid back — and inside
//! that feel a hit can wander a long way: a kick written on 4, played
//! 100 ms early, is nearer the sixteenth before 4 than 4 itself. The
//! nearest line puts it there, a sixteenth early for good, and every bar
//! of the section the same. The written part knows it is on 4.
//!
//! So each piece's hits are matched to that piece's own notes ([`align`]):
//! first the feel — how far the notes sit from the hits that are plainly
//! theirs — then each hit, moved by the feel, to the nearest note not
//! already taken, one to one, within most of a sixteenth. A hit with no
//! note near (a variation, a fill the chart does not have) is matched to
//! nothing and placed some other way. A note with no hit is not played.

/// How near a note a hit must be to say what the feel is, sixteenths.
const PLAIN: f64 = 0.5;
/// How far from a hit, its feel taken out, its note may be, sixteenths:
/// most of one, so a hit is never a whole line from its note.
const REACH: f64 = 0.75;
/// Fewer plain matches than this and there is no feel to speak of.
const FEEL_FROM: usize = 4;

/// One piece's hits matched to its notes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Aligned {
    /// How far the notes sit after the hits, seconds — positive when the
    /// drummer plays ahead (early).
    pub feel: f64,
    /// Each hit's note, in the hits' order.
    pub notes: Vec<Option<f64>>,
}

/// `hits` (one piece's, by time) matched to `notes` (the same piece's
/// written notes, by time), a sixteenth being `step` seconds.
#[must_use]
pub fn align(hits: &[f64], notes: &[f64], step: f64) -> Aligned {
    let nearest = |t: f64| {
        let at = notes.partition_point(|n| *n < t);
        [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .filter_map(|i| notes.get(i).copied())
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
    };
    let mut plain: Vec<f64> = hits
        .iter()
        .filter_map(|h| nearest(*h).map(|n| n - h))
        .filter(|d| d.abs() <= PLAIN * step)
        .collect();
    plain.sort_by(f64::total_cmp);
    let feel = if plain.len() >= FEEL_FROM {
        plain.get(plain.len() / 2).copied().unwrap_or(0.0)
    } else {
        0.0
    };
    // Every pairing within reach, nearest first; each hit and each note
    // taken once.
    let reach = REACH * step;
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, h) in hits.iter().enumerate() {
        let c = h + feel;
        let from = notes.partition_point(|n| *n < c - reach);
        for (j, n) in notes.iter().enumerate().skip(from) {
            if *n > c + reach {
                break;
            }
            pairs.push(((n - c).abs(), i, j));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut of_hit: Vec<Option<usize>> = vec![None; hits.len()];
    let mut taken = vec![false; notes.len()];
    for (_, i, j) in pairs {
        if of_hit.get(i).copied().flatten().is_some() || taken.get(j).copied().unwrap_or(true) {
            continue;
        }
        // Never out of order: a hit after another is never given a note
        // before that one's.
        let crosses = of_hit
            .iter()
            .enumerate()
            .any(|(k, other)| other.is_some_and(|o| (k < i && o > j) || (k > i && o < j)));
        if crosses {
            continue;
        }
        if let Some(slot) = of_hit.get_mut(i) {
            *slot = Some(j);
        }
        if let Some(t) = taken.get_mut(j) {
            *t = true;
        }
    }
    Aligned {
        feel,
        notes: of_hit
            .into_iter()
            .map(|j| j.and_then(|j| notes.get(j).copied()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 160 bpm: a sixteenth is 93.75 ms.
    const STEP: f64 = 0.093_75;

    /// The pre-chorus of Unbreakable: the kick written on 1, the "and" of
    /// 2 and 4, played 40 ms early — and the one on 4 100 ms early, nearer
    /// the sixteenth before it. Its note is still 4.
    #[test]
    fn a_hit_pushed_past_the_line_before_is_still_its_notes() {
        let bar = 61.5;
        let beat = 0.375;
        let notes: Vec<f64> = (0..4)
            .flat_map(|b| {
                let at = (f64::from(b) * 4.0).mul_add(beat, bar);
                [at, 1.5f64.mul_add(beat, at), 3.0f64.mul_add(beat, at)]
            })
            .collect();
        let mut hits: Vec<f64> = notes.iter().map(|n| n - 0.040).collect();
        hits[2] = notes[2] - 0.100;
        let aligned = align(&hits, &notes, STEP);
        assert!((aligned.feel - 0.040).abs() < 1e-9, "{}", aligned.feel);
        for (k, n) in aligned.notes.iter().enumerate() {
            assert_eq!(*n, Some(notes[k]), "hit {k}");
        }
        let nearest_line = (hits[2] / STEP).round() * STEP;
        assert!(
            (nearest_line - (notes[2] - STEP)).abs() < 1e-6,
            "the nearest line is the sixteenth before"
        );
    }

    /// A hit the chart does not have is nobody's note — and never takes
    /// one from the hit that is.
    #[test]
    fn an_extra_hit_takes_no_note() {
        let notes = [1.0, 1.375, 1.75, 2.125];
        let hits = [0.99, 1.37, 1.47, 1.75, 2.12];
        let aligned = align(&hits, &notes, STEP);
        assert_eq!(
            aligned.notes,
            [Some(1.0), Some(1.375), None, Some(1.75), Some(2.125)]
        );
    }

    /// Too few plain matches is no feel; nothing written is nothing
    /// matched.
    #[test]
    fn no_notes_no_matches() {
        let aligned = align(&[1.0, 2.0], &[], STEP);
        assert_eq!(
            aligned,
            Aligned {
                feel: 0.0,
                notes: vec![None, None]
            }
        );
        let aligned = align(&[1.0, 2.0], &[1.01, 2.01], STEP);
        assert!(aligned.feel.abs() < 1e-12);
        assert_eq!(aligned.notes, [Some(1.01), Some(2.01)]);
    }
}
