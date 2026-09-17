//! Counts and the dot: `4t`, then `.` over and over.
//!
//! Typing a number before a key means "that many times", and `.` means
//! "again". Both borrowed from vi, and borrowed on purpose: tempo
//! mapping is one gesture repeated a few hundred times down a song, and
//! the difference between pressing `t` four times and typing `4t` is
//! the difference between counting in your head and saying what you
//! want.
//!
//! What makes the dot worth having is that it repeats the COUNT too. A
//! tune that lands a downbeat every fourth transient is `4t` once and
//! then `.` for the rest of the song, and you never type the four
//! again.
//!
//! This is the machine, with nothing in it that knows what a command
//! does. It answers one question — what should happen when this key is
//! pressed — and the caller does it.

/// What a keypress amounts to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Press {
    /// A digit was taken as part of a count. Nothing to do yet.
    Counting,
    /// Run this command, this many times.
    Run { command: char, times: u32 },
    /// The key means nothing here; the caller should handle it.
    Ignored,
}

/// The most a count may ask for.
///
/// A typo like `9999t` should not walk nine thousand transients and
/// leave you scrolling back. High enough for any real phrase, low
/// enough to be an obvious mistake rather than a hang.
const MOST: u32 = 64;

/// A count being typed, and the last command run.
#[derive(Clone, Copy, Default, Debug)]
pub struct Repeat {
    count: Option<u32>,
    last: Option<(char, u32)>,
}

impl Repeat {
    /// What to do about a key.
    ///
    /// `commands` is what counts as a command here; anything else is
    /// [`Press::Ignored`] and the count is dropped — a count typed
    /// before something that is not a command was not a count, it was
    /// a number typed at the wrong window, and carrying it forward
    /// would make the NEXT command do something surprising.
    pub fn press(&mut self, key: char, commands: &[char]) -> Press {
        if let Some(digit) = key.to_digit(10) {
            // A leading zero is not a count. In vi it is a motion, and
            // here it is nothing — but it must not silently become a
            // count of zero, which would mean "do this no times".
            if self.count.is_none() && digit == 0 {
                return Press::Ignored;
            }
            let so_far = self.count.unwrap_or(0);
            self.count = Some(so_far.saturating_mul(10).saturating_add(digit).min(MOST));
            return Press::Counting;
        }
        if key == '.' {
            // The dot repeats the command AND its count, unless a new
            // count was typed in front of it — `2.` means twice, which
            // is how vi reads it and the only reading that lets you
            // correct a count without retyping the command.
            let Some((command, times)) = self.last else {
                self.count = None;
                return Press::Ignored;
            };
            let times = self.count.take().unwrap_or(times);
            self.last = Some((command, times));
            return Press::Run { command, times };
        }
        if commands.contains(&key) {
            let times = self.count.take().unwrap_or(1);
            self.last = Some((key, times));
            return Press::Run {
                command: key,
                times,
            };
        }
        // Not a command: whatever was being counted was not a count.
        self.count = None;
        Press::Ignored
    }

    /// Forget a half-typed count, on escape or on losing focus.
    ///
    /// A count left standing across a click would attach itself to
    /// whatever was pressed next, minutes later.
    pub fn clear(&mut self) {
        self.count = None;
    }

    /// The count being typed, for showing it.
    ///
    /// Worth showing: a count you cannot see is a count you have to
    /// remember typing, and the whole point was to stop counting in
    /// your head.
    #[must_use]
    pub const fn pending(self) -> Option<u32> {
        self.count
    }

    /// What the dot would repeat.
    #[must_use]
    pub const fn last(self) -> Option<(char, u32)> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::{Press, Repeat};

    const COMMANDS: [char; 2] = ['t', 'g'];

    fn run(keys: &str) -> (Repeat, Vec<Press>) {
        let mut repeat = Repeat::default();
        let out = keys
            .chars()
            .map(|key| repeat.press(key, &COMMANDS))
            .collect();
        (repeat, out)
    }

    /// A bare command runs once.
    #[test]
    fn a_command_on_its_own_runs_once() {
        let (_, out) = run("t");
        assert_eq!(
            out,
            vec![Press::Run {
                command: 't',
                times: 1
            }]
        );
    }

    /// `4t` runs it four times, and the digits do nothing on their own.
    #[test]
    fn a_count_multiplies_the_command() {
        let (_, out) = run("4t");
        assert_eq!(
            out,
            vec![
                Press::Counting,
                Press::Run {
                    command: 't',
                    times: 4
                }
            ]
        );
    }

    /// Two digits are one number, not two counts.
    #[test]
    fn a_count_can_have_two_digits() {
        let (_, out) = run("12t");
        assert_eq!(
            out[2],
            Press::Run {
                command: 't',
                times: 12
            }
        );
    }

    /// The dot repeats the command AND its count.
    ///
    /// The whole reason it is worth having: a tune that lands a
    /// downbeat every fourth transient is `4t` once and `.` for the
    /// rest of the song.
    #[test]
    fn the_dot_repeats_the_count_too() {
        let (_, out) = run("4t..");
        assert_eq!(
            out[1],
            Press::Run {
                command: 't',
                times: 4
            }
        );
        assert_eq!(
            out[2],
            Press::Run {
                command: 't',
                times: 4
            }
        );
        assert_eq!(
            out[3],
            Press::Run {
                command: 't',
                times: 4
            }
        );
    }

    /// A count in front of the dot replaces the remembered one.
    ///
    /// How you correct a count without retyping the command — and it
    /// STICKS, so the next bare dot uses the new one.
    #[test]
    fn a_count_before_the_dot_replaces_it() {
        let (_, out) = run("4t2..");
        assert_eq!(
            out[1],
            Press::Run {
                command: 't',
                times: 4
            }
        );
        assert_eq!(
            out[3],
            Press::Run {
                command: 't',
                times: 2
            }
        );
        assert_eq!(
            out[4],
            Press::Run {
                command: 't',
                times: 2
            },
            "the corrected count should stick"
        );
    }

    /// A dot with nothing to repeat does nothing.
    #[test]
    fn a_dot_with_no_history_does_nothing() {
        let (_, out) = run(".");
        assert_eq!(out, vec![Press::Ignored]);
    }

    /// A count typed before something that is not a command is dropped.
    ///
    /// The alternative is worse than doing nothing: a four left over
    /// from a mistyped key would silently make the NEXT command happen
    /// four times, long after you had forgotten typing it.
    #[test]
    fn a_count_before_a_non_command_is_forgotten() {
        let (mut repeat, out) = run("4x");
        assert_eq!(out[1], Press::Ignored);
        assert_eq!(repeat.pending(), None, "the count survived a stray key");
        assert_eq!(
            repeat.press('t', &COMMANDS),
            Press::Run {
                command: 't',
                times: 1
            },
            "the stray four came back"
        );
    }

    /// A leading zero is not a count of zero.
    ///
    /// Left alone it would mean "do this no times", which is a keypress
    /// that does nothing for a reason nobody could guess.
    #[test]
    fn a_leading_zero_is_not_a_count() {
        let (repeat, out) = run("0");
        assert_eq!(out, vec![Press::Ignored]);
        assert_eq!(repeat.pending(), None);
        // But a zero INSIDE a count is a digit like any other.
        let (_, out) = run("10t");
        assert_eq!(
            out[2],
            Press::Run {
                command: 't',
                times: 10
            }
        );
    }

    /// An absurd count is capped rather than obeyed.
    #[test]
    fn a_runaway_count_is_capped() {
        let (_, out) = run("9999t");
        let Press::Run { times, .. } = out[4] else {
            panic!("expected a run, got {:?}", out[4]);
        };
        assert!(times <= 64, "a typo asked for {times}");
    }

    /// Escape drops a half-typed count.
    #[test]
    fn a_half_typed_count_can_be_dropped() {
        let mut repeat = Repeat::default();
        assert_eq!(repeat.press('4', &COMMANDS), Press::Counting);
        repeat.clear();
        assert_eq!(
            repeat.press('t', &COMMANDS),
            Press::Run {
                command: 't',
                times: 1
            }
        );
    }
}
