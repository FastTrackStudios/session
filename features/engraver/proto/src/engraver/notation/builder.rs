//! Builders for automatic music notation layout.

use kurbo::{Affine, Point};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::segment::{Segment, SegmentType};
use crate::engraver::layout::segment_list::SegmentList;
use crate::engraver::layout::spacing::HorizontalSpacing;
use crate::engraver::layout::tlayout::{
    Accidental, BarlineParams, BarlineType, BeamLayoutConfig, BeamNote, ChordNote, ChordParams,
    ClefParams, ClefType, NoteHeadType, NoteParams, RestDuration, RestParams, StemDirection,
    TimeSigParams, TimeSigType, TupletConfig, TupletNote, TupletRatio, layout_barline, layout_beam,
    layout_chord, layout_clef, layout_note, layout_rest, layout_timesig, layout_tuplet,
};
use crate::engraver::quantize::{QuantizeConfig, QuantizedDuration, detect_tuplet_groups};
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::mode::NotationMode;
use super::{Duration, DurationKind, TimeSignature};

/// A rhythm entry representing either a note/chord or a rest.
///
/// This follows MuseScore's ChordRest pattern where both chords and rests
/// share common properties like duration, tuplet membership, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhythmEntry {
    /// A note or chord with the given duration
    Note(Duration),
    /// A rest with the given duration
    Rest(Duration),
}

impl RhythmEntry {
    /// Create a note entry.
    #[must_use]
    pub const fn note(duration: Duration) -> Self {
        Self::Note(duration)
    }

    /// Create a rest entry.
    #[must_use]
    pub const fn rest(duration: Duration) -> Self {
        Self::Rest(duration)
    }

    /// Get the duration of this entry.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        match self {
            Self::Note(d) | Self::Rest(d) => *d,
        }
    }

    /// Check if this entry is a rest.
    #[must_use]
    pub const fn is_rest(&self) -> bool {
        matches!(self, Self::Rest(_))
    }

    /// Check if this entry is a note.
    #[must_use]
    pub const fn is_note(&self) -> bool {
        matches!(self, Self::Note(_))
    }

    /// Get ticks for this entry (delegates to duration).
    #[must_use]
    pub const fn ticks(&self) -> i32 {
        self.duration().ticks()
    }
}

/// Specification for a tuplet group within a measure.
#[derive(Debug, Clone)]
pub struct TupletSpec {
    /// Start index in the rhythm array (inclusive)
    pub start_idx: usize,
    /// End index in the rhythm array (exclusive)
    pub end_idx: usize,
    /// The tuplet ratio (e.g., 3:2 for triplet)
    pub ratio: TupletRatio,
}

impl TupletSpec {
    /// Create a new tuplet specification.
    pub fn new(start_idx: usize, end_idx: usize, ratio: TupletRatio) -> Self {
        Self {
            start_idx,
            end_idx,
            ratio,
        }
    }

    /// Create a triplet (3:2) specification.
    pub fn triplet(start_idx: usize, end_idx: usize) -> Self {
        Self::new(start_idx, end_idx, TupletRatio::triplet())
    }
}

/// Builder for creating a single measure of music with automatic spacing.
#[derive(Debug, Clone)]
pub struct MeasureBuilder {
    /// Clef type (None = no clef metadata at all)
    clef: Option<ClefType>,
    /// Whether to render the clef glyph (metadata still used for layout)
    show_clef: bool,
    /// Time signature (None = unknown; defaults to 4/4 for beam grouping)
    time_signature: Option<TimeSignature>,
    /// Whether to render the time-signature glyph
    show_time_signature: bool,
    /// Optional time-signature glyph color. `None` = default black. A mid-chart
    /// meter change passes red so it stands out from the prevailing meter.
    time_signature_color: Option<Color>,
    /// Notation mode (Standard, Rhythmic, etc.)
    mode: NotationMode,
    /// Rhythm pattern (list of durations)
    rhythm: Vec<Duration>,
    /// Positions that are rests (indices into rhythm array)
    rest_positions: Vec<bool>,
    /// Per-note head type overrides (index -> head type).
    /// When set, overrides the mode's default head type for specific notes.
    head_type_overrides: Vec<Option<NoteHeadType>>,
    /// Starting barline type
    start_barline: Option<BarlineType>,
    /// Ending barline type
    end_barline: Option<BarlineType>,
    /// Staff width in spatiums (for justification)
    width: Option<f64>,
    /// Whether to justify (stretch to fill width)
    justify: bool,
    /// Unique ID base for elements
    id_base: u64,
    /// Whether notes should be stemless
    stemless: bool,
    /// Tuplet group specifications
    tuplet_groups: Vec<TupletSpec>,
    /// Compact mode: use minimal left margin for tight spaces (count-in, etc.)
    compact: bool,
    /// Minimum widths for chord/rest segments (e.g., from chord symbol widths).
    /// If provided, these are applied as minimum widths per segment index.
    segment_min_widths: Vec<f64>,
    /// Per-note pitch information for melody rendering.
    /// When set, overrides the default note_line with per-note staff positions and accidentals.
    /// Parallel to rhythm entries — Some((line, accidental)) for pitched notes, None for default.
    note_pitches: Vec<Option<(i32, Accidental)>>,
    /// Extra pitches stacked on each rhythm entry's stem — same length as
    /// `note_pitches` when set, each entry holding the secondary heads
    /// (octave doublings, double-stops). All share the primary's stem.
    note_pitch_stacks: Vec<Vec<(i32, Accidental)>>,
}

impl Default for MeasureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MeasureBuilder {
    /// Create a new measure builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clef: None,
            show_clef: true,
            time_signature: None,
            show_time_signature: true,
            time_signature_color: None,
            mode: NotationMode::Standard,
            rhythm: Vec::new(),
            rest_positions: Vec::new(),
            head_type_overrides: Vec::new(),
            start_barline: None,
            end_barline: Some(BarlineType::Single),
            width: None,
            justify: false,
            id_base: 1,
            stemless: false,
            tuplet_groups: Vec::new(),
            compact: false,
            segment_min_widths: Vec::new(),
            note_pitches: Vec::new(),
            note_pitch_stacks: Vec::new(),
        }
    }

    /// Set compact mode for minimal left margin (useful for count-in measures).
    #[must_use]
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Set minimum widths for chord/rest segments.
    ///
    /// This allows passing chord symbol widths as minimum width constraints,
    /// ensuring the spacing system gives enough room for each chord symbol.
    /// The widths are indexed by chord/rest segment index (not tick position).
    #[must_use]
    pub fn segment_min_widths(mut self, widths: Vec<f64>) -> Self {
        self.segment_min_widths = widths;
        self
    }

    /// Set per-note pitch information for melody rendering.
    ///
    /// Each entry corresponds to a rhythm entry. `Some((line, accidental))` overrides
    /// the default note line; `None` uses the mode's default line.
    #[must_use]
    pub fn note_pitches(mut self, pitches: Vec<Option<(i32, Accidental)>>) -> Self {
        self.note_pitches = pitches;
        self
    }

    /// Per-rhythm-entry stack of extra noteheads (octave doublings, etc.)
    /// that share the primary head's stem. Same length as `note_pitches`.
    #[must_use]
    pub fn note_pitch_stacks(mut self, stacks: Vec<Vec<(i32, Accidental)>>) -> Self {
        self.note_pitch_stacks = stacks;
        self
    }

    /// Set the clef.
    #[must_use]
    pub fn clef(mut self, clef: ClefType) -> Self {
        self.clef = Some(clef);
        self
    }

    /// Set the time signature.
    #[must_use]
    pub fn time_signature(mut self, numerator: u8, denominator: u8) -> Self {
        self.time_signature = Some(TimeSignature::new(numerator, denominator));
        self
    }

    /// Set the time-signature glyph color (e.g. red for a mid-chart change).
    #[must_use]
    pub fn time_signature_color(mut self, color: Color) -> Self {
        self.time_signature_color = Some(color);
        self
    }

    /// Set time-signature metadata without rendering the glyph (still used for beam grouping).
    #[must_use]
    pub fn time_signature_meta(mut self, ts: TimeSignature) -> Self {
        self.time_signature = Some(ts);
        self.show_time_signature = false;
        self
    }

    /// Set clef metadata without rendering the glyph.
    #[must_use]
    pub fn clef_meta(mut self, clef: ClefType) -> Self {
        self.clef = Some(clef);
        self.show_clef = false;
        self
    }

    /// Set the time signature from a TimeSignature struct.
    #[must_use]
    pub fn time_sig(mut self, ts: TimeSignature) -> Self {
        self.time_signature = Some(ts);
        self
    }

    /// Set the notation mode.
    #[must_use]
    pub fn mode(mut self, mode: NotationMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set to rhythmic (slash) notation mode.
    #[must_use]
    pub fn rhythmic(mut self) -> Self {
        self.mode = NotationMode::Rhythmic;
        self
    }

    /// Set notes to be stemless (noteheads only, no stems).
    #[must_use]
    pub fn stemless(mut self) -> Self {
        self.stemless = true;
        self
    }

    /// Enable automatic stemless detection for rhythmic notation.
    ///
    /// In rhythmic/slash notation, consecutive quarter notes (2 or more)
    /// should be displayed without stems for cleaner chord chart appearance.
    /// This follows the LilyPond convention where groups of identical
    /// quarter note slashes are stemless.
    #[must_use]
    pub fn auto_stemless(mut self) -> Self {
        // Only applies to rhythmic mode
        if matches!(self.mode, NotationMode::Rhythmic) {
            self.stemless = false; // Will be computed per-note
        }
        self
    }

    /// Add a tuplet group specification.
    ///
    /// Tuplet groups define which notes form a tuplet (triplet, quintuplet, etc.)
    /// and how they should be rendered with a bracket and number.
    ///
    /// # Arguments
    /// * `start_idx` - Start index in rhythm array (inclusive)
    /// * `end_idx` - End index in rhythm array (exclusive)
    /// * `ratio` - The tuplet ratio (e.g., 3:2 for triplet)
    ///
    /// # Example
    /// ```ignore
    /// // Create a measure with a triplet on notes 1-3
    /// builder.rhythm(vec![Quarter, Eighth, Eighth, Eighth, Quarter])
    ///        .tuplet_group(1, 4, TupletRatio::triplet())
    /// ```
    #[must_use]
    pub fn tuplet_group(mut self, start_idx: usize, end_idx: usize, ratio: TupletRatio) -> Self {
        self.tuplet_groups
            .push(TupletSpec::new(start_idx, end_idx, ratio));
        self
    }

    /// Add a triplet group (3:2 ratio) for the specified note range.
    #[must_use]
    pub fn triplet(mut self, start_idx: usize, end_idx: usize) -> Self {
        self.tuplet_groups
            .push(TupletSpec::triplet(start_idx, end_idx));
        self
    }

    /// Compute which notes should be stemless based on consecutive quarter note analysis.
    ///
    /// Returns a Vec<bool> where true means the note at that index should be stemless.
    ///
    /// The algorithm (based on LilyPond convention):
    /// 1. Consecutive quarter notes (no dots) are candidates for stemless
    /// 2. Non-quarter notes break the consecutive chain
    /// 3. Strong beats also break the chain (in 4/4, beats 1 and 3 are strong)
    /// 4. Groups of 2+ consecutive quarters within the same beat-group = stemless
    ///
    /// For example in 4/4 with quarters on beats 2, 3, 4:
    /// - Beat 2 is alone before the strong beat 3 -> has stem
    /// - Beats 3-4 are consecutive after strong beat 3 -> stemless
    fn compute_auto_stemless(&self) -> Vec<bool> {
        let mut result = vec![false; self.rhythm.len()];

        // Only apply in rhythmic mode
        if !matches!(self.mode, NotationMode::Rhythmic) {
            return result;
        }

        // Build a set of indices that are part of tuplet groups
        // These notes need stems for bracket attachment
        let mut in_tuplet: Vec<bool> = vec![false; self.rhythm.len()];
        for spec in &self.tuplet_groups {
            for i in spec.start_idx..spec.end_idx {
                if i < in_tuplet.len() {
                    in_tuplet[i] = true;
                }
            }
        }

        // Auto-stemless logic:
        // 1. Whole notes and half notes → stemless (rhythm slashes for longer durations)
        // 2. Plain quarter notes NOT in tuplet groups → stemless
        // 3. Notes in tuplet groups → stemmed (need stems for bracket attachment)
        // 4. Eighth notes and shorter → stemmed (specific rhythms need visual clarity)
        for (i, dur) in self.rhythm.iter().enumerate() {
            // Skip tuplet notes - they need stems for bracket attachment
            if in_tuplet[i] {
                result[i] = false;
                continue;
            }

            let is_long_duration = matches!(dur.kind, DurationKind::Whole | DurationKind::Half);
            // Quarter slashes — plain or dotted — are filler rhythm marks
            // (e.g. compound-meter `/. /.`); they get no stems unless the
            // user supplied an explicit chord rhythm.
            let is_quarter_slash = matches!(dur.kind, DurationKind::Quarter) && dur.dots <= 1;

            // Long durations or quarter-family slashes (outside tuplets) are stemless
            result[i] = is_long_duration || is_quarter_slash;
        }

        result
    }

    /// Set the rhythm pattern (all entries are notes).
    #[must_use]
    pub fn rhythm(mut self, rhythm: Vec<Duration>) -> Self {
        self.rest_positions = vec![false; rhythm.len()];
        self.rhythm = rhythm;
        self
    }

    /// Set the rhythm pattern using RhythmEntry (notes and rests).
    ///
    /// This allows mixing notes and rests in the rhythm pattern.
    /// Based on MuseScore's ChordRest pattern.
    ///
    /// # Example
    /// ```ignore
    /// use engraver::notation::{MeasureBuilder, RhythmEntry, Duration};
    ///
    /// // Triplet with rest, note, rest pattern
    /// let measure = MeasureBuilder::new()
    ///     .entries(vec![
    ///         RhythmEntry::Rest(Duration::TripletEighth),
    ///         RhythmEntry::Note(Duration::TripletEighth),
    ///         RhythmEntry::Rest(Duration::TripletEighth),
    ///     ])
    ///     .triplet(0, 3)
    ///     .build(&ctx);
    /// ```
    #[must_use]
    pub fn entries(mut self, entries: Vec<RhythmEntry>) -> Self {
        self.rhythm = entries.iter().map(|e| e.duration()).collect();
        self.rest_positions = entries.iter().map(|e| e.is_rest()).collect();
        self
    }

    /// Add a single duration to the rhythm (as a note).
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(mut self, duration: Duration) -> Self {
        self.rhythm.push(duration);
        self.rest_positions.push(false);
        self
    }

    /// Add a single entry (note or rest) to the rhythm.
    #[must_use]
    pub fn add_entry(mut self, entry: RhythmEntry) -> Self {
        self.rhythm.push(entry.duration());
        self.rest_positions.push(entry.is_rest());
        self
    }

    /// Build rhythm and tuplet groups from quantized MIDI durations.
    ///
    /// This method integrates with the quantization system to automatically:
    /// 1. Convert quantized durations to notation durations
    /// 2. Detect and configure tuplet groups (triplets, quintuplets, etc.)
    ///
    /// # Arguments
    /// * `quantized` - Pre-quantized durations from `quantize_duration_batch()`
    /// * `start_positions` - Start positions of each note in ticks
    /// * `config` - The quantization configuration used
    ///
    /// # Example
    /// ```ignore
    /// use engraver::quantize::{QuantizeConfig, quantize_duration_batch};
    ///
    /// let config = QuantizeConfig::reaper();
    /// let durations = vec![320, 320, 320]; // Three triplet quarters at 960 PPQ
    /// let positions = vec![0, 320, 640];
    ///
    /// let quantized = quantize_duration_batch(&durations, &positions, &config);
    ///
    /// let measure = MeasureBuilder::new()
    ///     .time_signature(4, 4)
    ///     .from_quantized(&quantized, &positions, &config)
    ///     .build(&ctx);
    /// ```
    #[must_use]
    pub fn from_quantized(
        mut self,
        quantized: &[QuantizedDuration],
        start_positions: &[i32],
        config: &QuantizeConfig,
    ) -> Self {
        // Convert quantized durations to notation durations
        self.rhythm = quantized.iter().map(|q| q.to_duration()).collect();

        // Detect tuplet groups and convert to TupletSpec
        let groups = detect_tuplet_groups(quantized, start_positions, config);
        self.tuplet_groups = groups
            .into_iter()
            .map(|g| TupletSpec::new(g.start_idx, g.end_idx, g.ratio))
            .collect();

        self
    }

    /// Set per-note head type overrides.
    ///
    /// Each entry corresponds to a note in the rhythm array by index.
    /// `Some(head_type)` overrides the mode's default, `None` uses the mode's default.
    ///
    /// This allows mixing standard noteheads with slash noteheads in the same measure,
    /// useful for showing melody notes alongside rhythm slashes.
    #[must_use]
    pub fn head_type_overrides(mut self, overrides: Vec<Option<NoteHeadType>>) -> Self {
        self.head_type_overrides = overrides;
        self
    }

    /// Set the starting barline.
    #[must_use]
    pub fn start_barline(mut self, barline: BarlineType) -> Self {
        self.start_barline = Some(barline);
        self
    }

    /// Set the ending barline.
    #[must_use]
    pub fn end_barline(mut self, barline: BarlineType) -> Self {
        self.end_barline = Some(barline);
        self
    }

    /// Disable all barlines (for when barlines are handled externally).
    #[must_use]
    pub fn no_barlines(mut self) -> Self {
        self.start_barline = None;
        self.end_barline = None;
        self
    }

    /// Set the target width and enable justification.
    #[must_use]
    pub fn justify_to(mut self, width_spatiums: f64) -> Self {
        self.width = Some(width_spatiums);
        self.justify = true;
        self
    }

    /// Set the ID base for generated elements.
    #[must_use]
    pub fn id_base(mut self, base: u64) -> Self {
        self.id_base = base;
        self
    }

    /// Build the measure scene with automatic spacing.
    #[must_use]
    pub fn build(self, ctx: &LayoutContext) -> MeasureScene {
        let spatium = ctx.spatium();
        let mut segment_vec: Vec<Segment> = Vec::new(); // Use Vec first, convert to SegmentList after sorting
        let mut scene_elements: Vec<SceneElement> = Vec::new();
        let mut current_tick: i32 = 0;
        let mut id = self.id_base;

        // Compute auto-stemless flags for rhythmic notation
        // If explicit stemless is set, all notes are stemless
        // Otherwise, compute based on duration (whole/half = stemless slashes)
        let auto_stemless_flags = if self.stemless {
            vec![true; self.rhythm.len()]
        } else {
            self.compute_auto_stemless()
        };
        let mut rhythm_index: usize = 0;

        // Get mode-specific settings
        let default_head_type = self.mode.notehead_type();
        let stem_dir = self.mode.default_stem_direction();
        let note_line = self.mode.default_line();

        // Helper to get per-note pitch (line + accidental) for melody notes
        let get_note_pitch = |idx: usize| -> (i32, Accidental) {
            self.note_pitches
                .get(idx)
                .and_then(|opt| *opt)
                .unwrap_or((note_line, Accidental::None))
        };

        // Helper to get the polyphony stack (extras) for a rhythm entry.
        // Returns an empty slice when nothing's stacked. Mirrors MuseScore's
        // `Chord::notes()` minus the primary head: every entry shares the
        // same stem and x position, with each notehead at its own line.
        let get_pitch_stack = |idx: usize| -> &[(i32, Accidental)] {
            self.note_pitch_stacks
                .get(idx)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
        };

        // Helper to get head type for a specific note index
        // Auto-stemless notes use slash noteheads, others use override or default
        let get_head_type = |idx: usize| -> NoteHeadType {
            // Check explicit override first
            if let Some(Some(override_type)) = self.head_type_overrides.get(idx) {
                return *override_type;
            }
            // Auto-stemless notes (whole/half) use slash noteheads
            if auto_stemless_flags.get(idx).copied().unwrap_or(false) {
                return NoteHeadType::Slash;
            }
            default_head_type
        };

        // 1. Add clef segment
        if let Some(clef_type) = self.clef.filter(|_| self.show_clef) {
            let mut seg = Segment::clef(current_tick);
            seg.min_width = spatium * 4.0; // Approximate clef width (use min_width so spacing respects it)
            segment_vec.push(seg);

            let (_, clef_node) = layout_clef(
                &ClefParams {
                    id,
                    clef_type,
                    ..Default::default()
                },
                ctx,
            );
            scene_elements.push(SceneElement::Clef {
                id,
                node: clef_node,
            });
            id += 1;
        }

        // 2. Add time signature segment
        if let Some(ts) = self.time_signature.filter(|_| self.show_time_signature) {
            let mut seg = Segment::time_sig(current_tick);
            seg.min_width = spatium * 3.0; // Approximate time sig width (use min_width so spacing respects it)
            segment_vec.push(seg);

            let (_, ts_node) = layout_timesig(
                &TimeSigParams {
                    id,
                    sig_type: TimeSigType::Numeric {
                        numerator: ts.numerator,
                        denominator: ts.denominator,
                    },
                    color: self.time_signature_color,
                    ..Default::default()
                },
                ctx,
            );
            scene_elements.push(SceneElement::TimeSignature { id, node: ts_node });
            id += 1;
        }

        // 3. Add start barline if specified
        if let Some(barline_type) = self.start_barline {
            let mut seg = Segment::barline(current_tick);
            seg.min_width = spatium * 1.0; // Use min_width so spacing respects it
            segment_vec.push(seg);

            let (_, bl_node) = layout_barline(
                &BarlineParams {
                    id,
                    barline_type,
                    ..Default::default()
                },
                ctx,
            );
            scene_elements.push(SceneElement::Barline { id, node: bl_node });
            id += 1;
        }

        // 4. Group rhythm into beam groups based on time signature
        let beam_groups = self.compute_beam_groups();

        // 5. Add chord/rest segments for each note
        // Track chord/rest segment index separately for min_widths
        let mut chord_rest_seg_idx: usize = 0;
        for group in &beam_groups {
            if group.notes.len() == 1 {
                // Single note/rest - use chord/rest layout
                let dur = group.notes[0];
                let mut seg = Segment::chord_rest(current_tick, dur.ticks());
                seg.ticks = dur.ticks();
                // Apply min width from chord symbol if provided
                if let Some(&min_w) = self.segment_min_widths.get(chord_rest_seg_idx) {
                    seg.min_width = seg.min_width.max(min_w);
                }
                chord_rest_seg_idx += 1;
                segment_vec.push(seg);

                // Check if this is a rest
                let is_rest = self
                    .rest_positions
                    .get(rhythm_index)
                    .copied()
                    .unwrap_or(false);

                if is_rest {
                    // Create rest node
                    let (_, rest_node) = layout_rest(
                        &RestParams {
                            id,
                            duration: duration_to_rest_duration(dur),
                            dots: dur.dots(),
                            line: 0, // Center line for rests
                        },
                        ctx,
                    );

                    scene_elements.push(SceneElement::Rest {
                        id,
                        node: rest_node,
                        tick: current_tick,
                        rhythm_index,
                    });
                } else {
                    // Get stemless flag for this note from auto-computed or explicit stemless
                    let note_stemless = auto_stemless_flags
                        .get(rhythm_index)
                        .copied()
                        .unwrap_or(false);

                    let note_head_type = get_head_type(rhythm_index);
                    let (actual_line, actual_acc) = get_note_pitch(rhythm_index);
                    // Build the chord-note list: primary + any polyphony
                    // stack. `layout_chord` already renders multiple
                    // noteheads sharing a stem (MuseScore parity).
                    let mut chord_notes =
                        Vec::with_capacity(1 + get_pitch_stack(rhythm_index).len());
                    chord_notes.push(ChordNote {
                        line: actual_line,
                        accidental: actual_acc,
                        tie: false,
                    });
                    for (extra_line, extra_acc) in get_pitch_stack(rhythm_index) {
                        chord_notes.push(ChordNote {
                            line: *extra_line,
                            accidental: *extra_acc,
                            tie: false,
                        });
                    }
                    let (_, chord_node) = layout_chord(
                        &ChordParams {
                            id,
                            duration: dur.to_note_duration(),
                            head_type: note_head_type,
                            notes: chord_notes,
                            stem_direction: stem_dir,
                            dots: dur.dots(),
                            beamed: false,
                            stemless: note_stemless,
                        },
                        ctx,
                    );

                    scene_elements.push(SceneElement::Chord {
                        id,
                        node: chord_node,
                        tick: current_tick,
                        rhythm_index,
                        stem_direction: stem_dir,
                        stemless: note_stemless,
                    });
                }

                current_tick += dur.ticks();
                rhythm_index += 1;
                id += 1;
            } else if group.notes.len() >= 2 && group.notes.iter().any(|d| d.needs_flag()) {
                // Beamed group (2+ notes with at least one flagged) - create beam notes
                let mut beam_notes: Vec<BeamNoteInfo> = Vec::new();
                // Use first note's head type for the whole beam group
                let beam_head_type = get_head_type(rhythm_index);

                for dur in &group.notes {
                    let mut seg = Segment::chord_rest(current_tick, dur.ticks());
                    seg.ticks = dur.ticks();
                    // Apply min width from chord symbol if provided
                    if let Some(&min_w) = self.segment_min_widths.get(chord_rest_seg_idx) {
                        seg.min_width = seg.min_width.max(min_w);
                    }
                    chord_rest_seg_idx += 1;
                    segment_vec.push(seg);

                    // Store info for beam layout (x position will be set after spacing)
                    let (beam_line, beam_acc) = get_note_pitch(rhythm_index);
                    beam_notes.push(BeamNoteInfo {
                        id,
                        tick: current_tick,
                        duration: *dur,
                        line: beam_line,
                        accidental: beam_acc,
                        extras: get_pitch_stack(rhythm_index).to_vec(),
                    });

                    current_tick += dur.ticks();
                    rhythm_index += 1;
                    id += 1;
                }

                // Per-beam-group auto stem direction (mirrors MuseScore
                // computeAutoStemDirection): sum every notehead's line
                // position relative to the staff middle line. In our
                // convention positive = above the middle, so a positive
                // sum (chord weight above the staff) gets a *down* stem
                // and a negative sum gets *up*. Tie → down.
                let line_sum: i32 = beam_notes
                    .iter()
                    .map(|n| n.line + n.extras.iter().map(|(l, _)| *l).sum::<i32>())
                    .sum();
                let auto_stem_dir = if line_sum > 0 {
                    StemDirection::Down
                } else {
                    StemDirection::Up
                };
                let group_stem_dir = match stem_dir {
                    StemDirection::Auto => auto_stem_dir,
                    explicit => explicit,
                };
                // Apply chosen direction to every beam note so beam
                // layout positions stems on the correct side.
                for bn in beam_notes.iter_mut() {
                    // nothing per-note today, but reserved for future
                    // explicit per-note overrides from MusicXML <stem>.
                    let _ = bn;
                }
                scene_elements.push(SceneElement::BeamGroup {
                    notes: beam_notes,
                    head_type: beam_head_type,
                    stem_dir: group_stem_dir,
                });
            } else {
                // Multiple non-flagged notes - individual chords
                for dur in &group.notes {
                    let mut seg = Segment::chord_rest(current_tick, dur.ticks());
                    seg.ticks = dur.ticks();
                    // Apply min width from chord symbol if provided
                    if let Some(&min_w) = self.segment_min_widths.get(chord_rest_seg_idx) {
                        seg.min_width = seg.min_width.max(min_w);
                    }
                    chord_rest_seg_idx += 1;
                    segment_vec.push(seg);

                    // Get stemless flag for this note from auto-computed or explicit stemless
                    let note_stemless = auto_stemless_flags
                        .get(rhythm_index)
                        .copied()
                        .unwrap_or(false);

                    let note_head_type = get_head_type(rhythm_index);
                    let (actual_line, actual_acc) = get_note_pitch(rhythm_index);
                    // Build the chord-note list: primary + any polyphony
                    // stack. `layout_chord` already renders multiple
                    // noteheads sharing a stem (MuseScore parity).
                    let mut chord_notes =
                        Vec::with_capacity(1 + get_pitch_stack(rhythm_index).len());
                    chord_notes.push(ChordNote {
                        line: actual_line,
                        accidental: actual_acc,
                        tie: false,
                    });
                    for (extra_line, extra_acc) in get_pitch_stack(rhythm_index) {
                        chord_notes.push(ChordNote {
                            line: *extra_line,
                            accidental: *extra_acc,
                            tie: false,
                        });
                    }
                    let (_, chord_node) = layout_chord(
                        &ChordParams {
                            id,
                            duration: dur.to_note_duration(),
                            head_type: note_head_type,
                            notes: chord_notes,
                            stem_direction: stem_dir,
                            dots: dur.dots(),
                            beamed: false,
                            stemless: note_stemless,
                        },
                        ctx,
                    );

                    scene_elements.push(SceneElement::Chord {
                        id,
                        node: chord_node,
                        tick: current_tick,
                        rhythm_index,
                        stem_direction: stem_dir,
                        stemless: note_stemless,
                    });

                    current_tick += dur.ticks();
                    rhythm_index += 1;
                    id += 1;
                }
            }
        }

        // 6. Add end barline
        if let Some(barline_type) = self.end_barline {
            let mut seg = Segment::end_barline(current_tick);
            seg.min_width = spatium * 1.0; // Use min_width so spacing respects it
            segment_vec.push(seg);

            let (_, bl_node) = layout_barline(
                &BarlineParams {
                    id,
                    barline_type,
                    ..Default::default()
                },
                ctx,
            );
            scene_elements.push(SceneElement::Barline { id, node: bl_node });
        }

        // 7. Sort segments by tick and type, then convert to SegmentList
        segment_vec.sort();
        let mut segments = SegmentList::from_sorted(segment_vec);

        // 8. Apply bar-note distance (leading space) to first chord/rest segment
        // and note-bar distance (trailing space) to target width
        // In compact mode, use minimal spacing (just enough to clear the barline)
        let bar_note_distance = if self.compact {
            spatium * 0.3 // Minimal spacing for compact measures
        } else {
            ctx.style_distance(crate::engraver::style::Sid::BarNoteDistance)
        };
        let note_bar_distance = if self.compact {
            spatium * 0.3 // Minimal spacing for compact measures
        } else {
            ctx.style_distance(crate::engraver::style::Sid::NoteBarDistance)
        };

        // Find first ChordRest segment and add leading space
        if let Some(first_chord) = segments.iter_mut().find(|s| s.seg_type.is_chord_rest()) {
            first_chord.extra_leading_space = bar_note_distance;
        }

        // 9. Apply horizontal spacing
        // Account for bar margins when justifying
        let spacing = HorizontalSpacing::new(spatium);
        let target_width = self
            .width
            .map(|w| {
                let full_width = w * spatium;
                // When justifying, the available space is reduced by the trailing margin
                if self.justify {
                    full_width - note_bar_distance
                } else {
                    full_width
                }
            })
            .unwrap_or(f64::MAX);
        let spacing_result = spacing.compute_spacing(&mut segments, target_width, self.justify);

        // 9. Build final scene with computed positions
        let scene = self.build_scene(ctx, &segments, &scene_elements);
        let note_line_stacks = self
            .note_pitches
            .iter()
            .enumerate()
            .map(|(idx, pitch)| {
                pitch.map(|(line, _)| {
                    let mut min_line = line;
                    let mut max_line = line;
                    if let Some(extras) = self.note_pitch_stacks.get(idx) {
                        for (extra_line, _) in extras {
                            min_line = min_line.min(*extra_line);
                            max_line = max_line.max(*extra_line);
                        }
                    }
                    (min_line, max_line)
                })
            })
            .collect();

        MeasureScene {
            scene,
            width: spacing_result.total_width,
            segments,
            note_line_stacks,
            internal_push_positions: Vec::new(), // Set by chart layout when needed
            spillback_positions: Vec::new(),     // Set by chart layout when needed
        }
    }

    /// Compute beam groups based on beat boundaries.
    ///
    /// Rules:
    /// 1. Non-flagged notes (quarter and longer) are never beamed
    /// 2. Flagged notes (8ths, 16ths, 32nds) are grouped within beats
    /// 3. Beam groups never cross beat boundaries
    /// 4. Within a beat, all consecutive flagged notes are beamed together
    /// 5. Rests break beam groups (rests are never beamed).
    ///
    /// Pulse boundaries come from `TimeSignature::beam_groups()` so
    /// compound meters get the canonical 3-eighth (dotted-quarter)
    /// grouping. In 6/8 that means a pair of dotted-eighth notes — total
    /// 720 ticks — beams as one group instead of breaking on the eighth
    /// "beat" boundary. Matches MuseScore's `Beam::layout` partitioning.
    fn compute_beam_groups(&self) -> Vec<BeamGroup> {
        if self.rhythm.is_empty() {
            return Vec::new();
        }

        let ts = self.time_signature.unwrap_or(TimeSignature::COMMON);
        // Beam-group boundary ticks: cumulative sum of pulse durations.
        // For 6/8 → [720, 1440]; a note ending at 720 closes the first
        // group; the next starts there.
        let beam_pulses = ts.beam_groups();
        let group_boundaries: Vec<i32> = beam_pulses
            .iter()
            .scan(0i32, |acc, &g| {
                *acc += g;
                Some(*acc)
            })
            .collect();
        let is_group_boundary = |tick: i32| -> bool {
            tick > 0
                && group_boundaries.iter().any(|&b| {
                    tick % group_boundaries.last().copied().unwrap_or(b) == 0 || tick == b
                })
        };
        let group_idx_at = |tick: i32| -> usize {
            for (i, &b) in group_boundaries.iter().enumerate() {
                if tick < b {
                    return i;
                }
            }
            group_boundaries.len().saturating_sub(1)
        };

        let mut groups: Vec<BeamGroup> = Vec::new();
        let mut current_group: Vec<Duration> = Vec::new();
        let mut current_tick: i32 = 0;

        for (idx, &dur) in self.rhythm.iter().enumerate() {
            let dur_ticks = dur.ticks();
            let needs_flag = dur.needs_flag();
            let is_rest = self.rest_positions.get(idx).copied().unwrap_or(false);

            let start_pulse = group_idx_at(current_tick);
            let end_tick = current_tick + dur_ticks;
            let end_pulse = group_idx_at(end_tick.saturating_sub(1));

            if is_rest {
                if !current_group.is_empty() {
                    groups.push(BeamGroup {
                        notes: std::mem::take(&mut current_group),
                    });
                }
                groups.push(BeamGroup { notes: vec![dur] });
                current_tick = end_tick;
                continue;
            }

            if !needs_flag {
                if !current_group.is_empty() {
                    groups.push(BeamGroup {
                        notes: std::mem::take(&mut current_group),
                    });
                }
                groups.push(BeamGroup { notes: vec![dur] });
                current_tick = end_tick;
                continue;
            }

            let crosses_pulse = start_pulse != end_pulse;

            if is_group_boundary(current_tick) && !current_group.is_empty() {
                groups.push(BeamGroup {
                    notes: std::mem::take(&mut current_group),
                });
            }

            current_group.push(dur);
            current_tick = end_tick;

            if crosses_pulse || is_group_boundary(current_tick) {
                groups.push(BeamGroup {
                    notes: std::mem::take(&mut current_group),
                });
            }
        }

        if !current_group.is_empty() {
            groups.push(BeamGroup {
                notes: current_group,
            });
        }

        groups
    }

    /// Build the final scene from computed segment positions.
    fn build_scene(
        &self,
        ctx: &LayoutContext,
        segments: &SegmentList,
        elements: &[SceneElement],
    ) -> SceneNode {
        let spatium = ctx.spatium();
        let note_line = self.mode.default_line();
        let mut root = SceneNode::group(SemanticId::new(ElementType::Measure, self.id_base));

        // Helper to find segment X by tick and type
        let find_segment_x = |tick: i32, seg_type: SegmentType| -> f64 {
            for seg in segments.iter() {
                if seg.tick == tick && seg.seg_type == seg_type {
                    return seg.x;
                }
            }
            0.0
        };

        // Helper to find chord/rest segment X by tick
        let find_chord_x = |tick: i32| -> f64 {
            for seg in segments.iter() {
                if seg.tick == tick && seg.seg_type.is_chord_rest() {
                    return seg.x;
                }
            }
            // Fallback: find closest segment before this tick
            segments
                .iter()
                .filter(|s| s.tick <= tick && s.seg_type.is_chord_rest())
                .last()
                .map(|s| s.x)
                .unwrap_or(0.0)
        };

        // Track which tick positions have been used for barlines
        let mut barline_count = 0;

        for element in elements {
            match element {
                SceneElement::Clef { id, node } => {
                    // Clef is at tick 0 with CLEF segment type
                    let x = find_segment_x(0, SegmentType::CLEF);
                    let mut container = SceneNode::group(SemanticId::new(ElementType::Clef, *id));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(node.clone());
                    root.add_child(container);
                }
                SceneElement::TimeSignature { id, node } => {
                    // Time sig is at tick 0 with TIME_SIG segment type
                    let x = find_segment_x(0, SegmentType::TIME_SIG);
                    let mut container =
                        SceneNode::group(SemanticId::new(ElementType::TimeSignature, *id));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(node.clone());
                    root.add_child(container);
                }
                SceneElement::Barline { id, node } => {
                    // Find the barline segment - first one is start barline, last is end barline
                    let barline_segments: Vec<_> = segments
                        .iter()
                        .filter(|s| s.seg_type.is_barline())
                        .collect();

                    let x = if barline_count < barline_segments.len() {
                        barline_segments[barline_count].x
                    } else {
                        // End barline - position at the end of last segment
                        segments.iter().last().map(|s| s.x + s.width).unwrap_or(0.0)
                    };
                    barline_count += 1;

                    let mut container =
                        SceneNode::group(SemanticId::new(ElementType::Barline, *id));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(node.clone());
                    root.add_child(container);
                }
                SceneElement::Chord {
                    id,
                    node,
                    tick,
                    rhythm_index: _,
                    stem_direction: _,
                    stemless: _,
                } => {
                    let x = find_chord_x(*tick);
                    let mut container = SceneNode::group(SemanticId::chord(*id));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(node.clone());
                    root.add_child(container);
                }
                SceneElement::Rest {
                    id,
                    node,
                    tick,
                    rhythm_index: _,
                } => {
                    let x = find_chord_x(*tick);
                    let mut container = SceneNode::group(SemanticId::new(ElementType::Rest, *id));
                    container.transform = Affine::translate((x, 0.0));
                    container.add_child(node.clone());
                    root.add_child(container);
                }
                SceneElement::BeamGroup {
                    notes,
                    head_type,
                    stem_dir,
                } => {
                    // Build beam notes with computed X positions. Noteheads
                    // are anchored directly on the rhythmic segment; accidentals
                    // extend left and must not move the stem/beam x.
                    let beam_notes: Vec<BeamNote> = notes
                        .iter()
                        .map(|info| {
                            let x = find_chord_x(info.tick);
                            // Compute the full chord range from primary +
                            // extras so the beam stem reaches every notehead.
                            let mut top_line = info.line;
                            let mut bottom_line = info.line;
                            for (extra_line, _) in &info.extras {
                                top_line = top_line.max(*extra_line);
                                bottom_line = bottom_line.min(*extra_line);
                            }
                            BeamNote {
                                x,
                                line: info.line,
                                top_line,
                                bottom_line,
                                duration: info.duration.to_note_duration(),
                                stem_direction: *stem_dir,
                                head_type: *head_type,
                            }
                        })
                        .collect();

                    // Layout noteheads (each with its own line and accidental).
                    // Polyphony stack: for each beam note, also emit one
                    // extra notehead per entry in `extras` at the same x.
                    // The shared stem is drawn by the beam pass itself;
                    // extras render as additional heads + ledger lines.
                    for info in notes {
                        let x = find_chord_x(info.tick);
                        let has_pitch =
                            info.line != note_line || info.accidental != Accidental::None;
                        let (_, note_node) = layout_note(
                            &NoteParams {
                                id: info.id,
                                duration: info.duration.to_note_duration(),
                                head_type: *head_type,
                                line: info.line,
                                accidental: info.accidental,
                                dots: info.duration.dots(),
                                ledger_lines: has_pitch,
                                ..Default::default()
                            },
                            ctx,
                        );
                        let mut container =
                            SceneNode::group(SemanticId::new(ElementType::Note, info.id));
                        container.transform = Affine::translate((x, 0.0));
                        container.add_child(note_node);
                        root.add_child(container);

                        for (extra_idx, (extra_line, extra_acc)) in info.extras.iter().enumerate() {
                            let (_, extra_node) = layout_note(
                                &NoteParams {
                                    id: info.id ^ ((extra_idx as u64 + 1) << 32),
                                    duration: info.duration.to_note_duration(),
                                    head_type: *head_type,
                                    line: *extra_line,
                                    accidental: *extra_acc,
                                    dots: info.duration.dots(),
                                    ledger_lines: true,
                                    // layout_note draws only the notehead +
                                    // accidental + ledger lines (no stem),
                                    // so the beam's primary stem isn't
                                    // duplicated by extra heads.
                                    ..Default::default()
                                },
                                ctx,
                            );
                            let mut extra_container = SceneNode::group(SemanticId::new(
                                ElementType::Note,
                                info.id ^ ((extra_idx as u64 + 1) << 32),
                            ));
                            extra_container.transform = Affine::translate((x, 0.0));
                            extra_container.add_child(extra_node);
                            root.add_child(extra_container);
                        }
                    }

                    // Layout beam
                    let beam_config = BeamLayoutConfig::default();
                    let beam_result = layout_beam(&beam_notes, spatium, &beam_config);
                    let beam_node = SceneNode::anonymous_leaf(beam_result.commands);
                    root.add_child(beam_node);
                }
            }
        }

        // Render tuplet brackets for any tuplet groups
        // Use TARGET width (where barline will be drawn), not actual computed width
        // This ensures brackets don't extend past barlines even when chord symbols
        // push content wider than the allocated measure width
        if !self.tuplet_groups.is_empty() {
            let target_width = self.width.map(|w| w * spatium);
            {
                let actual_width = segments.total_width();
                tracing::debug!(
                    "[measure-builder] Rendering {} tuplet groups, boundary={:?} (actual={:.1})",
                    self.tuplet_groups.len(),
                    target_width,
                    actual_width,
                );
            }
            self.render_tuplet_brackets(ctx, segments, elements, &mut root, target_width);
        }

        root
    }

    /// Render tuplet brackets for all tuplet groups.
    fn render_tuplet_brackets(
        &self,
        ctx: &LayoutContext,
        segments: &SegmentList,
        elements: &[SceneElement],
        root: &mut SceneNode,
        measure_width: Option<f64>,
    ) {
        let spatium = ctx.spatium();
        let note_line = self.mode.default_line();

        // Helper to find chord X position by tick
        let find_chord_x = |tick: i32| -> f64 {
            for seg in segments.iter() {
                if seg.tick == tick && seg.seg_type.is_chord_rest() {
                    return seg.x;
                }
            }
            0.0
        };

        // Collect chord/rest info by rhythm_index for tuplet layout
        // For rests, we use default stem direction and mark them as rests
        let mut entry_info: std::collections::HashMap<usize, (f64, StemDirection, bool, bool)> =
            std::collections::HashMap::new(); // (x, stem_dir, stemless, is_rest)
        for element in elements {
            match element {
                SceneElement::Chord {
                    tick,
                    rhythm_index,
                    stem_direction,
                    stemless,
                    ..
                } => {
                    let x = find_chord_x(*tick);
                    entry_info.insert(*rhythm_index, (x, *stem_direction, *stemless, false));
                }
                SceneElement::Rest {
                    tick, rhythm_index, ..
                } => {
                    let x = find_chord_x(*tick);
                    // Rests use Up stem direction for bracket positioning and are marked as rests
                    entry_info.insert(*rhythm_index, (x, StemDirection::Up, true, true));
                }
                _ => {}
            }
        }

        // Render each tuplet group
        let mut tuplet_id = self.id_base + 10000; // Use high ID range for tuplets
        for tuplet_spec in &self.tuplet_groups {
            let tuplet_notes: Vec<TupletNote> = (tuplet_spec.start_idx..tuplet_spec.end_idx)
                .filter_map(|idx| {
                    entry_info
                        .get(&idx)
                        .map(|(x, stem_dir, stemless, is_rest)| {
                            // Calculate Y positions based on staff line and stem direction
                            let y_head = -(note_line as f64 * spatium / 2.0);
                            let stem_length = spatium * 3.5; // Standard stem length

                            // For rests in tuplets, use a virtual stem tip so the bracket
                            // stays horizontal. This matches MuseScore behavior.
                            let (y_stem_tip, resolved_dir) = if *is_rest {
                                // Rests use a virtual stem tip at the same position as Up stems
                                // This keeps the tuplet bracket horizontal
                                (Some(y_head - stem_length), StemDirection::Up)
                            } else if *stemless {
                                (None, StemDirection::Up) // Default for stemless
                            } else {
                                match stem_dir {
                                    StemDirection::Up => {
                                        (Some(y_head - stem_length), StemDirection::Up)
                                    }
                                    StemDirection::Down => {
                                        (Some(y_head + stem_length), StemDirection::Down)
                                    }
                                    StemDirection::Auto => {
                                        // Auto defaults to up for slash notation
                                        (Some(y_head - stem_length), StemDirection::Up)
                                    }
                                }
                            };

                            TupletNote {
                                x: *x,
                                y_head,
                                y_stem_tip,
                                stem_direction: resolved_dir,
                                is_rest: *is_rest,
                            }
                        })
                })
                .collect();

            if tuplet_notes.len() >= 2 {
                let config = TupletConfig::default();
                let tuplet_layout = layout_tuplet(
                    &tuplet_notes,
                    tuplet_spec.ratio,
                    tuplet_id,
                    spatium,
                    &config,
                    measure_width,
                );
                root.add_child(tuplet_layout.scene);
                tuplet_id += 1;
            }
        }
    }
}

/// Information about a beam note before position is computed.
#[derive(Debug, Clone)]
struct BeamNoteInfo {
    id: u64,
    tick: i32,
    duration: Duration,
    /// Per-note staff line (for melody pitch rendering)
    line: i32,
    /// Per-note accidental (for melody pitch rendering)
    accidental: Accidental,
    /// Polyphony stack — extra noteheads on this beat's stem (octave
    /// doublings / double-stops). Each entry is (staff_line, accidental).
    /// The beam itself draws once for the primary head; extras render as
    /// stand-alone noteheads at the same x with their own ledger lines.
    extras: Vec<(i32, Accidental)>,
}

/// A group of notes that should be beamed together.
#[derive(Debug, Clone)]
struct BeamGroup {
    notes: Vec<Duration>,
}

/// Scene element before final positioning.
#[derive(Debug, Clone)]
enum SceneElement {
    Clef {
        id: u64,
        node: SceneNode,
    },
    TimeSignature {
        id: u64,
        node: SceneNode,
    },
    Barline {
        id: u64,
        node: SceneNode,
    },
    Chord {
        id: u64,
        node: SceneNode,
        tick: i32,
        /// Index in the rhythm array (for tuplet grouping)
        rhythm_index: usize,
        /// Stem direction for tuplet bracket positioning
        stem_direction: StemDirection,
        /// Whether this note is stemless
        stemless: bool,
    },
    /// A rest (following MuseScore's ChordRest pattern)
    Rest {
        id: u64,
        node: SceneNode,
        tick: i32,
        /// Index in the rhythm array (for tuplet grouping)
        rhythm_index: usize,
    },
    BeamGroup {
        notes: Vec<BeamNoteInfo>,
        head_type: NoteHeadType,
        stem_dir: StemDirection,
    },
}

/// Convert a Duration to a RestDuration for rest layout.
fn duration_to_rest_duration(dur: Duration) -> RestDuration {
    use crate::engraver::model::DurationKind;
    match dur.kind {
        DurationKind::Whole => RestDuration::Whole,
        DurationKind::Half => RestDuration::Half,
        DurationKind::Quarter => RestDuration::Quarter,
        DurationKind::Eighth => RestDuration::Eighth,
        DurationKind::Sixteenth => RestDuration::Sixteenth,
        DurationKind::ThirtySecond => RestDuration::ThirtySecond,
        DurationKind::SixtyFourth => RestDuration::SixtyFourth,
    }
}

/// Result of building a measure.
#[derive(Debug)]
pub struct MeasureScene {
    /// The scene graph for the measure
    pub scene: SceneNode,
    /// Total width after spacing
    pub width: f64,
    /// The segment list (for debugging/inspection)
    pub segments: SegmentList,
    /// Per-rhythm-entry notehead stack bounds as `(min_line, max_line)`.
    /// Used by chart layout to keep chord symbols clear of noteheads and
    /// ledger lines while preserving beat alignment.
    pub note_line_stacks: Vec<Option<(i32, i32)>>,
    /// Internal push positions: maps chord_idx to segment_idx for pushed chords
    /// (for chords that push back within the same measure, not spillbacks)
    pub internal_push_positions: Vec<(usize, usize)>,
    /// Spillback positions: maps (rhythm_idx, chord_symbol) for chords from next
    /// measure that push back across the barline. Used to place spillback chord
    /// symbols at correct triplet positions.
    pub spillback_positions: Vec<(usize, String)>,
}

/// Builder for creating a system (line) of multiple measures.
#[derive(Debug, Clone)]
pub struct SystemBuilder {
    /// Measures in this system
    measures: Vec<MeasureBuilder>,
    /// Total system width in spatiums
    system_width: f64,
    /// Staff Y position
    staff_y: f64,
}

impl SystemBuilder {
    /// Create a new system builder.
    #[must_use]
    pub fn new(system_width: f64) -> Self {
        Self {
            measures: Vec::new(),
            system_width,
            staff_y: 0.0,
        }
    }

    /// Add a measure to the system.
    #[must_use]
    pub fn measure(mut self, measure: MeasureBuilder) -> Self {
        self.measures.push(measure);
        self
    }

    /// Set the staff Y position.
    #[must_use]
    pub fn at_y(mut self, y: f64) -> Self {
        self.staff_y = y;
        self
    }

    /// Build the system scene.
    #[must_use]
    pub fn build(self, ctx: &LayoutContext) -> SystemScene {
        let spatium = ctx.spatium();
        let mut root = SceneNode::group(SemanticId::new(ElementType::System, 1));

        // Draw staff lines
        let staff_lines = draw_staff_lines(0.0, 0.0, self.system_width * spatium, spatium);
        root.add_child(SceneNode::anonymous_leaf(staff_lines));

        // Build each measure and position them sequentially
        let mut x_offset = 0.0;
        let mut measure_scenes = Vec::new();

        for (i, measure) in self.measures.into_iter().enumerate() {
            let measure_scene = measure.id_base((i as u64 + 1) * 1000).build(ctx);

            let mut measure_container =
                SceneNode::group(SemanticId::new(ElementType::Measure, i as u64 + 1));
            measure_container.transform = Affine::translate((x_offset, 0.0));
            measure_container.add_child(measure_scene.scene.clone());
            root.add_child(measure_container);

            x_offset += measure_scene.width;
            measure_scenes.push(measure_scene);
        }

        // Position root at staff Y
        root.transform = Affine::translate((0.0, self.staff_y));

        SystemScene {
            scene: root,
            width: x_offset,
            measures: measure_scenes,
        }
    }
}

/// Result of building a system.
#[derive(Debug)]
pub struct SystemScene {
    /// The scene graph for the system
    pub scene: SceneNode,
    /// Total width
    pub width: f64,
    /// Individual measure scenes
    pub measures: Vec<MeasureScene>,
}

/// Draw 5 staff lines.
fn draw_staff_lines(x: f64, y: f64, width: f64, spatium: f64) -> Vec<PaintCommand> {
    let mut commands = Vec::new();
    let line_thickness = spatium * 0.1;

    for i in 0..5 {
        let line_y = y + i as f64 * spatium;
        commands.push(PaintCommand::line(
            Point::new(x, line_y),
            Point::new(x + width, line_y),
            Color::BLACK,
            line_thickness,
        ));
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_stemless_all_quarters_in_4_4() {
        // 4 consecutive quarters in 4/4: beats 1-2 are one group, beats 3-4 are another
        // Both groups have 2+ quarters, so all are stemless
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Quarter,
                Duration::Quarter,
                Duration::Quarter,
                Duration::Quarter,
            ]);

        let flags = builder.compute_auto_stemless();
        // Beats 1-2 (before beat 3) = stemless, beats 3-4 (after beat 3) = stemless
        assert_eq!(flags, vec![true, true, true, true]);
    }

    #[test]
    fn test_auto_stemless_dotted_quarters_in_6_8() {
        // Compound-meter filler slashes (`/. /.` in 6/8) — dotted quarters
        // are rhythm-slash filler, not specific rhythm, so they're stemless.
        let builder = MeasureBuilder::new()
            .time_signature(6, 8)
            .rhythmic()
            .rhythm(vec![Duration::DottedQuarter, Duration::DottedQuarter]);

        let flags = builder.compute_auto_stemless();
        assert_eq!(flags, vec![true, true]);
    }

    #[test]
    fn test_auto_stemless_two_quarters() {
        // 2 consecutive quarters should be stemless (minimum threshold)
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![Duration::Quarter, Duration::Quarter]);

        let flags = builder.compute_auto_stemless();
        assert_eq!(flags, vec![true, true]);
    }

    #[test]
    fn test_auto_stemless_single_quarter() {
        // Single plain quarter is stemless (all plain quarters outside tuplets are stemless)
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![Duration::Quarter]);

        let flags = builder.compute_auto_stemless();
        assert_eq!(flags, vec![true]);
    }

    #[test]
    fn test_auto_stemless_mixed_eighths_quarters() {
        // 8th 8th Q Q Q starting on beat 1:
        // - 8th 8th = not quarters, so stemmed
        // - All plain quarters = stemless (regardless of position)
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Eighth,  // beat 1
                Duration::Eighth,  // beat 1.5
                Duration::Quarter, // beat 2
                Duration::Quarter, // beat 3
                Duration::Quarter, // beat 4
            ]);

        let flags = builder.compute_auto_stemless();
        // Eighths are stemmed, all plain quarters are stemless
        assert_eq!(flags, vec![false, false, true, true, true]);
    }

    #[test]
    fn test_auto_stemless_quarter_breaks_chain() {
        // Q 8th Q Q → all plain quarters are stemless, eighth is stemmed
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Quarter, // beat 1
                Duration::Eighth,  // beat 2 (not a quarter)
                Duration::Quarter, // beat 2.5
                Duration::Quarter, // beat 3
            ]);

        let flags = builder.compute_auto_stemless();
        // All plain quarters are stemless, eighth is stemmed
        assert_eq!(flags, vec![true, false, true, true]);
    }

    #[test]
    fn test_auto_stemless_half_note_breaks_chain() {
        // Half + Q + Q → half notes are also stemless (long duration), plain quarters are stemless
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![Duration::Half, Duration::Quarter, Duration::Quarter]);

        let flags = builder.compute_auto_stemless();
        // Half notes and plain quarters are all stemless
        assert_eq!(flags, vec![true, true, true]);
    }

    #[test]
    fn test_auto_stemless_dotted_quarter_is_filler_slash() {
        // Dotted quarters are filler rhythm slashes (e.g. `/. /.` in compound
        // meters) — like plain quarters, they get no stem unless the chord
        // carries an explicit rhythm.
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::DottedQuarter,
                Duration::DottedQuarter,
                Duration::Quarter,
            ]);

        let flags = builder.compute_auto_stemless();
        assert_eq!(flags, vec![true, true, true]);
    }

    #[test]
    fn test_auto_stemless_standard_mode_disabled() {
        // Auto-stemless only applies to rhythmic mode
        let builder = MeasureBuilder::new()
            .mode(NotationMode::Standard)
            .time_signature(4, 4)
            .rhythm(vec![
                Duration::Quarter,
                Duration::Quarter,
                Duration::Quarter,
                Duration::Quarter,
            ]);

        let flags = builder.compute_auto_stemless();
        // Standard mode = no auto-stemless
        assert_eq!(flags, vec![false, false, false, false]);
    }

    #[test]
    fn test_auto_stemless_strong_beat_crossing() {
        // In 4/4: Q rest Q Q starting on beat 2
        // Beat 2: Q (alone before beat 3) = stem
        // Beat 3: rest (not a quarter, but starts group after beat 3)
        // Beats 3.5-4: Q Q = consecutive after beat 3 = stemless
        // But wait, we start on beat 1, so let's be precise:
        // If we have rest, Q, Q, Q:
        // - rest on beat 1
        // - Q on beat 2 (alone before beat 3)
        // - Q on beat 3
        // - Q on beat 4
        // Beat 2 Q is alone (before beat 3), beats 3-4 are consecutive
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Quarter, // beat 1
                Duration::Quarter, // beat 2
                Duration::Quarter, // beat 3
                Duration::Quarter, // beat 4
            ]);

        let flags = builder.compute_auto_stemless();
        // Beats 1-2 form one group (2 quarters), beats 3-4 form another
        assert_eq!(flags, vec![true, true, true, true]);
    }

    #[test]
    fn test_auto_stemless_beat_2_3_4_pattern() {
        // 8th 8th Q Q Q: eighths are stemmed, all plain quarters are stemless
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Eighth,  // beat 1
                Duration::Eighth,  // beat 1.5
                Duration::Quarter, // beat 2
                Duration::Quarter, // beat 3
                Duration::Quarter, // beat 4
            ]);

        let flags = builder.compute_auto_stemless();
        // Eighths are stemmed, all plain quarters are stemless
        assert_eq!(flags, vec![false, false, true, true, true]);
    }

    #[test]
    fn test_auto_stemless_no_quarters_at_all() {
        // When there are NO plain quarter notes, all notes should have stems
        // This tests dotted eighths + sixteenths + eighths pattern
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::DottedEighth,
                Duration::Sixteenth,
                Duration::DottedEighth,
                Duration::Sixteenth,
                Duration::DottedEighth,
                Duration::Sixteenth,
                Duration::Eighth,
            ]);

        let flags = builder.compute_auto_stemless();
        // No plain quarters = no stemless notes
        assert_eq!(flags, vec![false, false, false, false, false, false, false]);
    }

    #[test]
    fn test_auto_stemless_syncopation_no_quarters() {
        // Complex syncopation with no plain quarters but has a half note
        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .rhythmic()
            .rhythm(vec![
                Duration::Eighth,
                Duration::DottedQuarter,
                Duration::Eighth,
                Duration::Half,
            ]);

        let flags = builder.compute_auto_stemless();
        // Eighth = stemmed, DottedQuarter = filler-slash stemless,
        // Eighth = stemmed, Half = stemless (long duration)
        assert_eq!(flags, vec![false, true, false, true]);
    }

    #[test]
    fn test_beam_groups_sixteenths_by_beat() {
        // 8 sixteenth notes in 4/4 should create 2 beam groups (4 per beat)
        let builder = MeasureBuilder::new().time_signature(4, 4).rhythm(vec![
            Duration::Sixteenth,
            Duration::Sixteenth,
            Duration::Sixteenth,
            Duration::Sixteenth, // End of beat 1
            Duration::Sixteenth,
            Duration::Sixteenth,
            Duration::Sixteenth,
            Duration::Sixteenth, // End of beat 2
        ]);

        let groups = builder.compute_beam_groups();
        // Should be 2 groups of 4 sixteenths each
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].notes.len(), 4);
        assert_eq!(groups[1].notes.len(), 4);
    }

    #[test]
    fn test_beam_groups_eighths_by_beat() {
        // 4 eighth notes in 4/4 should create 2 beam groups (2 per beat)
        let builder = MeasureBuilder::new().time_signature(4, 4).rhythm(vec![
            Duration::Eighth,
            Duration::Eighth, // End of beat 1
            Duration::Eighth,
            Duration::Eighth, // End of beat 2
        ]);

        let groups = builder.compute_beam_groups();
        // Should be 2 groups of 2 eighths each
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].notes.len(), 2);
        assert_eq!(groups[1].notes.len(), 2);
    }

    #[test]
    fn test_beam_groups_mixed_rhythms() {
        // Quarter + 2 eighths + quarter in 4/4
        let builder = MeasureBuilder::new().time_signature(4, 4).rhythm(vec![
            Duration::Quarter, // Beat 1 (not beamed)
            Duration::Eighth,  // Beat 2
            Duration::Eighth,  // Beat 2
            Duration::Quarter, // Beat 3 (not beamed)
        ]);

        let groups = builder.compute_beam_groups();
        // Should be: [Quarter], [Eighth, Eighth], [Quarter]
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].notes.len(), 1); // Quarter
        assert_eq!(groups[1].notes.len(), 2); // 2 eighths beamed
        assert_eq!(groups[2].notes.len(), 1); // Quarter
    }

    #[test]
    fn test_beam_groups_32nds_by_beat() {
        // 8 thirty-second notes in 4/4 (covers half a beat)
        let builder = MeasureBuilder::new().time_signature(4, 4).rhythm(vec![
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond,
            Duration::ThirtySecond, // Half of beat 1
        ]);

        let groups = builder.compute_beam_groups();
        // All 8 should be in one group (within beat 1)
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].notes.len(), 8);
    }

    #[test]
    fn test_beam_groups_cross_beat_boundary() {
        // 3 eighths starting on beat 1.5 should break at beat 2
        // (This is beat 1: eighth, then 2 eighths that cross into beat 2)
        let builder = MeasureBuilder::new().time_signature(4, 4).rhythm(vec![
            Duration::Eighth, // Beat 1 first half
            Duration::Eighth, // Beat 1 second half - completes beat 1
            Duration::Eighth, // Beat 2 first half
        ]);

        let groups = builder.compute_beam_groups();
        // Should be: [Eighth, Eighth] (beat 1), [Eighth] (beat 2)
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].notes.len(), 2);
        assert_eq!(groups[1].notes.len(), 1);
    }

    // Integration tests for from_quantized()
    #[test]
    fn test_from_quantized_triplet_eighths() {
        use crate::engraver::quantize::{QuantizeConfig, quantize_duration_batch};

        let config = QuantizeConfig::default();
        // Three triplet eighths (160 ticks each at 480 PPQ)
        let durations = vec![160, 160, 160];
        let positions = vec![0, 160, 320];

        let quantized = quantize_duration_batch(&durations, &positions, &config);

        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .from_quantized(&quantized, &positions, &config);

        // Should have 3 rhythm entries
        assert_eq!(builder.rhythm.len(), 3);

        // Should have 1 tuplet group (triplet)
        assert_eq!(builder.tuplet_groups.len(), 1);
        assert_eq!(builder.tuplet_groups[0].start_idx, 0);
        assert_eq!(builder.tuplet_groups[0].end_idx, 3);
        assert_eq!(builder.tuplet_groups[0].ratio, TupletRatio::triplet());
    }

    #[test]
    fn test_from_quantized_mixed_rhythms() {
        use crate::engraver::quantize::{QuantizeConfig, quantize_duration_batch};

        let config = QuantizeConfig::default();
        // Quarter + 3 triplet eighths + quarter
        // Quarter = 480, triplet eighth = 160
        let durations = vec![480, 160, 160, 160, 480];
        let positions = vec![0, 480, 640, 800, 960];

        let quantized = quantize_duration_batch(&durations, &positions, &config);

        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .from_quantized(&quantized, &positions, &config);

        // Should have 5 rhythm entries
        assert_eq!(builder.rhythm.len(), 5);

        // Should have 1 tuplet group (the triplet in the middle)
        assert_eq!(builder.tuplet_groups.len(), 1);
        assert_eq!(builder.tuplet_groups[0].start_idx, 1);
        assert_eq!(builder.tuplet_groups[0].end_idx, 4);
    }

    #[test]
    fn test_from_quantized_no_tuplets() {
        use crate::engraver::quantize::{QuantizeConfig, quantize_duration_batch};

        let config = QuantizeConfig::default();
        // Four regular quarter notes
        let durations = vec![480, 480, 480, 480];
        let positions = vec![0, 480, 960, 1440];

        let quantized = quantize_duration_batch(&durations, &positions, &config);

        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .from_quantized(&quantized, &positions, &config);

        // Should have 4 rhythm entries
        assert_eq!(builder.rhythm.len(), 4);

        // Should have no tuplet groups
        assert!(builder.tuplet_groups.is_empty());

        // All should be quarter notes
        for dur in &builder.rhythm {
            assert_eq!(*dur, Duration::Quarter);
        }
    }

    #[test]
    fn test_from_quantized_reaper_ppq() {
        use crate::engraver::quantize::{QuantizeConfig, quantize_duration_batch};

        // REAPER uses 960 PPQ
        let config = QuantizeConfig::reaper();
        // Three triplet eighths at 960 PPQ (320 ticks each)
        let durations = vec![320, 320, 320];
        let positions = vec![0, 320, 640];

        let quantized = quantize_duration_batch(&durations, &positions, &config);

        let builder = MeasureBuilder::new()
            .time_signature(4, 4)
            .from_quantized(&quantized, &positions, &config);

        // Should detect triplet group
        assert_eq!(builder.tuplet_groups.len(), 1);
        assert_eq!(builder.tuplet_groups[0].ratio, TupletRatio::triplet());
    }
}
