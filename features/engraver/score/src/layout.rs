//! Single-part page layout (P2 of the score-engraving effort).
//!
//! Turns one [`crate::model::Part`] into engraved pages: systems of measures
//! with clef/key/time prefixes, beamed rhythms via the engraver's
//! `MeasureBuilder`, key-aware accidentals with measure-local state, and
//! duration-based multi-measure rests (Finale writes plain `<rest/>` bars,
//! never `measure="yes"`).
//!
//! Coordinate model matches the chart pipeline: one scene, pages stacked
//! vertically, each page cropped at export time by its `PageRect`.

use engraver_proto::engraver::layout::chart::prefix_renderer::{
    PrefixRenderContext, calculate_prefix_width, render_system_prefix,
};
use engraver_proto::engraver::layout::chart::{constants, measure_layout, spacing as chart_spacing};
use engraver_proto::engraver::layout::context::LayoutContextOwned;
use engraver_proto::engraver::layout::tlayout::{Accidental, ClefType, TupletRatio};
use engraver_proto::engraver::model::{Octave, Pitch, PitchClass};
use engraver_proto::engraver::notation::{
    Duration, DurationKind, MeasureBuilder, RhythmEntry, TimeSignature,
};
use engraver_proto::engraver::scene::id::{ElementType, SemanticId};
use engraver_proto::engraver::scene::node::SceneNode;
use engraver_proto::engraver::scene::paint::{FontStyle, FontWeight, PaintCommand, TextAnchor};
use kurbo::{Affine, Point, Rect};
use peniko::Color;

use crate::model::{self, ClefSign, Event, NoteTypeValue, Part, Score, Step, WrittenPitch};

/// Page + engraving options. Defaults follow the Columbus part books:
/// US Letter portrait, generous margins, 6.5pt spatium.
#[derive(Debug, Clone)]
pub struct LayoutOptions {
    pub page_width: f64,
    pub page_height: f64,
    pub margin: f64,
    /// Staff space in points.
    pub spatium: f64,
    /// Vertical distance between system tops.
    pub system_gap: f64,
    /// Vertical space reserved for the page-1 title block.
    pub title_block_height: f64,
    /// Minimum measures a rest run needs to collapse into a multirest.
    pub multirest_min: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            page_width: 612.0,
            page_height: 792.0,
            margin: 44.0,
            spatium: 6.5,
            system_gap: 74.0,
            title_block_height: 84.0,
            multirest_min: 2,
        }
    }
}

/// One exported page's crop box within the shared scene.
#[derive(Debug, Clone, Copy)]
pub struct PageRect {
    pub x_offset: f64,
    pub y_offset: f64,
    pub width: f64,
    pub height: f64,
}

/// Result of laying out one part.
pub struct PartLayout {
    pub scene: SceneNode,
    pub pages: Vec<PageRect>,
    /// Diagnostics: events dropped because they sat in a non-primary voice.
    pub dropped_voice_events: usize,
}

// ───────────────────────── measure preparation ─────────────────────────

/// A prepared measure ready for `MeasureBuilder`, or a multirest run.
enum Prepared {
    Notated(Box<NotatedMeasure>),
    /// `count` consecutive resting measures collapsed into one H-bar block.
    MultiRest {
        count: usize,
        number: String,
        clef: ClefType,
        key_fifths: i8,
        time: (u32, u32),
    },
}

struct NotatedMeasure {
    number: String,
    entries: Vec<RhythmEntry>,
    pitches: Vec<Option<(i32, Accidental)>>,
    stacks: Vec<Vec<(i32, Accidental)>>,
    tuplets: Vec<(usize, usize, TupletRatio)>,
    /// Per entry: staff lines of noteheads that START a tie.
    tie_start_lines: Vec<Vec<i32>>,
    /// Per entry: staff lines of noteheads that END a tie.
    tie_stop_lines: Vec<Vec<i32>>,
    /// Per entry: slur start/stop flags.
    slur_starts: Vec<bool>,
    slur_stops: Vec<bool>,
    /// Per entry: tick onset (for direction anchoring).
    entry_ticks: Vec<u32>,
    /// Per entry: articulation tags (staccato, accent, …).
    arts: Vec<Vec<String>>,
    /// Directions in this measure (dynamics, hairpins, texts).
    directions: Vec<model::Direction>,
    /// Clef change taking effect at this measure (drawn in-measure when
    /// mid-system, absorbed by the prefix at system start).
    clef: ClefType,
    clef_changed: bool,
    key_fifths: i8,
    key_changed: bool,
    time: (u32, u32),
    time_changed: bool,
}

impl NotatedMeasure {
    /// A whole-bar-rest measure with the given attribute state.
    fn rest_bar(number: String, clef: ClefType, key_fifths: i8, time: (u32, u32)) -> Self {
        Self {
            number,
            entries: vec![RhythmEntry::Rest(Duration::Whole)],
            pitches: vec![None],
            stacks: vec![Vec::new()],
            tuplets: Vec::new(),
            tie_start_lines: vec![Vec::new()],
            tie_stop_lines: vec![Vec::new()],
            slur_starts: vec![false],
            slur_stops: vec![false],
            entry_ticks: vec![0],
            arts: vec![Vec::new()],
            directions: Vec::new(),
            clef,
            clef_changed: false,
            key_fifths,
            key_changed: false,
            time,
            time_changed: false,
        }
    }
}

/// Walk state: clef/key/time inherited across measures.
struct AttrState {
    clef: ClefType,
    key_fifths: i8,
    time: (u32, u32),
}

fn clef_type(clef: &model::Clef) -> ClefType {
    match (clef.sign, clef.line) {
        (ClefSign::G, _) => ClefType::Treble,
        (ClefSign::F, _) => ClefType::Bass,
        (ClefSign::C, Some(4)) => ClefType::Tenor,
        (ClefSign::C, _) => ClefType::Alto,
        (ClefSign::Percussion, _) => ClefType::Percussion,
        (ClefSign::Tab, _) => ClefType::Tab,
        (ClefSign::None, _) => ClefType::Treble,
    }
}

/// Staff position (diatonic steps) of the pitch sitting ON the middle line.
fn middle_line_ref(clef: ClefType, octave_change: i8) -> i32 {
    let base = match clef {
        ClefType::Treble => Pitch::new(PitchClass::B, Octave::new(4)),
        ClefType::Bass => Pitch::new(PitchClass::D, Octave::new(3)),
        ClefType::Alto => Pitch::new(PitchClass::C, Octave::new(4)),
        ClefType::Tenor => Pitch::new(PitchClass::A, Octave::new(3)),
        // Percussion/tab: treat like treble for display positions.
        ClefType::Percussion | ClefType::Tab => Pitch::new(PitchClass::B, Octave::new(4)),
    };
    base.staff_position() + i32::from(octave_change) * 7
}

fn pitch_class(step: Step) -> PitchClass {
    match step {
        Step::A => PitchClass::A,
        Step::B => PitchClass::B,
        Step::C => PitchClass::C,
        Step::D => PitchClass::D,
        Step::E => PitchClass::E,
        Step::F => PitchClass::F,
        Step::G => PitchClass::G,
    }
}

/// Alteration the key signature implies for a step (sharps F C G D A E B,
/// flats B E A D G C F).
fn key_alteration(step: Step, fifths: i8) -> i8 {
    const SHARPS: [Step; 7] = [Step::F, Step::C, Step::G, Step::D, Step::A, Step::E, Step::B];
    if fifths > 0 {
        let n = fifths.min(7) as usize;
        i8::from(SHARPS[..n].contains(&step))
    } else if fifths < 0 {
        let n = (-fifths).min(7) as usize;
        -i8::from(SHARPS[7 - n..].contains(&step))
    } else {
        0
    }
}

fn accidental_from_alter(alter: i8) -> Accidental {
    match alter {
        1 => Accidental::Sharp,
        -1 => Accidental::Flat,
        2 => Accidental::DoubleSharp,
        -2 => Accidental::DoubleFlat,
        _ => Accidental::Natural,
    }
}

/// Measure-local accidental memory: (step, octave) → alteration in effect.
#[derive(Default)]
struct AccidentalState(std::collections::HashMap<(Step, i8), i8>);

impl AccidentalState {
    /// Decide the printed accidental for a note and update the memory.
    fn resolve(&mut self, pitch: &WrittenPitch, key_fifths: i8) -> Accidental {
        let key = (pitch.step, pitch.octave);
        let in_effect = self
            .0
            .get(&key)
            .copied()
            .unwrap_or_else(|| key_alteration(pitch.step, key_fifths));
        if pitch.alter == in_effect {
            return Accidental::None;
        }
        self.0.insert(key, pitch.alter);
        accidental_from_alter(pitch.alter)
    }
}

fn duration_from_type(t: NoteTypeValue, dots: u8, tuplet: Option<(u8, u8)>) -> Duration {
    let kind = match t {
        NoteTypeValue::Maxima | NoteTypeValue::Long | NoteTypeValue::Breve | NoteTypeValue::Whole => {
            DurationKind::Whole
        }
        NoteTypeValue::Half => DurationKind::Half,
        NoteTypeValue::Quarter => DurationKind::Quarter,
        NoteTypeValue::Eighth => DurationKind::Eighth,
        NoteTypeValue::Sixteenth => DurationKind::Sixteenth,
        NoteTypeValue::ThirtySecond => DurationKind::ThirtySecond,
        _ => DurationKind::SixtyFourth,
    };
    Duration {
        kind,
        dots: dots.min(2),
        tuplet: tuplet.map(|(a, n)| TupletRatio {
            numerator: a,
            denominator: n,
        }),
    }
}

/// Derive a written duration from raw ticks when `<type>` is absent.
fn duration_from_ticks(ticks: u32, divisions: u32) -> Duration {
    let quarters = f64::from(ticks) / f64::from(divisions.max(1));
    const KINDS: [DurationKind; 7] = [
        DurationKind::Whole,
        DurationKind::Half,
        DurationKind::Quarter,
        DurationKind::Eighth,
        DurationKind::Sixteenth,
        DurationKind::ThirtySecond,
        DurationKind::SixtyFourth,
    ];
    let mut best = (f64::MAX, Duration::Quarter);
    for kind in KINDS {
        for dots in 0..=2u8 {
            let mut value = kind.quarters();
            if dots >= 1 {
                value *= 1.5;
            }
            if dots == 2 {
                value = kind.quarters() * 1.75;
            }
            let err = (value - quarters).abs();
            if err < best.0 {
                best = (
                    err,
                    Duration {
                        kind,
                        dots,
                        tuplet: None,
                    },
                );
            }
        }
    }
    best.1
}

/// Is this measure "empty" — nothing but rests (or nothing at all)?
fn is_resting(measure: &model::Measure) -> bool {
    measure
        .events
        .iter()
        .all(|e| matches!(e, Event::Rest(_)))
}

/// Choose the primary voice for a staff-1 measure: the voice with the most
/// chord events (ties broken toward the lowest voice number).
fn primary_voice(measure: &model::Measure) -> Option<u32> {
    let mut counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for event in &measure.events {
        if let Event::Chord(c) = event
            && c.staff == 1
        {
            *counts.entry(c.voice).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(v, _)| v)
}

fn prepare_measures(
    part: &Part,
    opts: &LayoutOptions,
    dropped: &mut usize,
) -> Vec<Prepared> {
    let mut state = AttrState {
        clef: ClefType::Treble,
        key_fifths: 0,
        time: (4, 4),
    };
    // Seed from the first measure's attributes so the initial prefix is right
    // even before the loop applies them.
    let mut prepared = Vec::new();
    // Pending resting measures: (number, clef, key, time) captured at walk time.
    type RestInfo = (String, ClefType, i8, (u32, u32));
    let mut rest_run: Vec<RestInfo> = Vec::new();

    let flush_rests = |run: &mut Vec<RestInfo>, out: &mut Vec<Prepared>, opts: &LayoutOptions| {
        if run.is_empty() {
            return;
        }
        if run.len() >= opts.multirest_min {
            let (number, clef, key_fifths, time) = run[0].clone();
            out.push(Prepared::MultiRest {
                count: run.len(),
                number,
                clef,
                key_fifths,
                time,
            });
        } else {
            for (number, clef, key_fifths, time) in run.iter() {
                out.push(Prepared::Notated(Box::new(NotatedMeasure::rest_bar(
                    number.clone(),
                    *clef,
                    *key_fifths,
                    *time,
                ))));
            }
        }
        run.clear();
    };

    for measure in &part.measures {
        let mut clef_changed = false;
        let mut key_changed = false;
        let mut time_changed = false;
        for change in &measure.attributes {
            if let Some((staff, clef)) = change.clefs.iter().find(|(s, _)| *s == 1) {
                let _ = staff;
                let ct = clef_type(clef);
                if ct != state.clef {
                    state.clef = ct;
                    clef_changed = true;
                }
            }
            if let Some(k) = change.key_fifths
                && k != state.key_fifths
            {
                state.key_fifths = k;
                key_changed = true;
            }
            if let Some(t) = change.time
                && t != state.time
            {
                state.time = t;
                time_changed = true;
            }
        }

        if is_resting(measure) && !clef_changed && !key_changed && !time_changed {
            rest_run.push((
                measure.number.clone(),
                state.clef,
                state.key_fifths,
                state.time,
            ));
            continue;
        }
        flush_rests(&mut rest_run, &mut prepared, opts);

        let Some(voice) = primary_voice(measure) else {
            // Rests only but with attribute changes: render a whole-rest bar.
            let mut bar = NotatedMeasure::rest_bar(
                measure.number.clone(),
                state.clef,
                state.key_fifths,
                state.time,
            );
            bar.clef_changed = clef_changed;
            bar.key_changed = key_changed;
            bar.time_changed = time_changed;
            prepared.push(Prepared::Notated(Box::new(bar)));
            continue;
        };

        // Homophonic second voice: when another voice's chords share the
        // primary voice's exact rhythm signature (tick + duration), merge its
        // noteheads into the primary chords instead of dropping them
        // (violin div., horns a2…). True polyphony is P3.
        let merged_voice = homophonic_partner(measure, voice);

        let octave_change = 0i8; // per-measure clef octave marks: P3
        let middle = middle_line_ref(state.clef, octave_change);
        let mut acc_state = AccidentalState::default();
        let mut entries = Vec::new();
        let mut pitches = Vec::new();
        let mut stacks = Vec::new();
        let mut tuplet_marks: Vec<Option<TupletRatio>> = Vec::new();

        let mut tie_start_lines: Vec<Vec<i32>> = Vec::new();
        let mut tie_stop_lines: Vec<Vec<i32>> = Vec::new();
        let mut slur_starts: Vec<bool> = Vec::new();
        let mut slur_stops: Vec<bool> = Vec::new();
        let mut entry_ticks: Vec<u32> = Vec::new();
        let mut arts: Vec<Vec<String>> = Vec::new();

        // Chords of the merged voice, keyed by (tick, duration).
        let partner_chords: std::collections::HashMap<(u32, u32), &model::Chord> = merged_voice
            .map(|mv| {
                measure
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        Event::Chord(c) if c.voice == mv && c.staff == 1 && !c.grace => {
                            Some(((c.tick, c.duration), c))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut events: Vec<&Event> = measure
            .events
            .iter()
            .filter(|e| {
                let is_merged_chord = matches!(
                    (e, merged_voice),
                    (Event::Chord(c), Some(mv)) if c.voice == mv && c.staff == 1 && !c.grace
                );
                let keep = e.voice() == voice
                    && match e {
                        Event::Chord(c) => c.staff == 1 && !c.grace,
                        Event::Rest(r) => r.staff == 1,
                    };
                if !keep && !is_merged_chord && !matches!((e, merged_voice), (Event::Rest(r), Some(mv)) if r.voice == mv)
                {
                    *dropped += 1;
                }
                keep
            })
            .collect();
        events.sort_by_key(|e| e.tick());

        for event in events {
            match event {
                Event::Chord(chord) => {
                    let duration = chord
                        .written
                        .note_type
                        .map(|t| duration_from_type(t, chord.written.dots, chord.written.tuplet))
                        .unwrap_or_else(|| duration_from_ticks(chord.duration, measure.divisions));
                    entries.push(RhythmEntry::Note(duration));
                    tuplet_marks.push(duration.tuplet);

                    let partner = partner_chords.get(&(chord.tick, chord.duration));
                    let all_notes = chord
                        .notes
                        .iter()
                        .chain(partner.into_iter().flat_map(|p| p.notes.iter()));

                    let mut lines: Vec<(i32, Accidental)> = Vec::new();
                    let mut starts: Vec<i32> = Vec::new();
                    let mut stops: Vec<i32> = Vec::new();
                    for n in all_notes {
                        let p = Pitch::with_alteration(
                            pitch_class(n.pitch.step),
                            Octave::new(n.pitch.octave),
                            n.pitch.alter,
                        );
                        let line = p.staff_position() - middle;
                        let acc = acc_state.resolve(&n.pitch, state.key_fifths);
                        if !lines.iter().any(|(l, _)| *l == line) {
                            lines.push((line, acc));
                        }
                        if n.tie_start {
                            starts.push(line);
                        }
                        if n.tie_stop {
                            stops.push(line);
                        }
                    }
                    // Primary head = highest note; extras stack below it.
                    lines.sort_by_key(|(line, _)| -line);
                    let primary = lines.remove(0);
                    pitches.push(Some(primary));
                    stacks.push(lines);
                    tie_start_lines.push(starts);
                    tie_stop_lines.push(stops);
                    slur_starts.push(chord.slur_start);
                    slur_stops.push(chord.slur_stop);
                    entry_ticks.push(chord.tick);
                    arts.push(chord.articulations.iter().cloned().collect());
                }
                Event::Rest(rest) => {
                    let duration = if rest.measure_rest || rest.duration >= measure.len_ticks {
                        Duration::Whole
                    } else {
                        rest.written
                            .note_type
                            .map(|t| duration_from_type(t, rest.written.dots, rest.written.tuplet))
                            .unwrap_or_else(|| {
                                duration_from_ticks(rest.duration, measure.divisions)
                            })
                    };
                    entries.push(RhythmEntry::Rest(duration));
                    tuplet_marks.push(duration.tuplet);
                    pitches.push(None);
                    stacks.push(Vec::new());
                    tie_start_lines.push(Vec::new());
                    tie_stop_lines.push(Vec::new());
                    slur_starts.push(false);
                    slur_stops.push(false);
                    entry_ticks.push(rest.tick);
                    arts.push(Vec::new());
                }
            }
        }

        if entries.is_empty() {
            entries.push(RhythmEntry::Rest(Duration::Whole));
            pitches.push(None);
            stacks.push(Vec::new());
            tuplet_marks.push(None);
            tie_start_lines.push(Vec::new());
            tie_stop_lines.push(Vec::new());
            slur_starts.push(false);
            slur_stops.push(false);
            entry_ticks.push(0);
            arts.push(Vec::new());
        }

        // Contiguous runs of same-ratio tuplet entries become bracket groups.
        let mut tuplets = Vec::new();
        let mut run_start: Option<(usize, TupletRatio)> = None;
        for (idx, mark) in tuplet_marks.iter().enumerate() {
            match (run_start, mark) {
                (None, Some(r)) => run_start = Some((idx, *r)),
                (Some((start, r)), Some(m)) if *m == r => {
                    let _ = start; // run continues
                }
                (Some((start, r)), _) => {
                    if idx - start >= 2 {
                        tuplets.push((start, idx - 1, r));
                    }
                    run_start = mark.as_ref().map(|m| (idx, *m));
                }
                (None, None) => {}
            }
        }
        if let Some((start, r)) = run_start
            && tuplet_marks.len() - start >= 2
        {
            tuplets.push((start, tuplet_marks.len() - 1, r));
        }

        prepared.push(Prepared::Notated(Box::new(NotatedMeasure {
            number: measure.number.clone(),
            entries,
            pitches,
            stacks,
            tuplets,
            tie_start_lines,
            tie_stop_lines,
            slur_starts,
            slur_stops,
            entry_ticks,
            arts,
            directions: measure.directions.clone(),
            clef: state.clef,
            clef_changed,
            key_fifths: state.key_fifths,
            key_changed,
            time: state.time,
            time_changed,
        })));
    }
    flush_rests(&mut rest_run, &mut prepared, opts);
    prepared
}

/// Find a second voice whose chords share the primary voice's exact rhythm
/// signature (same (tick, duration) multiset) — safe to merge as stacked
/// noteheads.
fn homophonic_partner(measure: &model::Measure, primary: u32) -> Option<u32> {
    let signature = |v: u32| -> Vec<(u32, u32)> {
        let mut sig: Vec<(u32, u32)> = measure
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Chord(c) if c.voice == v && c.staff == 1 && !c.grace => {
                    Some((c.tick, c.duration))
                }
                _ => None,
            })
            .collect();
        sig.sort_unstable();
        sig
    };
    let primary_sig = signature(primary);
    if primary_sig.is_empty() {
        return None;
    }
    let voices: std::collections::BTreeSet<u32> = measure
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Chord(c) if c.staff == 1 && !c.grace => Some(c.voice),
            _ => None,
        })
        .collect();
    voices
        .into_iter()
        .find(|v| *v != primary && signature(*v) == primary_sig)
}

// ───────────────────────── system + page assembly ─────────────────────────

const MULTIREST_WIDTH_SP: f64 = 16.0;

fn build_measure(
    m: &NotatedMeasure,
    ctx: &LayoutContextOwned,
    width_sp: Option<f64>,
    draw_clef: bool,
    draw_time: bool,
    id_base: u64,
) -> engraver_proto::engraver::notation::MeasureScene {
    let mut builder = MeasureBuilder::new()
        .id_base(id_base)
        .entries(m.entries.clone())
        .note_pitches(m.pitches.clone())
        .note_pitch_stacks(m.stacks.clone())
        .time_signature_meta(TimeSignature::new(m.time.0 as u8, m.time.1 as u8))
        .clef_meta(m.clef);
    if draw_clef {
        builder = builder.clef(m.clef);
    }
    if draw_time {
        builder = builder.time_signature(m.time.0 as u8, m.time.1 as u8);
    }
    for (start, end, ratio) in &m.tuplets {
        builder = builder.tuplet_group(*start, *end, *ratio);
    }
    if let Some(w) = width_sp {
        builder = builder.justify_to(w);
    } else {
        builder = builder.compact();
    }
    // System pass draws its own barlines: builder barlines use top-line y
    // coords while notes use middle-line coords (chart does the same).
    builder = builder.no_barlines();
    builder.build(&ctx.as_context())
}

/// A full-height barline at `x` in staff coords (top line = 0).
fn barline(x: f64, spatium: f64) -> SceneNode {
    SceneNode::anonymous_leaf(vec![PaintCommand::Line {
        start: Point::new(x, 0.0),
        end: Point::new(x, 4.0 * spatium),
        width: spatium * 0.16,
        color: Color::BLACK,
        line_cap: Default::default(),
    }])
}

/// Anchors for the tie/slur overlay: one notated measure with the
/// system-local x of each ChordRest segment, in entry order.
struct MeasureAnchors<'a> {
    m: &'a NotatedMeasure,
    entry_x: Vec<f64>,
}

/// Map a MusicXML articulation tag to a tlayout articulation type.
fn articulation_type(tag: &str) -> Option<engraver_proto::engraver::layout::tlayout::ArticulationType> {
    use engraver_proto::engraver::layout::tlayout::ArticulationType as A;
    Some(match tag {
        "staccato" => A::Staccato,
        "staccatissimo" => A::Staccatissimo,
        "accent" => A::Accent,
        "strong-accent" => A::Marcato,
        "tenuto" => A::Tenuto,
        "detached-legato" => A::TenutoStaccato,
        "stress" => A::Stress,
        "unstress" => A::Unstress,
        "soft-accent" => A::SoftAccent,
        _ => return None,
    })
}

/// Map a conventional dynamic name to the tlayout dynamic type.
fn dynamic_type(name: &str) -> engraver_proto::engraver::layout::tlayout::DynamicType {
    use engraver_proto::engraver::layout::tlayout::DynamicType as D;
    match name {
        "ppp" | "pppp" => D::Ppp,
        "pp" => D::Pp,
        "p" => D::P,
        "mp" => D::Mp,
        "mf" => D::Mf,
        "f" => D::F,
        "ff" => D::Ff,
        "fff" | "ffff" => D::Fff,
        "sf" | "sfz" => D::Sfz,
        "sffz" => D::Sffz,
        "fp" => D::Fp,
        "fz" => D::Fz,
        "rf" => D::Rf,
        "rfz" => D::Rfz,
        "sfp" => D::Sfp,
        _ => D::Other,
    }
}

/// Draw dynamics, hairpins, and articulation marks over a laid-out system.
fn draw_directions(
    sys_node: &mut SceneNode,
    anchors: &[MeasureAnchors<'_>],
    ctx: &LayoutContextOwned,
    system_width: f64,
    spatium: f64,
    id: &mut u64,
) {
    use engraver_proto::engraver::layout::tlayout::{
        DynamicsAlign, DynamicsParams, DynamicsPlacement, layout_dynamic,
    };
    let head_w = 1.18 * spatium;
    // Anchor x for a tick within one measure: last entry at or before it.
    let anchor_x = |a: &MeasureAnchors<'_>, tick: u32| -> f64 {
        let mut x = a.entry_x.first().copied().unwrap_or(0.0);
        for (i, t) in a.m.entry_ticks.iter().enumerate() {
            if *t <= tick
                && let Some(ex) = a.entry_x.get(i)
            {
                x = *ex;
            }
        }
        x
    };

    // ── dynamics + hairpins ──
    let mut wedge_open: Option<(f64, bool)> = None; // (start x, crescendo)
    let hairpin_y = 4.0 * spatium + 2.8 * spatium;
    let mut draw_wedge = |sys_node: &mut SceneNode, x1: f64, x2: f64, crescendo: bool| {
        if x2 <= x1 + spatium {
            return;
        }
        let half = 0.55 * spatium;
        let (open1, open2) = if crescendo { (0.0, half) } else { (half, 0.0) };
        let mut cmds = Vec::new();
        for sign in [-1.0, 1.0] {
            cmds.push(PaintCommand::Line {
                start: Point::new(x1, hairpin_y + sign * open1),
                end: Point::new(x2, hairpin_y + sign * open2),
                width: spatium * 0.12,
                color: Color::BLACK,
                line_cap: Default::default(),
            });
        }
        sys_node.add_child(SceneNode::anonymous_leaf(cmds));
    };

    for a in anchors {
        for d in &a.m.directions {
            match &d.kind {
                model::DirectionKind::Dynamic(name) => {
                    let (_, node) = layout_dynamic(
                        &DynamicsParams {
                            id: *id,
                            dynamic_type: dynamic_type(name),
                            custom_text: Some(name.clone()),
                            placement: DynamicsPlacement::Below,
                            align: DynamicsAlign::Center,
                            x: anchor_x(a, d.tick),
                            note_width: head_w,
                            center_on_notehead: true,
                        },
                        &ctx.as_context(),
                    );
                    *id += 1;
                    let mut container = SceneNode::group(SemanticId::new(ElementType::Dynamic, *id));
                    container.transform = Affine::translate((0.0, 4.0 * spatium));
                    container.add_child(node);
                    sys_node.add_child(container);
                }
                model::DirectionKind::WedgeStart { crescendo } => {
                    wedge_open = Some((anchor_x(a, d.tick), *crescendo));
                }
                model::DirectionKind::WedgeStop => {
                    if let Some((x1, cresc)) = wedge_open.take() {
                        draw_wedge(sys_node, x1, anchor_x(a, d.tick) + head_w, cresc);
                    }
                }
                model::DirectionKind::Words(_) | model::DirectionKind::Rehearsal(_) => {}
            }
        }
    }
    // Hairpin still open at line end: run it to the system edge.
    if let Some((x1, cresc)) = wedge_open {
        draw_wedge(sys_node, x1, system_width, cresc);
    }

    // ── articulation marks ──
    for a in anchors {
        for i in 0..a.m.entries.len() {
            if a.m.arts[i].is_empty() {
                continue;
            }
            let Some((primary_line, _)) = a.m.pitches[i] else {
                continue;
            };
            let ex = a.entry_x.get(i).copied().unwrap_or(0.0);
            // Stems point down for notes on/above the middle line, so the
            // mark goes above the notehead (and vice versa).
            let stem_up = primary_line < 0;
            let top_line = a
                .m
                .stacks[i]
                .iter()
                .map(|(l, _)| *l)
                .chain(std::iter::once(primary_line))
                .max()
                .unwrap_or(primary_line);
            let bottom_line = a
                .m
                .stacks[i]
                .iter()
                .map(|(l, _)| *l)
                .chain(std::iter::once(primary_line))
                .min()
                .unwrap_or(primary_line);
            let mut stack_offset = 0.0;
            for tag in &a.m.arts[i] {
                let Some(art) = articulation_type(tag) else {
                    continue;
                };
                let above = !stem_up;
                let (glyph, y) = if above {
                    let y_note = 2.0 * spatium - f64::from(top_line) * spatium / 2.0;
                    (
                        art.smufl_codepoint_above(),
                        y_note - 1.1 * spatium - stack_offset,
                    )
                } else {
                    let y_note = 2.0 * spatium - f64::from(bottom_line) * spatium / 2.0;
                    (
                        art.smufl_codepoint_below(),
                        y_note + 1.4 * spatium + stack_offset,
                    )
                };
                stack_offset += 1.1 * spatium;
                // PaintCommand::glyph size is in SPATIUMS (the SVG serializer
                // multiplies by 4 for the SMuFL em).
                sys_node.add_child(SceneNode::anonymous_leaf(vec![PaintCommand::glyph(
                    glyph,
                    Point::new(ex + head_w / 2.0 - 0.4 * spatium, y),
                    spatium,
                    Color::BLACK,
                )]));
            }
        }
    }
}

/// Draw ties and slurs over a laid-out system.
///
/// Anchor conventions follow MuseScore's `SlurTieLayout::adjustX` (clone at
/// ~/reference/MuseScore, src/engraving/rendering/score/slurtielayout.cpp):
/// tie start clears the notehead + ~0.2sp padding, end stops the same
/// padding before the target head; direction is away from the stem (single
/// voice: notes on/above the middle line stem down → curve up). Ties that
/// leave the system become half-ties to the line end; incoming stops with
/// no pending start become half-ties from the line start.
fn draw_ties_and_slurs(
    sys_node: &mut SceneNode,
    anchors: &[MeasureAnchors<'_>],
    system_width: f64,
    spatium: f64,
    id: &mut u64,
) {
    use engraver_proto::engraver::layout::tlayout::{
        SlurDirection, SlurEndpoint, SlurTieConfig, layout_slur, layout_tie,
    };
    let cfg = SlurTieConfig::default();
    let y_of = |line: i32| 2.0 * spatium - f64::from(line) * spatium / 2.0;
    let head_w = 1.18 * spatium;
    let pad = 0.2 * spatium;

    let mut draw_tie = |sys_node: &mut SceneNode, line: i32, x1: f64, x2: f64| {
        let dir = if line >= 0 {
            SlurDirection::Up
        } else {
            SlurDirection::Down
        };
        let start = SlurEndpoint {
            x: x1,
            y: y_of(line),
            stem_up: line < 0,
        };
        let end = SlurEndpoint {
            x: x2.max(x1 + spatium),
            y: y_of(line),
            stem_up: line < 0,
        };
        let tie = layout_tie(&start, &end, dir, *id, spatium, &cfg);
        *id += 1;
        sys_node.add_child(tie.scene);
    };

    // ── ties ──
    let mut pending: Vec<(i32, f64)> = Vec::new();
    for a in anchors {
        for i in 0..a.m.entries.len() {
            let ex = a.entry_x.get(i).copied().unwrap_or(0.0);
            for line in &a.m.tie_stop_lines[i] {
                if let Some(pos) = pending.iter().position(|(l, _)| l == line) {
                    let (l, sx) = pending.remove(pos);
                    draw_tie(sys_node, l, sx + head_w + pad, ex - pad);
                } else {
                    // Tie arriving from the previous system: half tie in.
                    draw_tie(sys_node, *line, (ex - 3.0 * spatium).max(0.0), ex - pad);
                }
            }
            for line in &a.m.tie_start_lines[i] {
                pending.push((*line, ex));
            }
        }
    }
    // Ties leaving the system: half tie out to the line end.
    for (line, sx) in pending {
        let x1 = sx + head_w + pad;
        draw_tie(sys_node, line, x1, (x1 + 3.0 * spatium).min(system_width));
    }

    // ── slurs ──
    let mut open: Option<(f64, i32)> = None;
    for a in anchors {
        for i in 0..a.m.entries.len() {
            let ex = a.entry_x.get(i).copied().unwrap_or(0.0);
            let line = a.m.pitches[i].map(|(l, _)| l).unwrap_or(0);
            if a.m.slur_stops[i]
                && let Some((sx, sl)) = open.take()
            {
                // Slur above; endpoints just over the noteheads.
                let start = SlurEndpoint {
                    x: sx + head_w / 2.0,
                    y: y_of(sl) - 0.8 * spatium,
                    stem_up: false,
                };
                let end = SlurEndpoint {
                    x: ex + head_w / 2.0,
                    y: y_of(line) - 0.8 * spatium,
                    stem_up: false,
                };
                if end.x > start.x + spatium {
                    let slur = layout_slur(&start, &end, SlurDirection::Up, *id, spatium, &cfg);
                    *id += 1;
                    sys_node.add_child(slur.scene);
                }
            }
            if a.m.slur_starts[i] && open.is_none() {
                open = Some((ex, line));
            }
        }
    }
}

/// Draw a multi-measure rest block: serifs + H-bar on the middle line and
/// the count in SMuFL time-signature digits above the staff.
fn multirest_scene(count: usize, width: f64, spatium: f64, id: u64) -> SceneNode {
    let mut node = SceneNode::group(SemanticId::new(ElementType::Measure, id));
    let mid_y = 2.0 * spatium;
    let pad = 1.6 * spatium;
    let bar_h = spatium * 1.0;
    let mut cmds = Vec::new();
    // H-bar body
    cmds.push(PaintCommand::Rect {
        rect: Rect::new(pad, mid_y - bar_h / 2.0, width - pad, mid_y + bar_h / 2.0),
        fill: Some(Color::BLACK),
        stroke: None,
        stroke_width: 0.0,
        corner_radius: None,
    });
    // End serifs
    for x in [pad, width - pad] {
        cmds.push(PaintCommand::Line {
            start: Point::new(x, mid_y - spatium),
            end: Point::new(x, mid_y + spatium),
            width: spatium * 0.16,
            color: Color::BLACK,
            line_cap: Default::default(),
        });
    }
    // Count in SMuFL timeSig digits, centered above the staff.
    let digits: String = count
        .to_string()
        .chars()
        .filter_map(|c| c.to_digit(10))
        .map(|d| char::from_u32(0xE080 + d).unwrap_or('0'))
        .collect();
    cmds.push(PaintCommand::Text {
        text: digits,
        font_family: "Leland".to_string(),
        font_size: spatium * 4.0,
        position: Point::new(width / 2.0, -1.2 * spatium),
        color: Color::BLACK,
        anchor: TextAnchor::Middle,
        weight: FontWeight::Normal,
        style: FontStyle::Normal,
    });
    // Closing barline
    cmds.push(PaintCommand::Line {
        start: Point::new(width, 0.0),
        end: Point::new(width, 4.0 * spatium),
        width: spatium * 0.16,
        color: Color::BLACK,
        line_cap: Default::default(),
    });
    node.add_child(SceneNode::anonymous_leaf(cmds));
    node
}

fn staff_lines(width: f64, spatium: f64) -> SceneNode {
    let mut cmds = Vec::new();
    for i in 0..5 {
        let y = f64::from(i) * spatium;
        cmds.push(PaintCommand::Line {
            start: Point::new(0.0, y),
            end: Point::new(width, y),
            width: spatium * 0.1,
            color: Color::BLACK,
            line_cap: Default::default(),
        });
    }
    SceneNode::anonymous_leaf(cmds)
}

fn text(
    s: &str,
    family: &str,
    size: f64,
    pos: Point,
    anchor: TextAnchor,
    weight: FontWeight,
) -> SceneNode {
    SceneNode::anonymous_leaf(vec![PaintCommand::Text {
        text: s.to_string(),
        font_family: family.to_string(),
        font_size: size,
        position: pos,
        color: Color::BLACK,
        anchor,
        weight,
        style: FontStyle::Normal,
    }])
}

/// Lay out one part into pages.
#[must_use]
pub fn layout_part(score: &Score, part_index: usize, opts: &LayoutOptions) -> PartLayout {
    let part = &score.parts[part_index];
    let style = engraver_proto::api::style::leak_lead_sheet_style();
    let ctx = LayoutContextOwned::new_minimal_arc(std::sync::Arc::new(style.clone()));
    let spatium = opts.spatium;

    let mut dropped = 0usize;
    let prepared = prepare_measures(part, opts, &mut dropped);
    if dropped > 0 {
        tracing::info!(part = part.name, dropped, "non-primary-voice events dropped (P2 single-voice)");
    }

    let avail_width = opts.page_width - 2.0 * opts.margin;
    let mut root = SceneNode::group(SemanticId::new(ElementType::System, 0));
    let mut pages = Vec::new();

    // ── measure metrics: min width (compact build) + content weight ──
    //
    // Mirrors the chart engine's spacing model (width_dist.rs): weights come
    // from rhythmic density via `chart_spacing::duration_stretch`, minimum
    // widths from a compact pre-build, and the per-system distribution is the
    // same spring physics (`measure_layout::distribute_measure_widths_spring`).
    const SLOPE: f64 = chart_spacing::DEFAULT_SPACING_SLOPE;
    const DENSITY: f64 = chart_spacing::DEFAULT_SPACING_DENSITY;

    let measure_ticks_of = |time: (u32, u32)| -> f64 {
        f64::from(time.0) * 4.0 * constants::TICKS_PER_QUARTER / f64::from(time.1.max(1))
    };

    struct MeasureMetrics {
        min_width: f64,
        weight: f64,
    }
    let metrics: Vec<MeasureMetrics> = prepared
        .iter()
        .map(|item| match item {
            Prepared::Notated(m) => {
                let min_width = build_measure(m, &ctx, None, false, false, 1).width;
                let measure_ticks = measure_ticks_of(m.time);
                let note_ticks: Vec<f64> = m
                    .entries
                    .iter()
                    .filter_map(|e| match e {
                        RhythmEntry::Note(d) => Some(f64::from(d.ticks())),
                        RhythmEntry::Rest(_) => None,
                    })
                    .collect();
                let shortest = note_ticks.iter().copied().fold(f64::INFINITY, f64::min);
                let dense =
                    note_ticks.len() >= 4 || shortest <= constants::TICKS_PER_QUARTER / 2.0;
                let duration_weight = if note_ticks.is_empty() || !dense {
                    chart_spacing::duration_stretch(
                        measure_ticks,
                        constants::TICKS_PER_QUARTER,
                        SLOPE,
                    )
                } else {
                    note_ticks
                        .iter()
                        .map(|t| {
                            chart_spacing::duration_stretch(
                                *t,
                                constants::TICKS_PER_QUARTER,
                                SLOPE,
                            )
                        })
                        .sum()
                };
                let tuplet_bonus: f64 = m
                    .tuplets
                    .iter()
                    .map(|(start, end, _)| (end - start) as f64 * 0.08)
                    .sum();
                MeasureMetrics {
                    min_width,
                    weight: (duration_weight + tuplet_bonus).max(0.5),
                }
            }
            Prepared::MultiRest { time, .. } => MeasureMetrics {
                min_width: MULTIREST_WIDTH_SP * spatium,
                weight: chart_spacing::duration_stretch(
                    measure_ticks_of(*time),
                    constants::TICKS_PER_QUARTER,
                    SLOPE,
                ),
            },
        })
        .collect();

    // ── system packing: min-width constrained, capped per system ──
    const MAX_MEASURES_PER_SYSTEM: usize = 6;
    struct SystemPlan {
        items: Vec<usize>, // indices into `prepared`
    }
    let mut systems: Vec<SystemPlan> = Vec::new();
    {
        let mut current = SystemPlan { items: Vec::new() };
        let mut used = 0.0;
        for idx in 0..prepared.len() {
            let key_fifths = match &prepared[idx] {
                Prepared::Notated(m) => m.key_fifths,
                Prepared::MultiRest { key_fifths, .. } => *key_fifths,
            };
            let (_, _, _, prefix_w) =
                calculate_prefix_width(spatium, true, true, key_fifths, current.items.is_empty());
            let w = metrics[idx].min_width;
            let full = current.items.len() >= MAX_MEASURES_PER_SYSTEM
                || (!current.items.is_empty() && used + w > avail_width - prefix_w);
            if full {
                systems.push(current);
                current = SystemPlan { items: Vec::new() };
                used = 0.0;
            }
            current.items.push(idx);
            used += w;
        }
        if !current.items.is_empty() {
            systems.push(current);
        }
    }

    // Expansion gating, verbatim from the chart engine's width_dist.rs
    // (pub(super) there): only stretch past base width when a measure's
    // rhythm or minimum actually demands it.
    let expansion_stretches = |weights: &[f64], min_widths: &[f64], base: f64| -> Vec<f64> {
        const RHYTHM_EXPANSION_THRESHOLD: f64 = 3.0;
        let has_expander = weights.iter().zip(min_widths).any(|(w, m)| {
            *w >= RHYTHM_EXPANSION_THRESHOLD || *m > base * 1.05
        });
        if !has_expander {
            return vec![1.0; weights.len()];
        }
        weights
            .iter()
            .zip(min_widths)
            .map(|(weight, min_width)| {
                if *weight >= RHYTHM_EXPANSION_THRESHOLD || *min_width > base * 1.05 {
                    weight.max(min_width / base.max(1.0)).max(1.0).sqrt()
                } else {
                    1.0
                }
            })
            .collect()
    };

    // ── render systems onto pages ──
    let mut id: u64 = 10;
    let mut page_index = 0usize;
    let mut y = opts.margin + opts.title_block_height;
    let mut page_origin_y = 0.0;

    let new_page = |pages: &mut Vec<PageRect>, page_index: &mut usize| -> f64 {
        let y_off = *page_index as f64 * (opts.page_height + 24.0);
        pages.push(PageRect {
            x_offset: 0.0,
            y_offset: y_off,
            width: opts.page_width,
            height: opts.page_height,
        });
        *page_index += 1;
        y_off
    };
    page_origin_y = new_page(&mut pages, &mut page_index);

    // Title block (page 1).
    {
        let title = score
            .work_title
            .clone()
            .or_else(|| score.movement_title.clone())
            .unwrap_or_default();
        let mut block = SceneNode::group(SemanticId::new(ElementType::Text, 1));
        block.add_child(text(
            &part.name,
            "FreeSans",
            12.0,
            Point::new(opts.margin, page_origin_y + opts.margin + 4.0),
            TextAnchor::Start,
            FontWeight::Bold,
        ));
        block.add_child(text(
            &title,
            "Chicago",
            26.0,
            Point::new(opts.page_width / 2.0, page_origin_y + opts.margin + 24.0),
            TextAnchor::Middle,
            FontWeight::Bold,
        ));
        if let Some(composer) = score.composer.clone().or_else(|| score.arranger.clone()) {
            block.add_child(text(
                &composer,
                "FreeSans",
                10.0,
                Point::new(opts.page_width - opts.margin, page_origin_y + opts.margin + 44.0),
                TextAnchor::End,
                FontWeight::Normal,
            ));
        }
        root.add_child(block);
    }

    let system_height = 4.0 * spatium;
    for (sys_idx, sys) in systems.iter().enumerate() {
        if y + system_height > opts.page_height - opts.margin {
            page_origin_y = new_page(&mut pages, &mut page_index);
            y = opts.margin + 30.0;
        }
        let staff_y = page_origin_y + y;

        let mut sys_node = SceneNode::group(SemanticId::new(ElementType::System, sys_idx as u64 + 1));
        sys_node.transform = Affine::translate((opts.margin, staff_y));

        // Prefix from the first item's attribute state.
        let (first_clef, first_key, first_time, show_time) = match &prepared[sys.items[0]] {
            Prepared::Notated(m) => (m.clef, m.key_fifths, m.time, sys_idx == 0 || m.time_changed),
            Prepared::MultiRest {
                clef,
                key_fifths,
                time,
                ..
            } => (*clef, *key_fifths, *time, sys_idx == 0),
        };
        let (clef_w, key_w, time_w, _) =
            calculate_prefix_width(spatium, true, true, first_key, show_time);
        let prefix = render_system_prefix(
            &PrefixRenderContext {
                x: 0.0,
                staff_y: 0.0,
                spatium,
                include_clef: true,
                clef_type: first_clef,
                include_key_sig: true,
                include_time_sig: show_time,
                key_signature: first_key,
                key_sig_color: None,
                time_signature: (first_time.0 as u8, first_time.1 as u8),
                clef_width: clef_w,
                key_sig_width: key_w,
                time_sig_width: time_w,
                page_number: None,
            },
            id,
            &ctx.as_context(),
        );
        id = prefix.next_id + 100;
        let prefix_w = prefix.total_width;

        // Spring-based width distribution (same engine as chart layout).
        let weights: Vec<f64> = sys.items.iter().map(|&i| metrics[i].weight).collect();
        let min_widths: Vec<f64> = sys.items.iter().map(|&i| metrics[i].min_width).collect();
        let measures_area = avail_width - prefix_w;
        let base_measure_width = measures_area / MAX_MEASURES_PER_SYSTEM as f64;
        let is_last_system = sys_idx + 1 == systems.len();
        let min_sum: f64 = min_widths.iter().sum();
        // Every system justifies to the full line except a sparse final one.
        let total_to_distribute = if is_last_system
            && sys.items.len() < MAX_MEASURES_PER_SYSTEM
            && min_sum < measures_area * 0.7
        {
            (sys.items.len() as f64 * base_measure_width).max(min_sum)
        } else {
            measures_area
        };
        let stretches = expansion_stretches(&weights, &min_widths, base_measure_width);
        let widths = measure_layout::distribute_measure_widths_spring(
            &stretches,
            0,
            total_to_distribute,
            0.4,
            base_measure_width,
            &min_widths,
            spatium,
            SLOPE,
            DENSITY,
        );

        let content_width: f64 = widths.iter().sum();
        sys_node.add_child(staff_lines(prefix_w + content_width, spatium));

        for node in prefix.nodes {
            sys_node.add_child(node);
        }

        let mut anchors: Vec<MeasureAnchors<'_>> = Vec::new();

        let mut x = prefix_w;
        for (item_pos, &item_idx) in sys.items.iter().enumerate() {
            let width = widths[item_pos];
            match &prepared[item_idx] {
                Prepared::Notated(m) => {
                    let scene = build_measure(
                        m,
                        &ctx,
                        Some(width / spatium),
                        m.clef_changed && item_pos > 0,
                        m.time_changed && !(item_pos == 0 && show_time),
                        id,
                    );
                    id += 1000;
                    let entry_x = engraver_proto::engraver::layout::chart::chord_layout::get_chord_rest_positions(&scene)
                        .into_iter()
                        .map(|seg_x| x + seg_x)
                        .collect();
                    anchors.push(MeasureAnchors { m, entry_x });
                    let mut container =
                        SceneNode::group(SemanticId::new(ElementType::Measure, item_idx as u64 + 1));
                    // MeasureScene notation is authored around the MIDDLE
                    // line (y=0) — same +2sp shift the chart pipeline applies
                    // to its measure containers.
                    container.transform = Affine::translate((x, 2.0 * spatium));
                    container.add_child(scene.scene.clone());
                    // Measure number over the first measure of each system.
                    if item_pos == 0 {
                        container.add_child(text(
                            &m.number,
                            "FreeSans",
                            8.0,
                            Point::new(0.0, -4.6 * spatium),
                            TextAnchor::Start,
                            FontWeight::Normal,
                        ));
                    }
                    sys_node.add_child(container);
                    // Advance by the ALLOCATED width (chart does the same):
                    // any spring-justification shortfall stays inside this
                    // measure's slot instead of accumulating as a gap at the
                    // end of the line.
                    x += width;
                    // Builder barlines are suppressed (mixed y-convention);
                    // draw the measure's closing barline in staff coords.
                    sys_node.add_child(barline(x, spatium));
                }
                Prepared::MultiRest { count, number, .. } => {
                    let mut container =
                        SceneNode::group(SemanticId::new(ElementType::Measure, item_idx as u64 + 1));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(multirest_scene(*count, width, spatium, id));
                    id += 10;
                    if item_pos == 0 {
                        container.add_child(text(
                            number,
                            "FreeSans",
                            8.0,
                            Point::new(0.0, -2.6 * spatium),
                            TextAnchor::Start,
                            FontWeight::Normal,
                        ));
                    }
                    sys_node.add_child(container);
                    x += width;
                }
            }
        }

        draw_ties_and_slurs(&mut sys_node, &anchors, prefix_w + content_width, spatium, &mut id);
        draw_directions(&mut sys_node, &anchors, &ctx, prefix_w + content_width, spatium, &mut id);

        root.add_child(sys_node);
        y += opts.system_gap;
    }

    PartLayout {
        scene: root,
        pages,
        dropped_voice_events: dropped,
    }
}
