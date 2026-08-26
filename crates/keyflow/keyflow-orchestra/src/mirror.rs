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
use crate::engine::{total_delay_ms, CcEvent, OutNote};
use crate::score::{bpm_at, TempoPoint};

/// The annotation model itself lives in `keyflow-annotate` (shared with
/// signal-sampler's document mode); re-exported for consumers of this pass.
pub use keyflow_annotate::{
    annotate_line, ks_blocks_rebow, ks_is_con_sord, ks_is_legato_toggle, ks_is_marcato, CcTimeline,
    EdgeParams, LineNote, NoteAnnotation, EPS,
};

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

/// The channel's CC timeline for one controller (state machine over events).
fn cc_timeline(ccs: &[CcEvent], chan: u8, cc: u8) -> CcTimeline {
    CcTimeline::from_events(
        ccs.iter()
            .filter(|e| e.chan == chan && e.cc == cc)
            .map(|e| (e.qn, e.val)),
    )
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
    /// Marcato patch engaged (no sampled pre-delay → no timing pull)? True
    /// for both the notated and the fast-run kind — the keyswitch band sees
    /// both; a notated hint catches it even if the keyswitch stream is bare.
    fn is_marcato(&self) -> bool {
        self.ks_val.map(ks_is_marcato).unwrap_or(false) || self.hint == Some("marcato")
    }
}

/// Does a note refuse to connect (block re-bow)? Keyswitch schemes are
/// lossy — e.g. woodwinds have no tremolo band, so a tremolo note carries
/// the sustain keyswitch — so a notation hint always wins over CC58 state.
///
/// The hint is the NOTATED articulation: `"marcato"` means a written
/// strong-accent (which breaks the line), whereas a fast-run note that the
/// engine auto-converted to the marcato patch is notated plain — it stays
/// connected. Without hints, a marcato keyswitch is assumed to be the
/// (far more common) fast-run kind and does not block.
fn note_blocks_rebow(hint: Option<&'static str>, ks_val: Option<u8>) -> bool {
    match hint {
        Some(a) => matches!(
            a,
            "spiccato" | "staccatissimo" | "staccato" | "pizzicato" | "tremolo" | "marcato"
        ),
        None => ks_val.map(ks_blocks_rebow).unwrap_or(false),
    }
}

/// Stage 1 of the mirror pass on its own: infer each note's articulation
/// state (CC58 at note-on) and the legato/re-bow edges, per channel — a thin
/// grouping adapter over [`keyflow_annotate::annotate_line`] (the ONE shared
/// implementation, also consumed by signal-sampler's document mode; the
/// adapter-level parity is asserted in signal-sampler's
/// `tests/annotation_parity.rs`).
pub fn stage1_annotations(
    src_notes: &[MidiNote],
    src_ccs: &[CcEvent],
    artic_hints: Option<&[&'static str]>,
    cfg: &Config,
) -> Vec<NoteAnnotation> {
    let prof = cfg.profile.profile();
    let params = EdgeParams {
        break_gap_qn: cfg.break_gap_qn,
        rebow_capable: cfg.re_bow && prof.legato.is_some() && !prof.polyphonic,
        // Notation-domain source: same-pitch notes never overlap (the engine
        // guarantees it), so only the abutment window connects.
        connect_same_pitch_overlap: false,
    };
    let mut ann = vec![NoteAnnotation::default(); src_notes.len()];
    let mut by_ch: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, n) in src_notes.iter().enumerate() {
        by_ch.entry(n.chan).or_default().push(i);
    }
    for list in by_ch.values_mut() {
        list.sort_by(|&a, &b| src_notes[a].start_qn.total_cmp(&src_notes[b].start_qn));
    }
    let hint = |i: usize| artic_hints.and_then(|h| h.get(i).copied());
    for (&ch, list) in &by_ch {
        let ks = cc_timeline(src_ccs, ch, cfg.cc_keyswitch);
        let line: Vec<LineNote> = list
            .iter()
            .map(|&ni| LineNote {
                start_qn: src_notes[ni].start_qn,
                end_qn: src_notes[ni].end_qn,
                pitch: src_notes[ni].pitch,
            })
            .collect();
        // The notated articulation wins over lossy CC58 state (see
        // `note_blocks_rebow`); `w` indexes the line, `list[w]` the part.
        let line_ann = annotate_line(
            &line,
            &ks,
            |w, ks_val| note_blocks_rebow(hint(list[w]), ks_val),
            &params,
        );
        for (w, &ni) in list.iter().enumerate() {
            ann[ni] = line_ann[w];
        }
    }
    ann
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

    // -------------------------------------------------------------
    // 1. Infer articulation state + legato/re-bow edges per channel
    // -------------------------------------------------------------
    let ann = stage1_annotations(src_notes, src_ccs, artic_hints, cfg);

    // Working notes, grouped per channel, sorted by source start.
    let mut notes: Vec<MNote> = src_notes
        .iter()
        .enumerate()
        .map(|(i, &src)| MNote {
            src,
            ks_val: ann[i].ks_val,
            hint: artic_hints.and_then(|h| h.get(i).copied()),
            legato_from: ann[i].legato_from,
            re_bow_to: ann[i].re_bow_to,
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
        let ks = cc_timeline(src_ccs, ch, cfg.cc_keyswitch);
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
