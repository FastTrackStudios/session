//! The processing pipeline — port of `css_engine.lua` §6–7 (`processPart`).
//!
//! Stage order is load-bearing (see `reference-data/css-orchestrator/README.md`):
//! gliss expansion → chord-artic propagation → tie merge → channel assignment →
//! solo routing → legato connection → phrasing → fast-run marcato → velocity →
//! fade marking → timing compensation → same-pitch de-overlap → emission.

mod shaping;

pub use shaping::Phrase;
use shaping::{EPS, arch_at, build_phrasing, dyn_interp, fade_shape, micro_for, vib_bloom_at};

use std::collections::BTreeMap;

use crate::config::{Config, LegatoMode};
use crate::profile::{Profile, VibratoMode};
use crate::score::{ArtSet, Marking, MarkingKind, Part, RawNote, TempoPoint, bpm_at, chord_pcs};

/// Semantic articulation keywords (string parity with the Lua engine).
pub type Artic = &'static str;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeMode {
    In,
    Out,
    Swell,
}

/// Working note — accretes fields as it moves through the pipeline
/// (the Lua engine mutates one table; we keep one struct with defaults).
#[derive(Debug, Clone)]
pub struct WNote {
    // From parsing
    pub onset: f64,
    pub dur: f64,
    pub pitch: i32,
    pub voice: u32,
    pub tie_start: bool,
    pub tie_stop: bool,
    pub art: ArtSet,
    pub slur_start: bool,
    pub slur_stop: bool,
    pub beat_qn: f64,
    pub beats: u32,
    pub beat_type: u32,
    pub fifths: i32,
    // assignChannels
    pub chan: u8,
    pub rank: u32,
    // connectLegato
    pub artic: Artic,
    pub start: f64,
    pub stop: f64,
    pub slurred: bool,
    pub slur_continues: bool,
    pub legato_to: bool,
    pub legato_from: bool,
    pub re_bow_to: bool,
    pub re_bow_from: bool,
    // phrasing
    pub contour: f64,
    pub leap: f64,
    pub metric: f64,
    pub phrase: Option<usize>,
    pub phrase_end: bool,
    // velocity / fades / timing
    pub vel: f64,
    pub fast_marcato: bool,
    pub porta: bool,
    pub fade_mode: Option<FadeMode>,
    pub fade_peak: f64,
    pub solo: bool,
    pub lead_ms: f64,
}

impl WNote {
    fn from_raw(r: RawNote) -> Self {
        Self {
            onset: r.onset,
            dur: r.dur,
            pitch: r.pitch,
            voice: r.voice,
            tie_start: r.tie_start,
            tie_stop: r.tie_stop,
            art: r.art,
            slur_start: r.slur_start,
            slur_stop: r.slur_stop,
            beat_qn: r.beat_qn,
            beats: r.beats,
            beat_type: r.beat_type,
            fifths: r.fifths,
            chan: 1,
            rank: 1,
            artic: "sustain",
            start: r.onset,
            stop: 0.0,
            slurred: false,
            slur_continues: false,
            legato_to: false,
            legato_from: false,
            re_bow_to: false,
            re_bow_from: false,
            contour: 0.0,
            leap: 0.0,
            metric: 0.5,
            phrase: None,
            phrase_end: false,
            vel: 64.0,
            fast_marcato: false,
            porta: false,
            fade_mode: None,
            fade_peak: 0.0,
            solo: false,
            lead_ms: 0.0,
        }
    }
}

/// A processed output note, ready for a MIDI writer. All times in QN.
#[derive(Debug, Clone)]
pub struct OutNote {
    pub start_qn: f64,
    pub end_qn: f64,
    /// 1-based channel; writers subtract 1.
    pub chan: u8,
    pub pitch: i32,
    pub vel: u8,
    /// Semantic articulation ("sustain", "staccato", "spiccato", …).
    pub artic: Artic,
    /// Extra per-note lead the writer applied (ms, informational).
    pub lead_ms: f64,
    /// Original (uncompensated) onset QN.
    pub onset_qn: f64,
    /// Comma-joined score articulation tags (annotations).
    pub score_art: String,
    pub slur_start: bool,
    pub slur_stop: bool,
    pub legato_from: bool,
    pub slurred: bool,
    pub solo: bool,
}

/// A CC event. All times in QN, channel 1-based.
#[derive(Debug, Clone, Copy)]
pub struct CcEvent {
    pub qn: f64,
    pub chan: u8,
    pub cc: u8,
    pub val: u8,
}

/// Result of processing one part.
#[derive(Debug, Clone, Default)]
pub struct PartOutput {
    pub notes: Vec<OutNote>,
    pub ccs: Vec<CcEvent>,
    pub channels: Vec<u8>,
    pub markings: Vec<Marking>,
    pub start_qn: f64,
    pub end_qn: f64,
    pub item_start_qn: f64,
    pub item_end_qn: f64,
    pub empty: bool,
}

// ---------------------------------------------------------------------
// Articulation / keyswitch helpers
// ---------------------------------------------------------------------

/// Decide the semantic articulation for a note from its score articulations.
fn articulation_of(n: &WNote) -> Artic {
    let a = &n.art;
    if a.has("staccatissimo") {
        "staccatissimo"
    } else if a.has("spiccato") {
        "spiccato"
    } else if a.has("staccato") || a.has("detached-legato") {
        "staccato"
    } else if a.has("tremolo") {
        "tremolo"
    } else if a.has("pizzicato") || a.has("snap-pizzicato") {
        "pizzicato"
    } else if a.has("strong-accent") {
        "marcato"
    } else {
        "sustain"
    }
}

fn is_short(artic: Artic) -> bool {
    matches!(
        artic,
        "spiccato" | "staccatissimo" | "staccato" | "pizzicato"
    )
}

/// The sustain/legato keyswitch must match `legato_mode` (it has to agree with
/// the timing compensation, which uses the same mode).
fn sustain_ks(cfg: &Config) -> u8 {
    match cfg.legato_mode {
        LegatoMode::Expressive => cfg.ks.sustain_exp,
        LegatoMode::LowLatency => cfg.ks.sustain_ll,
    }
}

/// Map a semantic articulation to a CC58 value for the given profile, falling
/// back to a band that exists in ALL libraries when the ideal one does not.
fn ks_value(artic: Artic, cfg: &Config, prof: &Profile) -> u8 {
    let k = &cfg.ks;
    let sus = sustain_ks(cfg);
    match artic {
        "staccato" => k.staccato,
        "staccatissimo" => k.staccatissimo,
        "spiccato" => {
            if prof.spiccato {
                k.spiccato
            } else {
                k.staccatissimo
            }
        }
        "pizzicato" => {
            if prof.pizz {
                k.pizzicato
            } else {
                sus
            }
        }
        "tremolo" => {
            if prof.tremolo {
                k.tremolo
            } else {
                sus
            }
        }
        "marcato" => k.marcato_ov,
        _ => sus,
    }
}

// ---------------------------------------------------------------------
// Pre-processing passes
// ---------------------------------------------------------------------

/// A chord's notes are bowed/tongued together, so an articulation engraved on
/// ANY note of the chord applies to ALL of them (same voice + onset).
const CHORD_ARTIC_KEYS: &[&str] = &[
    "staccato",
    "staccatissimo",
    "spiccato",
    "detached-legato",
    "tremolo",
    "pizzicato",
    "strong-accent",
    "accent",
    "tenuto",
];

fn propagate_chord_artics(notes: &mut [WNote]) {
    let mut groups: BTreeMap<(u32, i64), Vec<usize>> = BTreeMap::new();
    for (i, n) in notes.iter().enumerate() {
        let key = (n.voice, (n.onset * 100000.0).round() as i64);
        groups.entry(key).or_default().push(i);
    }
    for group in groups.values() {
        if group.len() < 2 {
            continue;
        }
        let mut union: Vec<&str> = Vec::new();
        for &i in group {
            for k in CHORD_ARTIC_KEYS {
                if notes[i].art.has(k) && !union.contains(k) {
                    union.push(k);
                }
            }
        }
        for &i in group {
            for k in &union {
                notes[i].art.insert(*k);
            }
        }
    }
}

/// Merge tie-start → tie-stop chains (same voice + pitch) into single notes.
fn merge_ties(mut notes: Vec<WNote>) -> Vec<WNote> {
    notes.sort_by(|a, b| {
        a.onset
            .total_cmp(&b.onset)
            .then_with(|| a.pitch.cmp(&b.pitch))
    });
    let mut out: Vec<WNote> = Vec::with_capacity(notes.len());
    let mut open: Vec<bool> = Vec::new();
    for nt in notes {
        if nt.tie_stop {
            // find the open note we tie back into
            let mut merged = false;
            for i in (0..out.len()).rev() {
                let o = &out[i];
                if open[i]
                    && o.pitch == nt.pitch
                    && o.voice == nt.voice
                    && ((o.onset + o.dur) - nt.onset).abs() < 1e-3
                {
                    let art = nt.art.clone();
                    let o = &mut out[i];
                    o.dur += nt.dur;
                    open[i] = nt.tie_start;
                    // inherit articulations that appear on the tail (e.g. fermata)
                    o.art.union_with(&art);
                    merged = true;
                    break;
                }
            }
            if !merged {
                open.push(nt.tie_start);
                out.push(nt);
            }
        } else {
            open.push(nt.tie_start);
            out.push(nt);
        }
    }
    out
}

/// Assign each note a MIDI channel so every channel is monophonic. Voices get
/// disjoint channel blocks; within a voice, simultaneous notes (chords/divisi)
/// are ranked high→low into successive channels.
fn assign_channels(notes: &mut [WNote], cfg: &Config) -> Vec<u8> {
    let mut voice_list: Vec<u32> = Vec::new();
    for n in notes.iter() {
        if !voice_list.contains(&n.voice) {
            voice_list.push(n.voice);
        }
    }
    voice_list.sort_unstable();

    // Rank by how many same-voice notes are actually SOUNDING at the onset
    // with a higher pitch (a held top note must not lose its channel to a
    // re-articulated lower note). Rank 1 = top → first channel of the block.
    let mut width: BTreeMap<u32, u32> = voice_list.iter().map(|&v| (v, 1)).collect();
    let ranks: Vec<u32> = (0..notes.len())
        .map(|i| {
            let n = &notes[i];
            let mut rank = 1;
            for (j, m) in notes.iter().enumerate() {
                if j != i
                    && m.voice == n.voice
                    && m.onset <= n.onset + EPS
                    && (m.onset + m.dur) > n.onset + EPS
                    && (m.pitch > n.pitch || (m.pitch == n.pitch && m.onset < n.onset - EPS))
                {
                    rank += 1;
                }
            }
            rank
        })
        .collect();
    for (i, n) in notes.iter().enumerate() {
        let w = width.get_mut(&n.voice).unwrap();
        *w = (*w).max(ranks[i]);
    }

    // base channel per voice
    let mut base: BTreeMap<u32, u32> = BTreeMap::new();
    let mut ch = 1u32;
    for &v in &voice_list {
        base.insert(v, ch);
        ch += width[&v];
    }

    let mut used: Vec<u8> = Vec::new();
    for (i, n) in notes.iter_mut().enumerate() {
        let c = (base[&n.voice] + ranks[i] - 1).min(cfg.max_channels as u32) as u8;
        n.chan = c; // 1-based; writer subtracts 1
        n.rank = ranks[i];
        if !used.contains(&c) {
            used.push(c);
        }
    }
    used.sort_unstable();
    used
}

/// Diatonic pitch-class set (major scale) for a key signature's `fifths`.
fn diatonic_pcs(fifths: i32) -> [bool; 12] {
    let tonic = ((fifths * 7).rem_euclid(12)) as usize;
    let mut pcs = [false; 12];
    for iv in [0usize, 2, 4, 5, 7, 9, 11] {
        pcs[(tonic + iv) % 12] = true;
    }
    pcs
}

/// Expand harp glissandos into the actual note sweep: every scale note from
/// the gliss-start pitch up/down to the stop pitch, spread across the span.
/// Intermediate written anchors pace the sweep; the landing note is kept.
fn expand_glissandos(
    notes: Vec<WNote>,
    cfg: &Config,
    harmonies: &[crate::score::HarmonyPoint],
) -> Vec<WNote> {
    let chord_for = |start_qn: f64, stop_qn: f64| {
        let mut h = None;
        for hh in harmonies {
            if hh.qn <= stop_qn + 1e-6 && hh.qn >= start_qn - 2.0 {
                h = Some(hh);
            }
        }
        h
    };

    let mut out: Vec<WNote> = Vec::with_capacity(notes.len());
    let mut consumed = vec![false; notes.len()];
    for i in 0..notes.len() {
        if consumed[i] {
            continue;
        }
        if !notes[i].art.has("glissStart") {
            out.push(notes[i].clone());
            continue;
        }
        // collect the anchor chain: start, pacing notes, stop.
        let mut anchors: Vec<usize> = vec![i];
        for (j, m) in notes.iter().enumerate().skip(i + 1) {
            if m.voice == notes[i].voice {
                anchors.push(j);
                if m.art.has("glissStop") {
                    break;
                }
            }
        }
        let landing_idx = *anchors.last().unwrap();
        if anchors.len() >= 2 && notes[landing_idx].art.has("glissStop") {
            for &k in &anchors[1..] {
                consumed[k] = true;
            }
            let mut landing = notes[landing_idx].clone();
            landing.art.0.remove("glissando");
            landing.art.0.remove("glissStop");
            let n = &notes[i];
            let pcs = chord_for(n.onset, landing.onset)
                .map(chord_pcs)
                .unwrap_or_else(|| diatonic_pcs(n.fifths));
            let ring_to = landing.onset;
            for s in 0..anchors.len() - 1 {
                let a = &notes[anchors[s]];
                let b = &notes[anchors[s + 1]];
                let up = b.pitch > a.pitch;
                let step = if up { 1 } else { -1 };
                let mut seg: Vec<i32> = Vec::new();
                let mut p = a.pitch;
                while (up && p < b.pitch) || (!up && p > b.pitch) {
                    if pcs[(p.rem_euclid(12)) as usize] {
                        seg.push(p);
                    }
                    p += step;
                }
                if seg.is_empty() {
                    seg.push(a.pitch);
                }
                let span = (b.onset - a.onset).max(cfg.gliss_min_span_qn);
                let count = seg.len() as f64;
                for (k, sp) in seg.into_iter().enumerate() {
                    let onset = a.onset + span * k as f64 / count;
                    let mut swept = WNote::from_raw(RawNote {
                        onset,
                        dur: (ring_to - onset).max(span / count),
                        pitch: sp,
                        voice: n.voice,
                        tie_start: false,
                        tie_stop: false,
                        art: ArtSet::default(),
                        slur_start: false,
                        slur_stop: false,
                        beat_qn: n.beat_qn,
                        beats: n.beats,
                        beat_type: n.beat_type,
                        fifths: n.fifths,
                    });
                    swept.art.insert("glissSwept");
                    out.push(swept);
                }
            }
            out.push(landing); // landing note kept
        } else {
            out.push(notes[i].clone());
        }
    }
    out
}

/// Is this note on a metrically strong beat (downbeat or mid-bar)?
fn is_strong_beat(n: &WNote) -> bool {
    let bt = if n.beat_type == 0 { 4 } else { n.beat_type };
    let beat_len = 4.0 / bt as f64;
    let idx = n.beat_qn / beat_len;
    let nearest = (idx + 0.5).floor();
    if (idx - nearest).abs() > 0.05 {
        return false; // offbeat
    }
    let beats = if n.beats == 0 { 4 } else { n.beats };
    let b = (nearest as i64).rem_euclid(beats as i64) as u32;
    b == 0 || (beats % 2 == 0 && b == beats / 2)
}

/// Legato connection + per-note end times, per channel — port of `connectLegato`.
fn connect_legato(
    notes: &mut [WNote],
    by_ch: &BTreeMap<u8, Vec<usize>>,
    cfg: &Config,
    prof: &Profile,
) {
    for list in by_ch.values() {
        let mut list = list.clone();
        list.sort_by(|&a, &b| notes[a].onset.total_cmp(&notes[b].onset));
        for &i in &list {
            let n = &mut notes[i];
            n.artic = articulation_of(n);
            n.start = n.onset;
            n.stop = n.onset + n.dur;
            // shorten shorts
            match n.artic {
                "spiccato" | "staccatissimo" => n.stop = n.onset + n.dur * cfg.spiccato_len_frac,
                "staccato" => n.stop = n.onset + n.dur * cfg.staccato_len_frac,
                _ => {}
            }
        }

        // polyphonic instruments (harp): notes ring at written length, no mono legato
        if prof.polyphonic {
            continue;
        }

        // slur state: `slurred` = inside a slur span; `slur_continues` = the
        // slur is still open AFTER the note (it connects to the next note).
        if cfg.use_slurs {
            let mut depth = 0i32;
            for &i in &list {
                let n = &mut notes[i];
                if n.slur_start {
                    depth += 1;
                }
                n.slurred = depth > 0;
                if n.slur_stop {
                    depth = (depth - 1).max(0);
                }
                n.slur_continues = depth > 0;
            }
        }

        let mut group_start_qn = list.first().map(|&i| notes[i].onset).unwrap_or(0.0);
        for w in 0..list.len().saturating_sub(1) {
            let (ai, bi) = (list[w], list[w + 1]);
            let gap = notes[bi].onset - (notes[ai].onset + notes[ai].dur);
            let a_short = is_short(notes[ai].artic);
            let same_p = notes[ai].pitch == notes[bi].pitch;
            let real_rest = gap > cfg.rest_threshold_qn + EPS;
            let both_sustain = notes[ai].artic == "sustain" && notes[bi].artic == "sustain";

            // decide connection: hard breaks first, then re-bow, then slur/gap legato
            #[derive(PartialEq)]
            enum Conn {
                Break,
                Rebow,
                Legato,
            }
            let mut connect = if real_rest
                || a_short
                || notes[ai].art.has("staccato")
                || notes[ai].art.has("staccatissimo")
            {
                Conn::Break
            } else if same_p {
                if cfg.re_bow && both_sustain {
                    Conn::Rebow
                } else {
                    Conn::Break
                }
            } else if cfg.use_slurs && notes[ai].slurred {
                // slur-driven: connect inside a slur, breathe at the slur's end
                if notes[ai].slur_continues && both_sustain {
                    Conn::Legato
                } else {
                    Conn::Break
                }
            } else if both_sustain {
                Conn::Legato // gap-based default (unslurred)
            } else {
                Conn::Break
            };

            // auto-phrasing: break a long UNSLURRED legato run at a natural
            // point. Break points are rhythm/meter-based only — NOT pitch —
            // so same-rhythm harmony voices break together and stay locked.
            if connect == Conn::Legato && cfg.auto_slur && !(cfg.use_slurs && notes[ai].slurred) {
                let run_len = notes[bi].onset - group_start_qn;
                if run_len >= cfg.auto_slur_max_qn {
                    connect = Conn::Break; // bow/breath can't last forever
                } else if run_len >= cfg.auto_slur_target_qn
                    && (is_strong_beat(&notes[bi]) || notes[ai].dur >= cfg.auto_slur_long_note_qn)
                {
                    connect = Conn::Break; // natural bow-change / breath
                }
            }

            match connect {
                Conn::Legato => {
                    notes[ai].stop = notes[bi].onset + cfg.legato_overlap_qn;
                    notes[ai].legato_to = true;
                    notes[bi].legato_from = true;
                }
                Conn::Rebow => {
                    // repeated note: must NOT overlap (same pitch would hang the
                    // sampler). CC64 pedal sustains across the gap instead.
                    notes[ai].stop = notes[bi].onset;
                    notes[ai].re_bow_to = true;
                    notes[bi].re_bow_from = true;
                    notes[bi].legato_from = true;
                }
                Conn::Break => {
                    let b_onset = notes[bi].onset;
                    let a = &mut notes[ai];
                    if a.stop > b_onset - cfg.break_gap_qn {
                        a.stop = b_onset - cfg.break_gap_qn;
                    }
                    if a.stop <= a.onset {
                        a.stop = a.onset + cfg.break_gap_qn.max(a.dur * 0.5);
                    }
                }
            }

            if connect != Conn::Legato {
                group_start_qn = notes[bi].onset; // next note starts a new run
            }
        }
    }
}

/// TOTAL sampled-attack delay (ms) for a note by role. Only legato transitions
/// get a non-zero script offset; fresh attacks / re-bows / shorts return the
/// track baseline so their script offset is zero (they stay on the grid).
fn total_delay_ms_for(n: &WNote, cfg: &Config, prof: &Profile) -> f64 {
    total_delay_ms(n.artic, n.legato_from, n.vel, cfg, prof)
}

/// Shared with the mirror pass — the delay depends only on (artic,
/// legato-following, velocity zone), so a MIDI-domain caller can use it too.
pub(crate) fn total_delay_ms(
    artic: Artic,
    legato_from: bool,
    vel: f64,
    cfg: &Config,
    prof: &Profile,
) -> f64 {
    if artic == "marcato" {
        return cfg.track_delay_ms; // no sampled pre-delay → stays on grid
    }
    if legato_from {
        if let Some(l) = &prof.legato {
            if l.modes && cfg.legato_mode == LegatoMode::Expressive {
                return if vel <= 64.0 {
                    l.expr_slow
                } else if vel <= 100.0 {
                    l.expr_medium
                } else {
                    l.expr_fast
                };
            } else if l.modes {
                // Low Latency: two zones at 64
                return if vel <= 64.0 { l.ll_medium } else { l.ll_fast };
            } else {
                // brass: two zones, no mode toggle
                return if vel <= 64.0 {
                    l.expr_medium
                } else {
                    l.expr_fast
                };
            }
        }
    }
    cfg.track_delay_ms + cfg.attack_delay_ms
}

// ---------------------------------------------------------------------
// processPart
// ---------------------------------------------------------------------

/// Run the full pipeline on one parsed part.
pub fn process_part(part: &Part, cfg: &Config) -> PartOutput {
    let prof = cfg.profile.profile();
    let mut tempos: Vec<TempoPoint> = part.tempos.clone();
    // All parts MUST share one tempo map (velocity/timing depend on real-time
    // spacing) — the caller passes the unioned map via cfg.tempo_map.
    if let Some(map) = &cfg.tempo_map {
        if !map.is_empty() {
            tempos = map.clone();
        }
    }
    if tempos.is_empty() {
        tempos.push(TempoPoint {
            qn: 0.0,
            bpm: 120.0,
        });
    }

    let mut raw: Vec<WNote> = part.notes.iter().cloned().map(WNote::from_raw).collect();
    // harp glissandos → scale-note sweeps (before any other processing)
    if cfg.expand_gliss && prof.gliss_sweep {
        raw = expand_glissandos(raw, cfg, &part.harmonies);
    }
    if !cfg.grace {
        raw.retain(|n| !n.art.has("grace"));
    }
    if raw.is_empty() {
        return PartOutput {
            empty: true,
            markings: part.markings.clone(),
            ..Default::default()
        };
    }

    propagate_chord_artics(&mut raw); // chord notes share articulations
    let mut notes = merge_ties(raw);
    let mut channels = assign_channels(&mut notes, cfg);

    // solo / section routing: build solo spans from the score words, then move
    // notes inside a span to the solo channel block (keeping divisi rank).
    if cfg.solo_routing && prof.solo_separate {
        let mut spans: Vec<(f64, f64)> = Vec::new();
        let mut solo_start: Option<f64> = None;
        for mk in &part.markings {
            if let MarkingKind::Words(text) = &mk.kind {
                let w = text.to_lowercase();
                if solo_start.is_none() && (w.contains("solo") || w.contains("soli")) {
                    solo_start = Some(mk.qn);
                } else if solo_start.is_some()
                    && (w.contains("section")
                        || w.contains("tutti")
                        || w.contains("unis")
                        || w.contains("a 2")
                        || w.contains("a2")
                        || w.contains("gli altri")
                        || w.contains("joins")
                        || w.contains("all desks")
                        || w.contains("full"))
                {
                    spans.push((solo_start.take().unwrap(), mk.qn));
                }
            }
        }
        if let Some(s) = solo_start {
            spans.push((s, f64::INFINITY));
        }
        if !spans.is_empty() {
            let mut used_solo = false;
            for n in notes.iter_mut() {
                for &(s, e) in &spans {
                    if n.onset >= s - EPS && n.onset < e {
                        n.solo = true;
                        n.chan = cfg.solo_channel_base
                            + (n.rank.saturating_sub(1) as u8).min(cfg.solo_max_voices - 1);
                        used_solo = true;
                        break;
                    }
                }
            }
            if used_solo {
                let mut set: Vec<u8> = notes.iter().map(|n| n.chan).collect();
                set.sort_unstable();
                set.dedup();
                channels = set;
            }
        }
    }

    // group by channel (indices into `notes`)
    let mut by_ch: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, n) in notes.iter().enumerate() {
        by_ch.entry(n.chan).or_default().push(i);
    }

    connect_legato(&mut notes, &by_ch, cfg, prof);

    // part time bounds
    let mut start_qn = f64::INFINITY;
    let mut end_qn = f64::NEG_INFINITY;
    for n in &notes {
        start_qn = start_qn.min(n.start);
        end_qn = end_qn.max(n.stop);
    }
    let mut item_start_qn = (start_qn - cfg.lead_in_qn).max(0.0);

    // dynamics anchors (sorted, clamped/scaled)
    let mut dyns = part.dynamics.clone();
    dyns.sort_by(|a, b| a.qn.total_cmp(&b.qn));
    let mut anchors: Vec<(f64, f64)> = dyns
        .iter()
        .map(|d| {
            let v = (d.value * cfg.dyn_scale + cfg.dyn_offset).clamp(cfg.dyn_min, 127.0);
            (d.qn, v)
        })
        .collect();
    if anchors.is_empty() {
        anchors.push((start_qn, cfg.dyn_default));
    }

    // sort channel lists by onset (used by phrasing / marcato / velocity)
    for list in by_ch.values_mut() {
        list.sort_by(|&a, &b| notes[a].onset.total_cmp(&notes[b].onset));
    }

    // build phrase models up front (annotates notes); reused for CC shaping
    let mut phrases: Vec<Phrase> = Vec::new();
    if cfg.phrasing {
        for list in by_ch.values() {
            build_phrasing(&mut notes, list, &cfg.phrase, &mut phrases);
        }
    }

    // fast-RUN → marcato: a sustained run of fast notes switches to marcato-
    // with-overlay (sustain legato can't articulate that fast). Detection is
    // by RUN — a 2-3 note flip stays legato. Only plain sustains convert.
    if cfg.marcato_fast {
        for list in by_ch.values() {
            let mut i = 0usize;
            while i < list.len() {
                let mut j = i;
                while j + 1 < list.len() {
                    let cur = &notes[list[j]];
                    let next = &notes[list[j + 1]];
                    let io_sec = (next.onset - cur.onset) * 60.0 / bpm_at(&tempos, cur.onset);
                    if io_sec < cfg.marcato_max_sec {
                        j += 1;
                    } else {
                        break;
                    }
                }
                if j - i + 1 >= cfg.marcato_min_run {
                    for &k in &list[i..=j] {
                        if notes[k].artic == "sustain" {
                            notes[k].artic = "marcato";
                            notes[k].fast_marcato = true;
                        }
                    }
                }
                i = if j > i { j + 1 } else { i + 1 };
            }
        }
    }

    // velocities (legato speed) + accents / shorts / portamento / harp
    for list in by_ch.values() {
        for (i, &ni) in list.iter().enumerate() {
            let n = &notes[ni];
            let mut vel = if n.artic == "marcato" {
                // overlay volume scales with the dynamic; stays >= marcato_vel_min
                let dv = dyn_interp(&anchors, n.onset).unwrap_or(cfg.dyn_default);
                cfg.marcato_vel_min + (dv / 127.0) * cfg.marcato_vel_range
            } else if n.legato_from && i > 0 {
                // Legato speed = the SHORTER of the gaps to prev and next notes.
                // Deterministic (onsets only) so same-rhythm voices stay locked.
                let io_prev = n.onset - notes[list[i - 1]].onset;
                let io_next = list
                    .get(i + 1)
                    .map(|&x| notes[x].onset - n.onset)
                    .unwrap_or(io_prev);
                let io_sec = io_prev.min(io_next) * 60.0 / bpm_at(&tempos, n.onset);
                let mut v = cfg.legato_vel_from_interval(io_sec);
                // phrase-level legato timing follows the arch
                if cfg.phrasing {
                    if let Some(ph) = n.phrase.map(|x| &phrases[x]) {
                        let p = &cfg.phrase;
                        let an = (arch_at(ph, n.onset, p) / p.arch_gain.max(1.0)).clamp(-1.2, 1.2);
                        v += p.legato_vel_mod * an * p.intensity;
                    }
                }
                v
            } else {
                cfg.vel_first
            };
            // explicit accents get a stronger attack/overlay
            if n.art.has("accent") || n.art.has("strong-accent") {
                vel = vel.max(cfg.vel_accent_min);
            }
            if is_short(n.artic) {
                // short notes: loudness lives partly in velocity too
                let dv = dyn_interp(&anchors, n.onset).unwrap_or(cfg.dyn_default);
                vel = dv.floor().clamp(35.0, 127.0);
                if n.art.has("accent") {
                    vel = vel.max(cfg.vel_accent_min);
                }
            }
            // portamento: a slurred gliss triggers CSS's sampled slide when the
            // legato transition velocity is below the portamento threshold.
            let mut porta = false;
            if cfg.portamento && prof.porta && n.art.has("glissando") && n.legato_from {
                vel = vel.min(cfg.port_vel);
                porta = true;
            }
            // harp/piano: dynamics ARE the velocity (pluck strength)
            if prof.vel_dynamics {
                vel = dyn_interp(&anchors, n.onset).unwrap_or(cfg.dyn_default);
            }
            let n = &mut notes[ni];
            n.porta = porta;
            n.vel = (vel + 0.5).floor().clamp(1.0, 127.0);
        }
    }

    // fade shaping: mark held notes that end a phrase into PART-WIDE silence
    // (a divisi note whose own channel rests must not fade while the section
    // still plays on another channel). Only the global first note blooms in.
    if cfg.fade_to_niente || cfg.fade_in {
        let global_first = notes.iter().map(|n| n.onset).fold(f64::INFINITY, f64::min);
        let snapshot: Vec<(f64, f64)> = notes.iter().map(|n| (n.onset, n.dur)).collect();
        for (i, n) in notes.iter_mut().enumerate() {
            let short = is_short(n.artic);
            let is_long = n.dur >= cfg.fade_min_note_dur_qn;
            let n_end = n.onset + n.dur;
            // gap until the PART next sounds (any channel); 0 if something's on
            let mut part_gap = f64::INFINITY;
            for (j, &(onset, dur)) in snapshot.iter().enumerate() {
                if j != i && onset + dur > n_end + EPS {
                    part_gap = part_gap.min((onset - n_end).max(0.0));
                }
            }
            let out = cfg.fade_to_niente && !short && is_long && part_gap >= cfg.fade_min_rest_qn;
            // Only a SOFT entrance blooms in — a part entering at mf/f attacks
            // at its marked level.
            let dyn_here = dyn_interp(&anchors, n.onset).unwrap_or(cfg.dyn_default);
            let fin = cfg.fade_in
                && n.onset <= global_first + EPS
                && !short
                && n.dur >= cfg.fade_in_min_dur_qn
                && dyn_here <= cfg.fade_in_max_dyn;
            n.fade_mode = match (out, fin) {
                (true, true) => Some(FadeMode::Swell),
                (true, false) => Some(FadeMode::Out),
                (false, true) => Some(FadeMode::In),
                (false, false) => None,
            };
            if n.fade_mode.is_some() {
                n.fade_peak = dyn_here;
            }
        }
    }

    // timing compensation: pull each note's MIDI start earlier by its sampled-
    // attack delay so the AUDIBLE attack lands on the beat. Ends stay put.
    if cfg.timing_comp {
        for n in notes.iter_mut() {
            // write only the part not covered by the track's -track_delay_ms
            let script_ms = total_delay_ms_for(n, cfg, prof) - cfg.track_delay_ms;
            n.lead_ms = script_ms;
            if script_ms != 0.0 {
                let lead_qn = (script_ms * bpm_at(&tempos, n.onset) / 60000.0)
                    .clamp(-cfg.max_lead_qn, cfg.max_lead_qn);
                n.start = n.onset - lead_qn; // positive lead = earlier
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
        // bounds may have moved earlier
        start_qn = notes.iter().map(|n| n.start).fold(f64::INFINITY, f64::min);
        item_start_qn = (start_qn - cfg.lead_in_qn).max(0.0);
    }

    // HARD GUARANTEE: two notes of the same pitch on a channel must NEVER
    // overlap, or the sampler hangs. Legato overlap of DIFFERENT pitches is
    // fine; for same-pitch repeats the CC64 re-bow pedal carries the sound.
    for list in by_ch.values_mut() {
        list.sort_by(|&a, &b| notes[a].start.total_cmp(&notes[b].start));
        let mut last_by_pitch: BTreeMap<i32, usize> = BTreeMap::new();
        for &ni in list.iter() {
            if let Some(&pi) = last_by_pitch.get(&notes[ni].pitch) {
                if notes[pi].stop > notes[ni].start - cfg.break_gap_qn {
                    let mut s = notes[ni].start - cfg.break_gap_qn;
                    if s <= notes[pi].start {
                        s = (notes[pi].start + notes[ni].start) * 0.5; // keep positive length
                    }
                    notes[pi].stop = s;
                }
            }
            last_by_pitch.insert(notes[ni].pitch, ni);
        }
    }

    // ------------------------------------------------------------------
    // emit events
    // ------------------------------------------------------------------
    let out_notes: Vec<OutNote> = notes
        .iter()
        .map(|n| OutNote {
            start_qn: n.start,
            end_qn: n.stop,
            chan: n.chan,
            pitch: n.pitch,
            vel: n.vel as u8,
            artic: n.artic,
            lead_ms: n.lead_ms,
            onset_qn: n.onset,
            score_art: n.art.joined(),
            slur_start: n.slur_start,
            slur_stop: n.slur_stop,
            legato_from: n.legato_from,
            slurred: n.slurred,
            solo: n.solo,
        })
        .collect();

    let mut ccs: Vec<CcEvent> = Vec::new();
    let clampu = |v: f64| -> u8 { (v + 0.5).floor().clamp(0.0, 127.0) as u8 };

    // Keyswitch (CC58) timeline, PER CHANNEL so each divisi voice keeps its own
    // articulation state. CSS shares one CC for legato-toggle + articulation
    // select, so every "press" needs its own tick, all before the first note.
    let ks_lead_qn = 1.0 / 64.0;
    if !prof.no_keyswitch {
        // events common to all channels: Legato On at start, con/senza sordino
        let mut shared: Vec<(f64, u8)> = vec![(item_start_qn, cfg.ks.legato_on)];
        if cfg.con_sord && prof.con_sord {
            for mk in &part.markings {
                if let MarkingKind::Words(text) = &mk.kind {
                    let w = text.to_lowercase();
                    let q = (mk.qn - ks_lead_qn).max(item_start_qn);
                    if w.contains("senza sord") || w.contains("via sord") || w.contains("ord.") {
                        shared.push((q, cfg.ks.con_sord_off));
                    } else if w.contains("con sord")
                        || w.contains("muted")
                        || w.contains("with mute")
                    {
                        shared.push((q, cfg.ks.con_sord_on));
                    }
                }
            }
        }

        for &ch in &channels {
            let mut tl: Vec<(f64, u8)> = shared.clone();
            let mut list: Vec<usize> = by_ch.get(&ch).cloned().unwrap_or_default();
            list.sort_by(|&a, &b| {
                notes[a]
                    .start
                    .total_cmp(&notes[b].start)
                    .then_with(|| notes[b].pitch.cmp(&notes[a].pitch))
            });
            let mut cur: Option<u8> = None;
            for &ni in &list {
                let want = ks_value(notes[ni].artic, cfg, prof);
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
    }

    // Re-bow pedal: hold CC64 across runs of repeated (re-bowed) notes. CC64
    // is CSS's re-bow/re-tongue trigger, so the stream must read cleanly as a
    // state machine: when runs on one channel overlap in time (divisi voices
    // sharing a channel), their spans are MERGED — the Lua engine emitted
    // them interleaved, which dropped the pedal mid-run.
    if cfg.re_bow {
        let ped_qn = 1.0 / 128.0;
        for list in by_ch.values() {
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
            if spans.is_empty() {
                continue;
            }
            let chan = notes[list[0]].chan;
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
                    chan,
                    cc: cfg.cc_sustain_pedal,
                    val: 127,
                });
                ccs.push(CcEvent {
                    qn: off,
                    chan,
                    cc: cfg.cc_sustain_pedal,
                    val: 0,
                });
            }
        }
    }

    // Portamento volume (CC5) at slide notes.
    if cfg.portamento {
        for n in &notes {
            if n.porta {
                ccs.push(CcEvent {
                    qn: (n.start - 1.0 / 128.0).max(item_start_qn),
                    chan: n.chan,
                    cc: cfg.cc_portamento,
                    val: clampu(cfg.port_vol),
                });
            }
        }
    }

    // Reset CC11 volume to full at item start on every channel (CSS latches
    // the last CC11 it received; a low leftover keeps a channel quiet).
    if cfg.reset_volume {
        for ch in 1..=cfg.max_channels {
            ccs.push(CcEvent {
                qn: item_start_qn,
                chan: ch,
                cc: cfg.cc_fade_volume,
                val: 127,
            });
        }
    }

    // CC1 / CC2 dynamics on a grid + at every anchor. Macro dynamics come from
    // the marks (shared); micro-dynamics (phrase shaping) are per channel.
    // Skipped entirely for velocity-dynamics instruments (harp).
    if !prof.no_cc_dynamics {
        let mut points: Vec<f64> = vec![item_start_qn];
        for &(qn, _) in &anchors {
            if qn >= item_start_qn && qn <= end_qn {
                points.push(qn);
            }
        }
        let mut qn = item_start_qn;
        while qn < end_qn {
            qn += cfg.cc_grid_qn;
            points.push(qn.min(end_qn));
        }
        points.sort_by(|a, b| a.total_cmp(b));

        let p = &cfg.phrase;

        // per-channel emission so each divisi voice shapes its own phrase
        for &ch in &channels {
            let list = by_ch.get(&ch).cloned().unwrap_or_default();
            let mut ptr = 0usize;
            let mut active: Option<usize> = None;
            let (mut last_v1, mut last_v2, mut last_vol): (Option<f64>, Option<f64>, Option<f64>) =
                (None, None, None);

            for &q in &points {
                while ptr < list.len() && notes[list[ptr]].onset <= q + EPS {
                    active = Some(list[ptr]);
                    ptr += 1;
                }
                let act = active.map(|i| &notes[i]);
                let sounding = act.map(|a| q <= a.onset + a.dur + EPS).unwrap_or(false);
                let tenuto_active = act
                    .map(|a| {
                        a.art.has("tenuto") && q >= a.onset - EPS && q <= a.onset + a.dur + EPS
                    })
                    .unwrap_or(false);

                // Hold the dynamics at the floor through a fade-out's tail so
                // the release decays instead of sparking back up.
                let mut hold_tail = false;
                if let (false, true, Some(a)) = (sounding, cfg.fade_tail_hold, act) {
                    if matches!(a.fade_mode, Some(FadeMode::Out) | Some(FadeMode::Swell)) {
                        let beats = if a.beats == 0 { 4 } else { a.beats };
                        let bt = if a.beat_type == 0 { 4 } else { a.beat_type };
                        let bar_len = beats as f64 * 4.0 / bt as f64;
                        let next_onset = list
                            .get(ptr)
                            .map(|&x| notes[x].onset)
                            .unwrap_or(f64::INFINITY);
                        hold_tail = q
                            < (next_onset - cfg.fade_recover_lead_qn)
                                .min(a.onset + a.dur + cfg.fade_tail_bars * bar_len);
                    }
                }

                // fade shaping overrides the dynamic while a fade note sounds
                let (v1_raw, fade_vol) = if let (true, Some(a)) = (sounding, act) {
                    if let Some(mode) = a.fade_mode {
                        let u = (q - a.onset) / a.dur.max(EPS);
                        let (cc1, vol_frac) = fade_shape(mode, u, a.fade_peak, cfg);
                        (cc1, cfg.fade_use_volume.then_some(127.0 * vol_frac))
                    } else {
                        let macro_v = dyn_interp(&anchors, q).unwrap_or(cfg.dyn_default);
                        let micro = if cfg.phrasing {
                            micro_for(a, &phrases, q, p)
                        } else {
                            0.0
                        };
                        (macro_v + micro, None)
                    }
                } else if hold_tail {
                    (cfg.fade_niente_floor, cfg.fade_use_volume.then_some(0.0))
                } else {
                    let macro_v = dyn_interp(&anchors, q).unwrap_or(cfg.dyn_default);
                    (macro_v, None)
                };
                let v1 = (v1_raw + 0.5).floor().clamp(0.0, 127.0);
                if last_v1.is_none_or(|l| (v1 - l).abs() >= cfg.cc_deadband) || q == item_start_qn {
                    ccs.push(CcEvent {
                        qn: q,
                        chan: ch,
                        cc: cfg.cc_dynamics,
                        val: v1 as u8,
                    });
                    last_v1 = Some(v1);
                }

                // CC11 volume: full normally, ramps for a true fade to silence
                if cfg.fade_use_volume {
                    let vol = (fade_vol.unwrap_or(127.0) + 0.5).floor().clamp(0.0, 127.0);
                    if last_vol.is_none_or(|l| (vol - l).abs() >= cfg.cc_deadband)
                        || q == item_start_qn
                    {
                        ccs.push(CcEvent {
                            qn: q,
                            chan: ch,
                            cc: cfg.cc_fade_volume,
                            val: vol as u8,
                        });
                        last_vol = Some(vol);
                    }
                }

                // CC2 vibrato: xfade follows CC1 (+bloom), switch is on/off
                if prof.vib != VibratoMode::None {
                    let mut v2 = if prof.vib == VibratoMode::Switch {
                        if tenuto_active { 0.0 } else { 127.0 }
                    } else {
                        let mut v = v1 * cfg.vib_follow + cfg.vib_base;
                        if tenuto_active {
                            v -= cfg.vib_tenuto_drop;
                        }
                        if let (true, true, Some(a)) = (cfg.phrasing, sounding, act) {
                            if p.do_vib_bloom {
                                v += vib_bloom_at(a, q, p);
                            }
                        }
                        v
                    };
                    v2 = (v2 + 0.5).floor().clamp(0.0, 127.0);
                    if last_v2.is_none_or(|l| (v2 - l).abs() >= cfg.cc_deadband)
                        || q == item_start_qn
                    {
                        ccs.push(CcEvent {
                            qn: q,
                            chan: ch,
                            cc: cfg.cc_vibrato,
                            val: v2 as u8,
                        });
                        last_v2 = Some(v2);
                    }
                }
            }
        }
    }

    ccs.sort_by(|a, b| {
        a.qn.total_cmp(&b.qn)
            .then_with(|| a.chan.cmp(&b.chan))
            .then_with(|| a.cc.cmp(&b.cc))
    });

    PartOutput {
        notes: out_notes,
        ccs,
        channels,
        markings: part.markings.clone(),
        start_qn,
        end_qn,
        item_start_qn,
        item_end_qn: end_qn + cfg.item_tail_qn, // item extends past the last note-off
        empty: false,
    }
}
