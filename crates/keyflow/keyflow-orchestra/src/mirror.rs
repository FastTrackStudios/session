//! Mirror pass — source-track MIDI → performance-track MIDI.
//!
//! The source track holds score-time MIDI: notes at notated positions plus the
//! full CC picture (CC1/CC2 expression, CC58 keyswitches, CC64 re-bow pedal).
//! This pass produces the library-facing copy: per-note legato/attack timing
//! pulls, strict note-on ordering, and same-pitch de-overlap — with the
//! note-anchored CCs (58/64/5) re-anchored to the shifted note-ons.
//!
//! Deliberately MIDI-domain: everything is re-derived from what a MIDI item
//! can hold, so hand-played material mirrors exactly like MusicXML imports:
//! - **articulation** = CC58 keyswitch state at each note-on
//! - **legato edge** = different-pitch overlap with the previous note
//! - **re-bow** = same-pitch abutment under a held CC64 pedal
//!
//! Given a stage-1 engine output (`Config::timing_comp = false`), this pass
//! reproduces the full engine's timed output (see `tests/mirror_parity.rs`).

use std::collections::BTreeMap;

use crate::config::Config;
use crate::engine::{CcEvent, OutNote, total_delay_ms};
use crate::score::{TempoPoint, bpm_at};

const EPS: f64 = 1e-6;

/// A plain MIDI note (QN domain, 1-based channel) — the exchange type between
/// DAW readers/writers and the mirror pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiNote {
    pub start_qn: f64,
    pub end_qn: f64,
    pub chan: u8,
    pub pitch: i32,
    pub vel: u8,
}

impl From<&OutNote> for MidiNote {
    fn from(n: &OutNote) -> Self {
        Self {
            start_qn: n.start_qn,
            end_qn: n.end_qn,
            chan: n.chan,
            pitch: n.pitch,
            vel: n.vel,
        }
    }
}

/// Result of mirroring one part/item.
#[derive(Debug, Clone, Default)]
pub struct MirrorOutput {
    pub notes: Vec<MidiNote>,
    pub ccs: Vec<CcEvent>,
    /// Suggested item start (min shifted note-on minus the configured lead-in).
    pub item_start_qn: f64,
}

/// How a CC58 value classifies for timing purposes. Only "marcato" changes the
/// delay math; shorts never overlap so they never read as legato anyway.
fn ks_is_marcato(val: u8) -> bool {
    (66..=75).contains(&val) // marcato + marcato-with-overlay bands
}

fn ks_is_legato_toggle(val: u8) -> bool {
    (76..=85).contains(&val) // legato on / legato off
}

fn ks_is_con_sord(val: u8) -> bool {
    (86..=95).contains(&val) // con sordino on / off
}

/// Short-articulation bands (spiccato/staccatissimo/staccato/sfz/pizzicato)
/// plus tremolo — none of these connect, so a same-pitch abutment between
/// them is a plain break, not a re-bow. Marcato deliberately does NOT block:
/// the engine decides re-bow before its fast-run→marcato conversion, so a
/// marcato-keyswitched note abutting the same pitch is (almost always) a
/// fast-run tail flowing into a held note, which re-bows.
fn ks_blocks_rebow(val: u8) -> bool {
    (11..=35).contains(&val) || (56..=60).contains(&val)
}

/// Per-channel step timeline of a CC's value (state machine over events).
struct CcState {
    /// (qn, val) sorted by qn.
    events: Vec<(f64, u8)>,
}

impl CcState {
    fn new(ccs: &[CcEvent], chan: u8, cc: u8) -> Self {
        let mut events: Vec<(f64, u8)> = ccs
            .iter()
            .filter(|e| e.chan == chan && e.cc == cc)
            .map(|e| (e.qn, e.val))
            .collect();
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { events }
    }

    /// Last value at or before `qn` (None if no event yet).
    fn at(&self, qn: f64) -> Option<u8> {
        let mut cur = None;
        for &(q, v) in &self.events {
            if q <= qn + EPS {
                cur = Some(v);
            } else {
                break;
            }
        }
        cur
    }

    fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Working copy of a note through the mirror pass.
struct MNote {
    src: MidiNote,
    /// CC58 state at the source note-on (articulation identity), if any.
    ks_val: Option<u8>,
    /// Authoritative articulation from source notation events, if present.
    hint: Option<&'static str>,
    legato_from: bool,
    re_bow_to: bool,
    /// Shifted note-on.
    start: f64,
    /// Note-off (source end, then de-overlap may pull it in).
    stop: f64,
}

impl MNote {
    /// Does this note refuse to connect (block re-bow)? Keyswitch schemes are
    /// lossy — e.g. woodwinds have no tremolo band, so a tremolo note carries
    /// the sustain keyswitch — so a notation hint always wins over CC58 state.
    ///
    /// The hint is the NOTATED articulation: `"marcato"` means a written
    /// strong-accent (which breaks the line), whereas a fast-run note that the
    /// engine auto-converted to the marcato patch is notated plain — it stays
    /// connected. Without hints, a marcato keyswitch is assumed to be the
    /// (far more common) fast-run kind and does not block.
    fn blocks_rebow(&self) -> bool {
        match self.hint {
            Some(a) => matches!(
                a,
                "spiccato" | "staccatissimo" | "staccato" | "pizzicato" | "tremolo" | "marcato"
            ),
            None => self.ks_val.map(ks_blocks_rebow).unwrap_or(false),
        }
    }

    /// Marcato patch engaged (no sampled pre-delay → no timing pull)? True
    /// for both the notated and the fast-run kind — the keyswitch band sees
    /// both; a notated hint catches it even if the keyswitch stream is bare.
    fn is_marcato(&self) -> bool {
        self.ks_val.map(ks_is_marcato).unwrap_or(false) || self.hint == Some("marcato")
    }
}

/// Mirror one part's source MIDI into performance MIDI.
///
/// `cfg.profile` selects the delay tables (which library plays this mirror);
/// `tempos` must be the project tempo map so ms↔QN conversion lands right.
/// `artic_hints`, when given, is index-aligned with `src_notes` and carries
/// each note's notated articulation (from the source item's notation events);
/// it overrides CC58 inference where the keyswitch scheme is lossy.
pub fn mirror_part(
    src_notes: &[MidiNote],
    src_ccs: &[CcEvent],
    artic_hints: Option<&[&'static str]>,
    cfg: &Config,
    tempos: &[TempoPoint],
) -> MirrorOutput {
    let prof = cfg.profile.profile();
    if src_notes.is_empty() {
        return MirrorOutput::default();
    }
    let fallback_tempo = [TempoPoint {
        qn: 0.0,
        bpm: 120.0,
    }];
    let tempos: &[TempoPoint] = if tempos.is_empty() {
        &fallback_tempo
    } else {
        tempos
    };

    // Working notes, grouped per channel, sorted by source start.
    let mut notes: Vec<MNote> = src_notes
        .iter()
        .enumerate()
        .map(|(i, &src)| MNote {
            src,
            ks_val: None,
            hint: artic_hints.and_then(|h| h.get(i).copied()),
            legato_from: false,
            re_bow_to: false,
            start: src.start_qn,
            stop: src.end_qn,
        })
        .collect();
    let mut by_ch: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, n) in notes.iter().enumerate() {
        by_ch.entry(n.src.chan).or_default().push(i);
    }
    for list in by_ch.values_mut() {
        list.sort_by(|&a, &b| notes[a].src.start_qn.total_cmp(&notes[b].src.start_qn));
    }

    // -------------------------------------------------------------
    // 1. Infer articulation state + legato/re-bow edges per channel
    // -------------------------------------------------------------
    for (&ch, list) in &by_ch {
        let ks = CcState::new(src_ccs, ch, cfg.cc_keyswitch);
        for &ni in list {
            notes[ni].ks_val = ks.at(notes[ni].src.start_qn).filter(|v| {
                // legato-toggle / sordino presses are state, not articulation
                !ks_is_legato_toggle(*v) && !ks_is_con_sord(*v)
            });
        }
        let rebow_capable = cfg.re_bow && prof.legato.is_some() && !prof.polyphonic;
        for w in 0..list.len().saturating_sub(1) {
            let (ai, bi) = (list[w], list[w + 1]);
            let a = &notes[ai];
            let b = &notes[bi];
            let gap = b.src.start_qn - a.src.end_qn;
            if a.src.pitch != b.src.pitch {
                // different-pitch overlap = legato transition
                if gap < -EPS {
                    notes[bi].legato_from = true;
                }
            } else {
                // same-pitch never overlaps in a valid source; a tiny gap
                // between two SUSTAIN notes is a re-bow/re-tongue (the engine
                // always re-bows same-pitch sustains). Shorts and tremolo
                // don't connect, so those junctions are breaks. (The CC64
                // pedal isn't a reliable witness — consecutive runs' on/off
                // presses interleave in the source stream.)
                if rebow_capable
                    && !a.blocks_rebow()
                    && !b.blocks_rebow()
                    && gap.abs() <= cfg.break_gap_qn * 2.0 + EPS
                {
                    notes[ai].re_bow_to = true;
                    notes[bi].legato_from = true;
                }
            }
        }
    }

    // -------------------------------------------------------------
    // 2. Timing compensation (same math as the engine)
    // -------------------------------------------------------------
    if cfg.timing_comp {
        for n in notes.iter_mut() {
            let artic = if n.is_marcato() {
                "marcato"
            } else {
                "sustain" // only marcato-vs-not matters for the delay
            };
            let script_ms = total_delay_ms(artic, n.legato_from, n.src.vel as f64, cfg, prof)
                - cfg.track_delay_ms;
            if script_ms != 0.0 {
                let lead_qn = (script_ms * bpm_at(tempos, n.src.start_qn) / 60000.0)
                    .clamp(-cfg.max_lead_qn, cfg.max_lead_qn);
                n.start = n.src.start_qn - lead_qn;
            }
        }
        // keep each channel's note-ons strictly ordered after shifting
        for list in by_ch.values() {
            for w in 1..list.len() {
                if notes[list[w]].start <= notes[list[w - 1]].start {
                    notes[list[w]].start = notes[list[w - 1]].start + 1e-4;
                }
            }
        }
    }

    let min_start = notes.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
    let item_start_qn = (min_start - cfg.lead_in_qn).max(0.0);

    // -------------------------------------------------------------
    // 3. Same-pitch de-overlap (sampler-hang guarantee)
    // -------------------------------------------------------------
    for list in by_ch.values_mut() {
        list.sort_by(|&a, &b| notes[a].start.total_cmp(&notes[b].start));
        let mut last_by_pitch: BTreeMap<i32, usize> = BTreeMap::new();
        for &ni in list.iter() {
            if let Some(&pi) = last_by_pitch.get(&notes[ni].src.pitch) {
                if notes[pi].stop > notes[ni].start - cfg.break_gap_qn {
                    let mut s = notes[ni].start - cfg.break_gap_qn;
                    if s <= notes[pi].start {
                        s = (notes[pi].start + notes[ni].start) * 0.5;
                    }
                    notes[pi].stop = s;
                }
            }
            last_by_pitch.insert(notes[ni].src.pitch, ni);
        }
    }

    // -------------------------------------------------------------
    // 4. CC re-anchoring / regeneration
    // -------------------------------------------------------------
    let mut ccs: Vec<CcEvent> = Vec::new();
    let ks_lead_qn = 1.0 / 64.0;

    // CC58: regenerated per channel from the inferred per-note state so each
    // press precedes its (shifted) note. Legato-toggle presses re-anchor to
    // the new item start; sordino presses keep their musical position.
    for (&ch, list) in &by_ch {
        let ks = CcState::new(src_ccs, ch, cfg.cc_keyswitch);
        if ks.is_empty() {
            continue;
        }
        let mut tl: Vec<(f64, u8)> = Vec::new();
        // initial legato-toggle press (first one wins), re-anchored
        if let Some(&(_, v)) = ks.events.iter().find(|(_, v)| ks_is_legato_toggle(*v)) {
            tl.push((item_start_qn, v));
        }
        // sordino presses stay where the score put them
        for &(q, v) in ks.events.iter().filter(|(_, v)| ks_is_con_sord(*v)) {
            tl.push((q.max(item_start_qn), v));
        }
        // articulation presses: state change before each shifted note-on
        let mut order: Vec<usize> = list.clone();
        order.sort_by(|&a, &b| {
            notes[a]
                .start
                .total_cmp(&notes[b].start)
                .then_with(|| notes[b].src.pitch.cmp(&notes[a].src.pitch))
        });
        let mut cur: Option<u8> = None;
        for &ni in &order {
            let Some(want) = notes[ni].ks_val else {
                continue;
            };
            if cur != Some(want) {
                tl.push(((notes[ni].start - ks_lead_qn).max(item_start_qn), want));
                cur = Some(want);
            }
        }
        // strictly-increasing ticks; drop redundant repeats
        tl.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut prevq: Option<f64> = None;
        let mut prevv: Option<u8> = None;
        for (mut q, v) in tl {
            if let Some(pq) = prevq {
                if q <= pq {
                    q = pq + cfg.ks_stagger_qn;
                }
            }
            if prevv != Some(v) {
                ccs.push(CcEvent {
                    qn: q,
                    chan: ch,
                    cc: cfg.cc_keyswitch,
                    val: v,
                });
                prevv = Some(v);
            }
            prevq = Some(q);
        }
    }

    // CC64: regenerated across each re-bow run, anchored to shifted times.
    // CC64 is CSS's re-bow/re-tongue trigger — overlapping runs on one
    // channel merge into a single held span so the pedal never drops mid-run.
    let ped_qn = 1.0 / 128.0;
    for (&ch, list) in &by_ch {
        let mut spans: Vec<(f64, f64)> = Vec::new();
        let mut i = 0usize;
        while i < list.len() {
            if notes[list[i]].re_bow_to {
                let s = list[i];
                let mut j = i;
                while j < list.len() && notes[list[j]].re_bow_to {
                    j += 1;
                }
                let last = list.get(j).copied().unwrap_or(list[j - 1]);
                spans.push((
                    (notes[s].start - ped_qn).max(item_start_qn),
                    notes[last].stop + ped_qn,
                ));
                i = j + 1;
            } else {
                i += 1;
            }
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (on, off) in spans {
            match merged.last_mut() {
                Some(m) if on <= m.1 + EPS => m.1 = m.1.max(off),
                _ => merged.push((on, off)),
            }
        }
        for (on, off) in merged {
            ccs.push(CcEvent {
                qn: on,
                chan: ch,
                cc: cfg.cc_sustain_pedal,
                val: 127,
            });
            ccs.push(CcEvent {
                qn: off,
                chan: ch,
                cc: cfg.cc_sustain_pedal,
                val: 0,
            });
        }
    }

    // CC5 (portamento volume): anchored 1/128 before its note in the source;
    // move with that note. Unanchored events pass through.
    let porta_anchor = |qn: f64, chan: u8| -> Option<usize> {
        by_ch
            .get(&chan)?
            .iter()
            .copied()
            .find(|&ni| (notes[ni].src.start_qn - (qn + ped_qn)).abs() < 1e-4)
    };
    for e in src_ccs.iter().filter(|e| e.cc == cfg.cc_portamento) {
        let qn = match porta_anchor(e.qn, e.chan) {
            Some(ni) => (notes[ni].start - ped_qn).max(item_start_qn),
            None => e.qn,
        };
        ccs.push(CcEvent { qn, ..*e });
    }

    // Everything else (CC1/CC2 expression, CC11 volume, …) is musical-time
    // data: pass through untouched.
    for e in src_ccs {
        if e.cc == cfg.cc_keyswitch || e.cc == cfg.cc_sustain_pedal || e.cc == cfg.cc_portamento {
            continue;
        }
        ccs.push(*e);
    }

    ccs.sort_by(|a, b| {
        a.qn.total_cmp(&b.qn)
            .then_with(|| a.chan.cmp(&b.chan))
            .then_with(|| a.cc.cmp(&b.cc))
    });

    let out_notes = notes
        .iter()
        .map(|n| MidiNote {
            start_qn: n.start,
            end_qn: n.stop,
            chan: n.src.chan,
            pitch: n.src.pitch,
            vel: n.src.vel,
        })
        .collect();

    MirrorOutput {
        notes: out_notes,
        ccs,
        item_start_qn,
    }
}
