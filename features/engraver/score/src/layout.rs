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
    /// Clef change taking effect at this measure (drawn in-measure when
    /// mid-system, absorbed by the prefix at system start).
    clef: ClefType,
    clef_changed: bool,
    key_fifths: i8,
    key_changed: bool,
    time: (u32, u32),
    time_changed: bool,
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
                out.push(Prepared::Notated(Box::new(NotatedMeasure {
                    number: number.clone(),
                    entries: vec![RhythmEntry::Rest(Duration::Whole)],
                    pitches: vec![None],
                    stacks: vec![Vec::new()],
                    tuplets: Vec::new(),
                    clef: *clef,
                    clef_changed: false,
                    key_fifths: *key_fifths,
                    key_changed: false,
                    time: *time,
                    time_changed: false,
                })));
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
            prepared.push(Prepared::Notated(Box::new(NotatedMeasure {
                number: measure.number.clone(),
                entries: vec![RhythmEntry::Rest(Duration::Whole)],
                pitches: vec![None],
                stacks: vec![Vec::new()],
                tuplets: Vec::new(),
                clef: state.clef,
                clef_changed,
                key_fifths: state.key_fifths,
                key_changed,
                time: state.time,
                time_changed,
            })));
            continue;
        };

        let octave_change = 0i8; // per-measure clef octave marks: P3
        let middle = middle_line_ref(state.clef, octave_change);
        let mut acc_state = AccidentalState::default();
        let mut entries = Vec::new();
        let mut pitches = Vec::new();
        let mut stacks = Vec::new();
        let mut tuplet_marks: Vec<Option<TupletRatio>> = Vec::new();

        let mut events: Vec<&Event> = measure
            .events
            .iter()
            .filter(|e| {
                let keep = e.voice() == voice
                    && match e {
                        Event::Chord(c) => c.staff == 1 && !c.grace,
                        Event::Rest(r) => r.staff == 1,
                    };
                if !keep {
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

                    let mut lines: Vec<(i32, Accidental)> = chord
                        .notes
                        .iter()
                        .map(|n| {
                            let p = Pitch::with_alteration(
                                pitch_class(n.pitch.step),
                                Octave::new(n.pitch.octave),
                                n.pitch.alter,
                            );
                            let line = p.staff_position() - middle;
                            let acc = acc_state.resolve(&n.pitch, state.key_fifths);
                            (line, acc)
                        })
                        .collect();
                    // Primary head = highest note; extras stack below it.
                    lines.sort_by_key(|(line, _)| -line);
                    let primary = lines.remove(0);
                    pitches.push(Some(primary));
                    stacks.push(lines);
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
                }
            }
        }

        if entries.is_empty() {
            entries.push(RhythmEntry::Rest(Duration::Whole));
            pitches.push(None);
            stacks.push(Vec::new());
            tuplet_marks.push(None);
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
                    run_start = if let Some(m) = mark { Some((idx, *m)) } else { None };
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
    builder.build(&ctx.as_context())
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

    // ── greedy system packing on natural (compact) widths ──
    struct SystemPlan {
        items: Vec<usize>, // indices into `prepared`
    }
    let natural_width = |idx: usize| -> f64 {
        match &prepared[idx] {
            Prepared::Notated(m) => {
                build_measure(m, &ctx, None, false, false, 1).width
            }
            Prepared::MultiRest { .. } => MULTIREST_WIDTH_SP * spatium,
        }
    };

    let mut systems: Vec<SystemPlan> = Vec::new();
    {
        let mut current = SystemPlan { items: Vec::new() };
        let mut used = 0.0;
        for idx in 0..prepared.len() {
            // Reserve prefix room (clef + key) on every system.
            let key_fifths = match &prepared[idx] {
                Prepared::Notated(m) => m.key_fifths,
                Prepared::MultiRest { key_fifths, .. } => *key_fifths,
            };
            let (_, _, _, prefix_w) =
                calculate_prefix_width(spatium, true, true, key_fifths, current.items.is_empty());
            let w = natural_width(idx);
            if !current.items.is_empty() && used + w > avail_width - prefix_w {
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

        // Distribute the remaining width proportionally to natural widths.
        let naturals: Vec<f64> = sys.items.iter().map(|&i| natural_width(i)).collect();
        let natural_sum: f64 = naturals.iter().sum();
        let target_total = avail_width - prefix_w;
        let is_last_system = sys_idx + 1 == systems.len();
        // Don't stretch a sparse final system across the whole page.
        let scale = if is_last_system && natural_sum < target_total * 0.7 {
            1.0
        } else {
            target_total / natural_sum.max(1.0)
        };

        let content_width = natural_sum * scale;
        sys_node.add_child(staff_lines(prefix_w + content_width, spatium));

        for node in prefix.nodes {
            sys_node.add_child(node);
        }

        let mut x = prefix_w;
        for (item_pos, &item_idx) in sys.items.iter().enumerate() {
            let width = naturals[item_pos] * scale;
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
                    let mut container =
                        SceneNode::group(SemanticId::new(ElementType::Measure, item_idx as u64 + 1));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(scene.scene.clone());
                    // Measure number over the first measure of each system.
                    if item_pos == 0 {
                        container.add_child(text(
                            &m.number,
                            "FreeSans",
                            8.0,
                            Point::new(0.0, -2.6 * spatium),
                            TextAnchor::Start,
                            FontWeight::Normal,
                        ));
                    }
                    sys_node.add_child(container);
                    x += scene.width;
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

        root.add_child(sys_node);
        y += opts.system_gap;
    }

    PartLayout {
        scene: root,
        pages,
        dropped_voice_events: dropped,
    }
}
