//! Shared articulation / legato / re-bow annotation model.
//!
//! The ONE source of truth for the stage-1 inference that used to live as a
//! hand-maintained parity pair: `keyflow-orchestra/src/mirror.rs` (the
//! reference — parity-tested against the real CSS Orchestrator engine in
//! keyflow's `tests/mirror_parity.rs`) and signal-sampler's `document.rs`
//! (document-mode lookahead playback). Both now consume this crate; their
//! adapter-level equivalence is locked by signal-sampler's
//! `tests/annotation_parity.rs` over the whole MusicXML corpus.
//!
//! Deliberately MIDI-domain: everything is re-derived from what a MIDI item
//! can hold, so hand-played material annotates exactly like MusicXML imports:
//! - **articulation** = CC58 keyswitch state at each note-on
//! - **legato edge** = different-pitch overlap with the previous note on the
//!   same mono line
//! - **re-bow** = same-pitch abutment between connectable (sustain-family)
//!   notes
//!
//! `no_std` + `alloc`, dependency-free, no file I/O — consumable from
//! processing-core crates (native / WASM / embedded) and std tooling alike.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// QN-domain comparison tolerance shared by both consumers.
pub const EPS: f64 = 1e-6;

// ── CC58 band classification ─────────────────────────────────────────────────
// How a CC58 value classifies for timing purposes. Only "marcato" changes the
// delay math; shorts never overlap so they never read as legato anyway.

/// Marcato + marcato-with-overlay bands (no sampled pre-delay → no timing
/// pull).
pub fn ks_is_marcato(val: u8) -> bool {
    (66..=75).contains(&val)
}

/// Legato on / legato off presses — state, not articulation.
pub fn ks_is_legato_toggle(val: u8) -> bool {
    (76..=85).contains(&val)
}

/// Con sordino on / off presses — state, not articulation.
pub fn ks_is_con_sord(val: u8) -> bool {
    (86..=95).contains(&val)
}

/// Short-articulation bands (spiccato/staccatissimo/staccato/sfz/pizzicato)
/// plus tremolo — none of these connect, so a same-pitch abutment between
/// them is a plain break, not a re-bow. Marcato deliberately does NOT block:
/// the engine decides re-bow before its fast-run→marcato conversion, so a
/// marcato-keyswitched note abutting the same pitch is (almost always) a
/// fast-run tail flowing into a held note, which re-bows.
pub fn ks_blocks_rebow(val: u8) -> bool {
    (11..=35).contains(&val) || (56..=60).contains(&val)
}

// ── CC step timeline ─────────────────────────────────────────────────────────

/// Per-line step timeline of a CC's value (state machine over events).
pub struct CcTimeline {
    /// `(qn, val)` sorted by qn.
    pub events: Vec<(f64, u8)>,
}

impl CcTimeline {
    /// Build from any event iterator (the caller filters by channel + CC
    /// number); events are sorted here.
    pub fn from_events(events: impl IntoIterator<Item = (f64, u8)>) -> Self {
        let mut events: Vec<(f64, u8)> = events.into_iter().collect();
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { events }
    }

    /// Last value at or before `qn` (None if no event yet).
    pub fn at(&self, qn: f64) -> Option<u8> {
        self.last_matching(qn, |_| true)
    }

    /// Last value at/before `qn` satisfying `pred`, skipping non-matching
    /// events (e.g. the last CC58 that selected a legato *mode*, ignoring the
    /// articulation / toggle keyswitches pressed since).
    pub fn last_matching(&self, qn: f64, pred: impl Fn(u8) -> bool) -> Option<u8> {
        let mut cur = None;
        for &(q, v) in &self.events {
            if q <= qn + EPS {
                if pred(v) {
                    cur = Some(v);
                }
            } else {
                break;
            }
        }
        cur
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// ── Stage-1 annotation ───────────────────────────────────────────────────────

/// One note on a mono line, as the edge inference sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineNote {
    pub start_qn: f64,
    pub end_qn: f64,
    pub pitch: i32,
}

/// Stage-1 annotation of one note: CC58 articulation state at the note-on
/// plus the inferred legato/re-bow edges. Index-aligned with the notes given
/// to [`annotate_line`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteAnnotation {
    /// CC58 state at the note-on (articulation identity) — filtered of
    /// legato-toggle / sordino presses, which are state, not articulation.
    pub ks_val: Option<u8>,
    /// This note is reached by a legato transition (different-pitch overlap
    /// or same-pitch re-bow).
    pub legato_from: bool,
    /// This note flows into ANY legato transition (different-pitch move or
    /// same-pitch re-bow).
    pub legato_to: bool,
    /// This note flows into a same-pitch re-bow (subset of `legato_to`).
    pub re_bow_to: bool,
}

/// Tunables for [`annotate_line`].
#[derive(Debug, Clone, Copy)]
pub struct EdgeParams {
    /// Same-pitch abutment tolerance (QN): gaps up to `break_gap_qn * 2`
    /// connect. Both consumers default to 1/64 (i.e. gaps up to ~1/32 QN).
    pub break_gap_qn: f64,
    /// Whether this line can re-bow at all — keyflow gates on
    /// `cfg.re_bow && profile.legato.is_some() && !profile.polyphonic`,
    /// signal-sampler on `spec.legato_engine.is_some()`.
    pub rebow_capable: bool,
    /// Notation-domain sources (keyflow's mirror pass) guarantee same-pitch
    /// notes never overlap, so only the ABUTMENT window connects
    /// (`gap.abs() <= break_gap_qn * 2`). Performance-domain documents
    /// (signal-sampler) set this: same-pitch overlaps deeper than the window
    /// still connect — on a mono line an overlapping repeat can only be a
    /// re-bow; treating it as a break would leave the line sounding at the
    /// next note-on and push it down the reactive path. The two behaviours
    /// are IDENTICAL on valid notation-domain input (gap ≥ 0).
    pub connect_same_pitch_overlap: bool,
}

impl Default for EdgeParams {
    fn default() -> Self {
        Self {
            break_gap_qn: 1.0 / 64.0,
            rebow_capable: true,
            connect_same_pitch_overlap: false,
        }
    }
}

/// Stage-1 inference over ONE mono line (the caller groups notes per channel
/// or per engine line and sorts them by `start_qn`):
///
/// - **articulation** = CC58 keyswitch state at each note-on (`ks` is the
///   line's CC58 timeline), filtered of legato-toggle / sordino presses
/// - **legato edge** = different-pitch overlap with the previous note
/// - **re-bow** = same-pitch abutment between connectable notes — same-pitch
///   never overlaps in a valid source; a tiny gap between two SUSTAIN notes
///   is a re-bow/re-tongue (the engine always re-bows same-pitch sustains).
///   Shorts and tremolo don't connect, so those junctions are breaks. (The
///   CC64 pedal isn't a reliable witness — consecutive runs' on/off presses
///   interleave in the source stream.)
///
/// `blocks_rebow(i, ks_val)` decides whether note `i` refuses to connect —
/// callers with notation hints (keyflow) let the NOTATED articulation win
/// over lossy keyswitch state; bare-MIDI callers pass
/// [`default_blocks_rebow`].
pub fn annotate_line(
    notes: &[LineNote],
    ks: &CcTimeline,
    mut blocks_rebow: impl FnMut(usize, Option<u8>) -> bool,
    p: &EdgeParams,
) -> Vec<NoteAnnotation> {
    let mut ann = vec![NoteAnnotation::default(); notes.len()];
    for (i, n) in notes.iter().enumerate() {
        ann[i].ks_val = ks
            .at(n.start_qn)
            .filter(|v| !ks_is_legato_toggle(*v) && !ks_is_con_sord(*v));
    }
    for w in 0..notes.len().saturating_sub(1) {
        let (a, b) = (&notes[w], &notes[w + 1]);
        let gap = b.start_qn - a.end_qn;
        if a.pitch != b.pitch {
            // different-pitch overlap = legato transition
            if gap < -EPS {
                ann[w + 1].legato_from = true;
                ann[w].legato_to = true;
            }
        } else if p.rebow_capable
            && !blocks_rebow(w, ann[w].ks_val)
            && !blocks_rebow(w + 1, ann[w + 1].ks_val)
        {
            let window = p.break_gap_qn * 2.0 + EPS;
            let connect = if p.connect_same_pitch_overlap {
                gap <= window
            } else {
                gap.abs() <= window
            };
            if connect {
                ann[w].re_bow_to = true;
                ann[w].legato_to = true;
                ann[w + 1].legato_from = true;
            }
        }
    }
    ann
}

/// The bare-MIDI re-bow blocker: keyswitch state alone (no notation hints).
pub fn default_blocks_rebow(_i: usize, ks_val: Option<u8>) -> bool {
    ks_val.map(ks_blocks_rebow).unwrap_or(false)
}
