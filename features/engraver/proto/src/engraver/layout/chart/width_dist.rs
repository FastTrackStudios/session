//! Spring-based per-measure width distribution.
//!
//! `compute_chord_min_widths` measures each chord symbol against the
//! current harmony style font, `estimate_measure_content_weight` derives
//! a relative weight for a measure (number of chord segments, beam
//! density, etc.), and `distribute_measure_widths` runs the spring
//! solver from `spacing.rs` to assign final widths. Lives here so the
//! engine impl in `mod.rs` stays focused on the orchestration loops.

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::text_metrics::TextFontMetrics;
use crate::engraver::notation::MeasureScene;
use tracing::debug;

use super::{
    ChartLayoutConfig, ChartLayoutEngine, LayoutMode, chord_layout, constants,
    key_signature_fifths, measure_chart, measure_layout, prefix_renderer, rhythm_builder, spacing,
};
use crate::sections::SectionType;

#[derive(Debug, Clone)]
pub struct MusicXmlWidthComparison {
    pub section_idx: usize,
    pub system_idx: usize,
    pub slot: usize,
    pub measure_idx: usize,
    pub source_measure: Option<u32>,
    pub source_measure_width: Option<f64>,
    pub adjusted_source_body_width: Option<f64>,
    pub assigned_width: f64,
    pub source_body_share: Option<f64>,
    pub assigned_share: f64,
    pub relative_error: Option<f64>,
    pub weight: f64,
    pub min_width: f64,
    pub prefix_xml_units_removed: f64,
}

#[derive(Debug, Clone)]
pub struct MusicXmlWidthComparisonSummary {
    pub rows: Vec<MusicXmlWidthComparison>,
    pub compared: usize,
    pub median_abs_error: Option<f64>,
    pub p90_abs_error: Option<f64>,
    pub max_abs_error: Option<f64>,
}

fn adjust_source_widths_for_prefix(
    source_widths: &[Option<f64>],
    prefix_width: f64,
    assigned_body_width_sum: f64,
) -> Vec<Option<f64>> {
    let source_total = source_widths.iter().flatten().copied().sum::<f64>();
    if source_total <= 0.0 || assigned_body_width_sum <= 0.0 {
        return source_widths.to_vec();
    }

    let xml_units_per_point = source_total / (assigned_body_width_sum + prefix_width).max(1.0);
    let prefix_xml_units = prefix_width * xml_units_per_point;
    let mut adjusted = source_widths.to_vec();
    if let Some(Some(first)) = adjusted.first_mut() {
        *first = (*first - prefix_xml_units).max(1.0);
    }
    adjusted
}

fn summarize_width_comparison(
    rows: Vec<MusicXmlWidthComparison>,
) -> MusicXmlWidthComparisonSummary {
    let mut errors = rows
        .iter()
        .filter_map(|row| row.relative_error.map(f64::abs))
        .filter(|err| err.is_finite())
        .collect::<Vec<_>>();
    errors.sort_by(|a, b| a.total_cmp(b));
    let compared = errors.len();
    let median_abs_error = percentile_sorted(&errors, 0.5);
    let p90_abs_error = percentile_sorted(&errors, 0.9);
    let max_abs_error = errors.last().copied();

    MusicXmlWidthComparisonSummary {
        rows,
        compared,
        median_abs_error,
        p90_abs_error,
        max_abs_error,
    }
}

fn percentile_sorted(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let idx = ((values.len() - 1) as f64 * percentile.clamp(0.0, 1.0)).round() as usize;
    values.get(idx).copied()
}

impl ChartLayoutEngine {
    pub(super) fn has_spacing_expander(
        &self,
        weights: &[f64],
        min_widths: &[f64],
        base_measure_width: f64,
    ) -> bool {
        has_spacing_expander(weights, min_widths, base_measure_width)
    }

    pub fn compare_musicxml_widths(
        &self,
        chart: &crate::chart::Chart,
        mode: &LayoutMode,
        config: &ChartLayoutConfig,
    ) -> MusicXmlWidthComparisonSummary {
        let config_with_settings = config.clone().with_chart_settings(&chart.settings);
        let temp_engine = ChartLayoutEngine {
            config: config_with_settings,
            style: self.style,
            text_font_data: self.text_font_data.clone(),
            symbol_font_data: self.symbol_font_data.clone(),
        };

        let page_width = mode.page_width();
        let content_width =
            page_width - temp_engine.config.margins.left - temp_engine.config.margins.right;
        let mut measurement_cache = super::MeasurementCache::new();
        let chart_measurements = measure_chart(
            chart
                .sections
                .iter()
                .filter(|section| !section.section.section_type.is_compact())
                .filter(|section| !matches!(section.section.section_type, SectionType::End))
                .map(|section| section.measures()),
            &temp_engine.config.harmony_style,
            &mut measurement_cache,
        );
        let text_metrics = TextFontMetrics::new(temp_engine.text_font_data.clone());
        let key_signature = key_signature_fifths(chart);
        let mut rows = Vec::new();
        let mut global_system_index = 0usize;
        let mut global_section_measure_offset = 0usize;

        for (section_idx, chart_section) in chart.sections.iter().enumerate() {
            if chart_section.section.section_type.is_compact()
                || matches!(chart_section.section.section_type, SectionType::End)
            {
                continue;
            }

            let systems =
                temp_engine.group_measures_into_systems(chart_section.measures(), content_width);

            for (system_idx, measure_indices) in systems.iter().enumerate() {
                let include_time_sig = global_system_index == 0;
                let (_, _, _, prefix_width) = prefix_renderer::calculate_prefix_width(
                    temp_engine.config.spatium,
                    true,
                    true,
                    key_signature,
                    include_time_sig,
                );
                let measures_area_width = content_width - prefix_width;
                let max_measures = temp_engine.config.max_measures_per_system;
                let base_measure_width = if temp_engine.config.snippet_mode {
                    spacing::natural_width(
                        constants::TICKS_PER_QUARTER,
                        temp_engine.config.spatium,
                        temp_engine.config.spacing_slope,
                        temp_engine.config.spacing_density,
                        1.0,
                    ) * 4.0
                } else {
                    measures_area_width / max_measures as f64
                };
                let is_short_system =
                    temp_engine.config.snippet_mode || measure_indices.len() < max_measures;

                let (measure_weights, measure_min_widths): (Vec<f64>, Vec<f64>) = measure_indices
                    .iter()
                    .filter_map(|&idx| chart_section.measures().get(idx).map(|m| (idx, m)))
                    .map(|(idx, measure)| {
                        let weight =
                            temp_engine.estimate_measure_content_weight(measure, &text_metrics);
                        let global_idx = global_section_measure_offset + idx;
                        let min_width = chart_measurements
                            .get(global_idx)
                            .map(|m| m.min_width)
                            .unwrap_or(0.0);
                        (weight, min_width)
                    })
                    .unzip();
                let has_spacing_expansion = temp_engine.has_spacing_expander(
                    &measure_weights,
                    &measure_min_widths,
                    base_measure_width,
                );
                let total_width_to_distribute = if temp_engine.config.snippet_mode {
                    measure_indices.len() as f64 * base_measure_width
                } else if is_short_system && has_spacing_expansion {
                    measures_area_width
                } else if is_short_system {
                    measure_indices.len() as f64 * base_measure_width
                } else {
                    measures_area_width
                };
                let assigned_widths = temp_engine.distribute_measure_widths(
                    &measure_weights,
                    0,
                    total_width_to_distribute,
                    0.4,
                    base_measure_width,
                    &measure_min_widths,
                );

                let source_widths = measure_indices
                    .iter()
                    .map(|&idx| {
                        chart_section
                            .measures()
                            .get(idx)
                            .and_then(|m| m.source_measure_width)
                    })
                    .collect::<Vec<_>>();
                let adjusted_source_widths = adjust_source_widths_for_prefix(
                    &source_widths,
                    prefix_width,
                    assigned_widths.iter().sum(),
                );
                let adjusted_sum = adjusted_source_widths
                    .iter()
                    .flatten()
                    .copied()
                    .sum::<f64>();
                let assigned_sum = assigned_widths.iter().sum::<f64>();
                let prefix_xml_units_removed = source_widths
                    .first()
                    .copied()
                    .flatten()
                    .zip(adjusted_source_widths.first().copied().flatten())
                    .map(|(raw, adjusted)| raw - adjusted)
                    .unwrap_or(0.0);

                for (slot, &measure_idx) in measure_indices.iter().enumerate() {
                    let Some(measure) = chart_section.measures().get(measure_idx) else {
                        continue;
                    };
                    let assigned_width = assigned_widths.get(slot).copied().unwrap_or_default();
                    let assigned_share = if assigned_sum > 0.0 {
                        assigned_width / assigned_sum
                    } else {
                        0.0
                    };
                    let source_body_width = adjusted_source_widths.get(slot).copied().flatten();
                    let source_body_share = source_body_width.and_then(|width| {
                        if adjusted_sum > 0.0 {
                            Some(width / adjusted_sum)
                        } else {
                            None
                        }
                    });
                    let relative_error = source_body_share.and_then(|share| {
                        if share > 0.0 {
                            Some((assigned_share - share) / share)
                        } else {
                            None
                        }
                    });

                    rows.push(MusicXmlWidthComparison {
                        section_idx,
                        system_idx,
                        slot,
                        measure_idx,
                        source_measure: measure.source_measure_number,
                        source_measure_width: measure.source_measure_width,
                        adjusted_source_body_width: source_body_width,
                        assigned_width,
                        source_body_share,
                        assigned_share,
                        relative_error,
                        weight: measure_weights.get(slot).copied().unwrap_or_default(),
                        min_width: measure_min_widths.get(slot).copied().unwrap_or_default(),
                        prefix_xml_units_removed: if slot == 0 {
                            prefix_xml_units_removed
                        } else {
                            0.0
                        },
                    });
                }

                global_system_index += 1;
            }

            global_section_measure_offset += chart_section.measures().len();
        }

        summarize_width_comparison(rows)
    }

    pub(super) fn log_system_width_decisions(
        &self,
        section_idx: usize,
        system_idx: usize,
        measure_indices: &[usize],
        measures: &[crate::chart::types::Measure],
        weights: &[f64],
        min_widths: &[f64],
        widths: &[f64],
        total_width_to_distribute: f64,
        base_measure_width: f64,
        is_short_system: bool,
    ) {
        if !tracing::enabled!(
            target: "engraver_proto::engraver::layout::chart::spacing",
            tracing::Level::DEBUG
        ) {
            return;
        }

        let width_sum: f64 = widths.iter().sum();
        debug!(
            target: "engraver_proto::engraver::layout::chart::spacing",
            section_idx,
            system_idx,
            measures = measure_indices.len(),
            total_width_to_distribute,
            assigned_width_sum = width_sum,
            base_measure_width,
            is_short_system,
            "[spacing-system] distributed measure widths"
        );

        for (slot, &measure_idx) in measure_indices.iter().enumerate() {
            let Some(measure) = measures.get(measure_idx) else {
                continue;
            };
            let weight = weights.get(slot).copied().unwrap_or_default();
            let min_width = min_widths.get(slot).copied().unwrap_or_default();
            let width = widths.get(slot).copied().unwrap_or_default();
            let pressure = if width > 0.0 { min_width / width } else { 0.0 };
            let default_width = base_measure_width;
            let width_delta = width - default_width;
            let grow_drivers = spacing_grow_drivers(weights, min_widths, base_measure_width);
            let shrink_due_to = if width_delta < -0.01 {
                grow_drivers.as_str()
            } else {
                ""
            };
            let symbols = measure
                .chords
                .iter()
                .filter(|c| !c.full_symbol.is_empty())
                .map(|c| c.full_symbol.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let beat_buckets = chord_beat_buckets(measure);
            let index_buckets = chord_index_buckets(measure);
            let overlap_buckets = overlapping_chord_beats(measure);

            debug!(
                target: "engraver_proto::engraver::layout::chart::spacing",
                section_idx,
                system_idx,
                slot,
                measure_idx,
                source_measure = ?measure.source_measure_number,
                source_measure_width = ?measure.source_measure_width,
                time_signature = ?measure.time_signature,
                chord_count = measure.chords.len(),
                visible_symbols = measure.chords.iter().filter(|c| !c.full_symbol.is_empty()).count(),
                symbols = %symbols,
                beat_buckets = %beat_buckets,
                index_buckets = %index_buckets,
                overlap_buckets = %overlap_buckets,
                weight,
                min_width,
                assigned_width = width,
                default_width,
                width_delta,
                shrink_due_to,
                floor_pressure = pressure,
                "[spacing-measure] width decision"
            );
        }
    }

    /// Extract ChordRest segment x-positions from a MeasureScene (in spatiums).
    ///
    /// Delegates to [`chord_layout::get_chord_rest_positions`].
    pub(super) fn get_chord_rest_positions(&self, measure_scene: &MeasureScene) -> Vec<f64> {
        chord_layout::get_chord_rest_positions(measure_scene)
    }

    /// Estimate the "content weight" of a measure for spring-based layout.
    ///
    /// Content weight determines how much space a measure should receive relative
    /// to other measures in the same system. Measures with more content (more chords,
    /// longer chord names, more complex rhythms) get higher weights.
    ///
    /// # Arguments
    /// * `measure` - The measure to estimate
    /// * `text_metrics` - Text metrics for measuring chord name widths
    ///
    /// # Returns
    /// A weight value (typically 1.0-3.0) where higher = more space needed
    /// Compute minimum segment widths based on actual chord symbol layout bounds.
    ///
    /// Compute minimum segment widths based on chord symbol collision avoidance.
    ///
    /// This calculates the minimum width each segment needs so that chord symbols
    /// placed above them don't collide with the next chord symbol. By setting
    /// segment minimum widths, the spacing system will allocate enough horizontal
    /// space for chord symbols, and the noteheads will naturally move to accommodate.
    ///
    /// # Arguments
    /// * `measure` - The measure containing chord data
    /// * `num_segments` - Number of rhythm segments in this measure
    /// * `measure_width` - Target measure width in points
    /// * `_ctx` - Layout context
    ///
    /// # Returns
    /// A vector of minimum widths (in spatiums) for each segment index.
    pub(super) fn compute_chord_min_widths(
        &self,
        measure: &crate::chart::types::Measure,
        num_segments: usize,
        measure_width: f64,
        ctx: &LayoutContext<'_>,
        is_boundary: bool,
    ) -> Vec<f64> {
        use crate::chart::types::RhythmElement;

        let spatium = ctx.spatium();
        let mut min_widths = vec![0.0; num_segments];

        // Build list of (segment_index, chord) using cumulative beat positions.
        // For slash notation, a chord with Slashes { count: 2 } occupies 2 segments,
        // so the segment index must be computed from cumulative beat durations,
        // not from the rhythm_elements array index.
        let mut seen_real_chord = false;
        let mut cumulative_beats = 0usize;
        let visible_chords: Vec<_> = measure
            .rhythm_elements
            .iter()
            .filter_map(|elem| {
                if let RhythmElement::Chord(chord) = elem {
                    // Capture this chord's segment position BEFORE adding its duration
                    let seg_idx = cumulative_beats;

                    // Add this chord's beat duration to the running total
                    let chord_beats = match &chord.rhythm {
                        crate::chord::ChordRhythm::Slashes { count, .. } => *count as usize,
                        crate::chord::ChordRhythm::Default => 1,
                        _ => 1,
                    };
                    cumulative_beats += chord_beats;

                    // Skip invisible chords (spaces, rests represented as chords)
                    let is_visible = !chord.full_symbol.is_empty()
                        && chord.full_symbol != "s"
                        && chord.full_symbol != "r";

                    if !is_visible {
                        return None;
                    }

                    // Check if this is a pushed spillback chord (first real chord that's pushed).
                    // Spillback chords render in the PREVIOUS measure. However, at boundaries
                    // (first measure of section/system), they ALSO render in the current measure
                    // to avoid confusion, so we still need to reserve width.
                    let is_pushed = !seen_real_chord
                        && chord
                            .push_pull
                            .as_ref()
                            .is_some_and(|(is_push, _)| *is_push);

                    // Only skip if pushed AND not at a boundary
                    let should_skip_for_spillback = is_pushed && !is_boundary;

                    seen_real_chord = true;

                    if should_skip_for_spillback {
                        debug!(
                            "[chord-min-width] Skipping pushed spillback '{}' - renders in previous measure",
                            chord.full_symbol
                        );
                        return None;
                    }

                    return Some((seg_idx, chord));
                }
                None
            })
            .collect();

        if visible_chords.len() < 2 {
            return min_widths; // No collision possible with 0 or 1 chord
        }

        // Get font metrics for measuring chord symbol widths
        let text_metrics = self.config.harmony_style.text_font_metrics.as_ref();
        let base_font_size = self.config.harmony_style.root_size;
        let min_gap = base_font_size * 0.5; // Minimum gap between chord symbols

        // Estimate segment width assuming equal distribution
        let estimated_segment_width = if num_segments > 0 {
            measure_width / (num_segments as f64)
        } else {
            measure_width
        };

        // Calculate chord widths and required segment widths
        for i in 0..visible_chords.len() - 1 {
            let (idx1, chord1) = visible_chords[i];
            let (idx2, _chord2) = visible_chords[i + 1];

            // Calculate chord width using actual font metrics if available
            let chord1_width = if let Some(metrics) = text_metrics {
                metrics.horizontal_advance(&chord1.full_symbol, base_font_size)
            } else {
                // Fallback estimate: ~0.6 × font_size per character
                chord1.full_symbol.len() as f64 * base_font_size * 0.6
            };
            // Add minimum width floor
            let chord1_width = chord1_width.max(base_font_size * 1.5);

            // Calculate how many segments between these chords
            let segment_gap = idx2.saturating_sub(idx1);
            if segment_gap == 0 {
                continue; // Same segment, can't help here
            }

            // Required space for chord symbol + gap
            let required_space = chord1_width + min_gap;

            // Available space based on current segment widths
            let available_space = segment_gap as f64 * estimated_segment_width;

            // Only set minimum if there would be an actual collision
            if required_space > available_space {
                // Collision deficit = how much the chords would overlap
                let collision_deficit = required_space - available_space;

                // Split the work between segment spacing and left-shifting.
                // First chord (segment 0) can overhang into clef area, so it relies
                // more on movement (70%) and less on spacing (30%).
                // Other chords use 50/50 split.
                let spacing_ratio = if idx1 == 0 { 0.3 } else { 0.5 };
                let spacing_contribution = collision_deficit * spacing_ratio;

                // Add the spacing contribution to the current estimated segment width
                // NOTE: min_width is in POINTS (same units as segment.width)
                let min_width_points = estimated_segment_width + spacing_contribution;

                // Only set if it's larger than the current minimum
                if idx1 < min_widths.len() {
                    min_widths[idx1] = min_widths[idx1].max(min_width_points);
                }

                debug!(
                    "[chord-min-width] Chord '{}' at seg {} collision: deficit={:.1}pt, \
                     spacing contribution={:.1}pt. Setting min_width[{}]={:.1}pt",
                    chord1.full_symbol,
                    idx1,
                    collision_deficit,
                    spacing_contribution,
                    idx1,
                    min_widths[idx1]
                );
            }
        }

        // Set minimum for last segment to prevent notehead overflow into barline.
        // The last notehead needs room to render without crossing the barline.
        let last_segment_padding = spatium * 1.5; // ~1.5 staff spaces for last notehead
        if num_segments > 0 {
            let last_idx = num_segments - 1;
            min_widths[last_idx] = min_widths[last_idx].max(last_segment_padding);
        }

        if min_widths.iter().any(|&w| w > 0.0) {
            debug!("[chord-min-width] Final min_widths (pts): {:?}", min_widths);
        }

        min_widths
    }

    /// Calculate content weight for a measure (for spring-based spacing).
    ///
    /// Weight is based on the actual rhythm elements (after push/pull processing).
    /// We call the real rhythm building functions to get accurate counts,
    /// ensuring weight calculation matches rendering.
    ///
    /// Triplets receive extra weight because they require bracket notation
    /// (└3┘) which needs horizontal space for visual clarity.
    ///
    /// # Note
    ///
    /// Chord collision handling is now done via `min_width` from the measurement
    /// cache (Pass 1), which acts as a hard constraint in the spring system.
    /// This eliminates the need for heuristic collision penalties in the weight
    /// calculation.
    pub(super) fn estimate_measure_content_weight(
        &self,
        measure: &crate::chart::types::Measure,
        text_metrics: &TextFontMetrics,
    ) -> f64 {
        // MuseScore-style content weight: derive the measure's "want" from
        // rhythmic density, not from how many chord symbols happen to be
        // printed above the staff. Chord symbols still contribute hard
        // minimum widths through the measurement pass, but they should not
        // make two bars with the same rhythm receive wildly different width.
        let config = rhythm_builder::RhythmBuildConfig {
            time_signature: measure.time_signature,
            ..Default::default()
        };
        let source = if measure.has_explicit_rhythm() {
            rhythm_builder::RhythmSource::ExplicitRhythm {
                elements: &measure.rhythm_elements,
                spillbacks: None,
            }
        } else {
            rhythm_builder::RhythmSource::SlashNotation {
                chords: &measure.chords,
                spillbacks: None,
            }
        };
        let rhythm_result = rhythm_builder::build_rhythm(source, &config);

        let slope = self.config.spacing_slope;
        let visible_symbol_count = measure
            .chords
            .iter()
            .filter(|c| !c.full_symbol.is_empty())
            .count();
        let is_written_rest_measure = visible_symbol_count == 0
            && !measure.melodies.is_empty()
            && measure
                .melodies
                .iter()
                .flat_map(|melody| melody.notes.iter())
                .all(|note| note.pitch == "r");
        let is_empty_measure = visible_symbol_count == 0
            && measure.chords.is_empty()
            && measure.melodies.is_empty()
            && !measure.has_explicit_rhythm();
        let is_sparse_rest_measure = is_written_rest_measure || is_empty_measure;
        let duration_weight: f64 = if is_sparse_rest_measure {
            let measure_ticks = measure_duration_ticks(measure.time_signature);
            spacing::duration_stretch(measure_ticks, constants::TICKS_PER_QUARTER, slope)
        } else if measure
            .melodies
            .iter()
            .any(|melody| !melody.notes.is_empty())
        {
            melody_density_weight(measure, slope)
        } else {
            // Chord-only (slash) bar: size it by the whole-measure duration, not
            // by summing per-chord slash durations. Summing makes a 2+1+1 bar
            // sprawl ~1.8x wider than a 2+2 bar purely because it has more
            // slashes, even though the chord symbols fit comfortably. Chord
            // symbols still reserve non-overlapping room via the min-width pass,
            // so a bar only grows past base width when its chords actually need
            // it — not from slash count.
            let measure_ticks = measure_duration_ticks(measure.time_signature);
            spacing::duration_stretch(measure_ticks, constants::TICKS_PER_QUARTER, slope)
        };
        let durations = rhythm_result
            .entries
            .iter()
            .map(|e| e.duration().ticks().to_string())
            .collect::<Vec<_>>()
            .join(",");

        // Triplet brackets still need a small extra cushion on top.
        let triplet_count: usize = rhythm_result
            .tuplet_specs
            .iter()
            .map(|spec| spec.end_idx.saturating_sub(spec.start_idx))
            .sum();
        let triplet_bonus = triplet_count as f64 * 0.08;

        let _ = text_metrics;
        let symbol_bonus = 0.0;

        let weight = (duration_weight + triplet_bonus + symbol_bonus).max(0.5);
        debug!(
            target: "engraver_proto::engraver::layout::chart::spacing",
            source_measure = ?measure.source_measure_number,
            time_signature = ?measure.time_signature,
            entries = rhythm_result.entries.len(),
            durations_ticks = %durations,
            chord_count = measure.chords.len(),
            visible_symbols = visible_symbol_count,
            is_written_rest_measure,
            is_empty_measure,
            beat_buckets = %chord_beat_buckets(measure),
            index_buckets = %chord_index_buckets(measure),
            overlap_buckets = %overlapping_chord_beats(measure),
            duration_weight,
            triplet_bonus,
            symbol_bonus,
            weight,
            "[spacing-weight] measure content weight"
        );

        weight
    }

    /// Distribute available width among measures using spring physics.
    ///
    /// Uses spring-based distribution when stretch values are available,
    /// otherwise falls back to weight-proportional distribution.
    pub(super) fn distribute_measure_widths(
        &self,
        weights: &[f64],
        count_in_measures: usize,
        total_width: f64,
        compact_scale: f64,
        base_measure_width: f64,
        min_widths: &[f64],
    ) -> Vec<f64> {
        let stretches = expansion_gated_stretches(weights, min_widths, base_measure_width);
        let widths = measure_layout::distribute_measure_widths_spring(
            &stretches,
            count_in_measures,
            total_width,
            compact_scale,
            base_measure_width,
            min_widths,
            self.config.spatium,
            self.config.spacing_slope,
            self.config.spacing_density,
        );

        debug!(
            target: "engraver_proto::engraver::layout::chart::spacing",
            measure_count = weights.len(),
            count_in_measures,
            total_width,
            compact_scale,
            base_measure_width,
            weights = ?weights,
            stretches = ?stretches,
            grow_drivers = %spacing_grow_drivers(weights, min_widths, base_measure_width),
            min_widths = ?min_widths,
            widths = ?widths,
            "[spacing-distribute] spring distribution"
        );

        widths
    }
}

fn measure_duration_ticks(time_signature: (u8, u8)) -> f64 {
    let (beats, denominator) = time_signature;
    if denominator == 0 {
        return constants::TICKS_PER_QUARTER;
    }

    f64::from(beats) * constants::TICKS_PER_QUARTER * 4.0 / f64::from(denominator)
}

fn expansion_gated_stretches(
    weights: &[f64],
    min_widths: &[f64],
    base_measure_width: f64,
) -> Vec<f64> {
    const RHYTHM_EXPANSION_THRESHOLD: f64 = 3.0;

    let has_expander = has_spacing_expander(weights, min_widths, base_measure_width);

    if !has_expander {
        return vec![1.0; weights.len()];
    }

    weights
        .iter()
        .enumerate()
        .map(|(idx, weight)| {
            let min_width = min_widths.get(idx).copied().unwrap_or_default();
            if *weight >= RHYTHM_EXPANSION_THRESHOLD || min_width > base_measure_width * 1.05 {
                let raw_stretch = (*weight)
                    .max(min_width / base_measure_width.max(1.0))
                    .max(1.0);
                raw_stretch.sqrt()
            } else {
                1.0
            }
        })
        .collect()
}

fn has_spacing_expander(weights: &[f64], min_widths: &[f64], base_measure_width: f64) -> bool {
    const RHYTHM_EXPANSION_THRESHOLD: f64 = 3.0;

    weights.iter().enumerate().any(|(idx, weight)| {
        *weight >= RHYTHM_EXPANSION_THRESHOLD
            || min_widths.get(idx).copied().unwrap_or_default() > base_measure_width * 1.05
    })
}

fn spacing_grow_drivers(weights: &[f64], min_widths: &[f64], base_measure_width: f64) -> String {
    const RHYTHM_EXPANSION_THRESHOLD: f64 = 3.0;

    weights
        .iter()
        .enumerate()
        .filter_map(|(idx, weight)| {
            let min_width = min_widths.get(idx).copied().unwrap_or_default();
            let rhythm_driver = *weight >= RHYTHM_EXPANSION_THRESHOLD;
            let min_width_driver = min_width > base_measure_width * 1.05;
            if rhythm_driver || min_width_driver {
                Some(format!(
                    "slot{idx}:{}{}",
                    if rhythm_driver { "rhythm" } else { "" },
                    if min_width_driver { "+min" } else { "" }
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn melody_note_ticks(note: &crate::chart::melody::MelodyNote) -> f64 {
    (note.duration_beats() * constants::TICKS_PER_QUARTER).max(1.0)
}

fn melody_density_weight(measure: &crate::chart::types::Measure, slope: f64) -> f64 {
    let note_ticks = measure
        .melodies
        .iter()
        .flat_map(|melody| melody.notes.iter())
        .filter(|note| note.pitch != "r")
        .map(melody_note_ticks)
        .collect::<Vec<_>>();

    if note_ticks.is_empty() {
        return spacing::duration_stretch(
            measure_duration_ticks(measure.time_signature),
            constants::TICKS_PER_QUARTER,
            slope,
        );
    }

    let visible_symbol_count = measure
        .chords
        .iter()
        .filter(|chord| !chord.full_symbol.is_empty())
        .count();
    let uniform_eighth_grid = measure.time_signature.1 == 8
        && note_ticks.len() == measure.time_signature.0 as usize
        && note_ticks
            .iter()
            .all(|ticks| (*ticks - constants::TICKS_PER_QUARTER / 2.0).abs() < f64::EPSILON);
    if uniform_eighth_grid && visible_symbol_count <= 1 {
        return spacing::duration_stretch(
            measure_duration_ticks(measure.time_signature),
            constants::TICKS_PER_QUARTER,
            slope,
        );
    }

    let shortest_ticks = note_ticks.iter().copied().fold(f64::INFINITY, f64::min);
    let dense_enough_for_extra_space =
        note_ticks.len() >= 4 || shortest_ticks <= constants::TICKS_PER_QUARTER / 2.0;

    if !dense_enough_for_extra_space {
        return spacing::duration_stretch(
            measure_duration_ticks(measure.time_signature),
            constants::TICKS_PER_QUARTER,
            slope,
        );
    }

    note_ticks
        .iter()
        .map(|ticks| spacing::duration_stretch(*ticks, constants::TICKS_PER_QUARTER, slope))
        .sum()
}

fn chord_beat_buckets(measure: &crate::chart::types::Measure) -> String {
    measure
        .chords
        .iter()
        .filter(|c| !c.full_symbol.is_empty())
        .map(|c| {
            let beat = c.position.beats() as f64 + c.position.subdivisions() as f64 / 1000.0 + 1.0;
            format!("{beat:.2}:{}", c.full_symbol)
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn chord_index_buckets(measure: &crate::chart::types::Measure) -> String {
    measure
        .chords
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.full_symbol.is_empty())
        .map(|(idx, c)| format!("{idx}:{}", c.full_symbol))
        .collect::<Vec<_>>()
        .join(",")
}

fn overlapping_chord_beats(measure: &crate::chart::types::Measure) -> String {
    let mut buckets: std::collections::BTreeMap<(u32, u32), Vec<&str>> =
        std::collections::BTreeMap::new();
    for chord in measure.chords.iter().filter(|c| !c.full_symbol.is_empty()) {
        buckets
            .entry((chord.position.beats(), chord.position.subdivisions()))
            .or_default()
            .push(chord.full_symbol.as_str());
    }

    buckets
        .into_iter()
        .filter(|(_, symbols)| symbols.len() > 1)
        .map(|((beat, subdivisions), symbols)| {
            let beat = beat as f64 + subdivisions as f64 / 1000.0 + 1.0;
            format!("{beat:.2}:{}", symbols.join("+"))
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::engraver::layout::chart::measure_pass::{MeasurementCache, measure_measure};
    use crate::engraver::layout::text_metrics::TextFontMetrics;
    use crate::engraver::layout::tlayout::HarmonyStyle;
    use crate::engraver::style::MStyle;

    fn test_engine() -> ChartLayoutEngine {
        let style = Box::leak(Box::new(MStyle::default()));
        let font_data = Arc::new(crate::engraver::fonts::EMPTY_FONT_DATA_FOR_TESTS.to_vec());
        ChartLayoutEngine::new(style, font_data.clone(), font_data)
    }

    fn test_harmony_style() -> HarmonyStyle {
        let font_data = Arc::new(crate::engraver::fonts::EMPTY_FONT_DATA_FOR_TESTS.to_vec());
        HarmonyStyle::default().with_text_font_metrics(TextFontMetrics::new(font_data))
    }

    #[test]
    fn written_rest_weight_is_sparser_than_visible_chord_content() {
        let mut fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fixture.push("../../../crates/keyflow");
        fixture.push("examples/png-project-charts/02 LORD OF THE FIGHT Master RS.musicxml");
        let chart = keyflow_musicxml::import_file(fixture).expect("LotF should import");
        let measures: Vec<&crate::chart::types::Measure> = chart
            .sections
            .iter()
            .flat_map(|section| section.tracks.iter())
            .flat_map(|track| track.measures.iter())
            .collect();
        let by_source_number = |n: u32| -> &crate::chart::types::Measure {
            measures
                .iter()
                .copied()
                .find(|m| m.source_measure_number == Some(n))
                .unwrap_or_else(|| panic!("missing source measure {n}"))
        };

        let engine = test_engine();
        let text_metrics = TextFontMetrics::new(Arc::new(
            crate::engraver::fonts::EMPTY_FONT_DATA_FOR_TESTS.to_vec(),
        ));
        let m3 = by_source_number(3);
        let m4 = by_source_number(4);
        let m5 = by_source_number(5);
        let m6 = by_source_number(6);
        let weights = [
            engine.estimate_measure_content_weight(m3, &text_metrics),
            engine.estimate_measure_content_weight(m4, &text_metrics),
            engine.estimate_measure_content_weight(m5, &text_metrics),
            engine.estimate_measure_content_weight(m6, &text_metrics),
        ];

        let harmony_style = test_harmony_style();
        let mut cache = MeasurementCache::new();
        let min_widths = [
            measure_measure(m3, &harmony_style, &mut cache).min_width,
            measure_measure(m4, &harmony_style, &mut cache).min_width,
            measure_measure(m5, &harmony_style, &mut cache).min_width,
            measure_measure(m6, &harmony_style, &mut cache).min_width,
        ];
        let widths = engine.distribute_measure_widths(&weights, 0, 420.0, 0.5, 100.0, &min_widths);

        assert!(
            weights[2] > weights[0] && weights[2] > weights[1],
            "visible chord measure should want more space than written-rest measures: weights={weights:?}"
        );
        assert!(
            widths[2] > widths[0] && widths[2] > widths[1],
            "m5 should distribute wider than m3/m4 written rests: widths={widths:?} weights={weights:?} min_widths={min_widths:?}"
        );
        assert!(
            (widths[3] - widths[2]).abs() / widths[2] < 0.15,
            "m5 and m6 have the same melody rhythm and should distribute similarly despite different chord counts: widths={widths:?}"
        );

        let simple_line = [7_u32, 8, 9, 10]
            .iter()
            .map(|source| by_source_number(*source))
            .collect::<Vec<_>>();
        let simple_weights = simple_line
            .iter()
            .map(|measure| engine.estimate_measure_content_weight(measure, &text_metrics))
            .collect::<Vec<_>>();
        assert!(
            simple_weights
                .iter()
                .all(|weight| (*weight - simple_weights[0]).abs() < 0.001),
            "one- and two-event measures should stay at default rhythmic width until density crosses the threshold: {simple_weights:?}"
        );
    }

    #[test]
    fn lotf_default_measures_do_not_shrink_without_same_line_expansion_pressure() {
        let mut fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fixture.push("../../../crates/keyflow");
        fixture.push("examples/png-project-charts/02 LORD OF THE FIGHT Master RS.musicxml");
        let chart = keyflow_musicxml::import_file(fixture).expect("LotF should import");
        let engine = test_engine();
        let report = engine.compare_musicxml_widths(
            &chart,
            &LayoutMode::paginated_a4(),
            &ChartLayoutConfig::master_rhythm().with_page_offsets(false),
        );
        let by_source = |source: u32| -> &MusicXmlWidthComparison {
            report
                .rows
                .iter()
                .find(|row| row.source_measure == Some(source))
                .unwrap_or_else(|| panic!("missing source measure {source}"))
        };

        let m15 = by_source(15).assigned_width;
        for source in [16_u32, 17, 18] {
            let width = by_source(source).assigned_width;
            assert!(
                (width - m15).abs() < 0.1,
                "m15-m18 should stay at default equal widths unless the line has an expansion driver; m15={m15} m{source}={width}"
            );
        }

        let m55 = by_source(55);
        assert!(
            m55.weight < 3.0,
            "six ordinary eighth-notes in 6/8 should remain baseline, not force expansion: {m55:?}"
        );
    }

    #[test]
    fn lotf_opening_dense_measures_are_capped_against_written_rests() {
        let mut fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        fixture.push("../../../crates/keyflow");
        fixture.push("examples/png-project-charts/02 LORD OF THE FIGHT Master RS.musicxml");
        let chart = keyflow_musicxml::import_file(fixture).expect("LotF should import");
        let engine = test_engine();
        let report = engine.compare_musicxml_widths(
            &chart,
            &LayoutMode::paginated_a4(),
            &ChartLayoutConfig::master_rhythm().with_page_offsets(false),
        );
        let by_source = |source: u32| -> &MusicXmlWidthComparison {
            report
                .rows
                .iter()
                .find(|row| row.source_measure == Some(source))
                .unwrap_or_else(|| panic!("missing source measure {source}"))
        };

        let rest_width = (by_source(3).assigned_width + by_source(4).assigned_width) / 2.0;
        let dense_width = (by_source(5).assigned_width + by_source(6).assigned_width) / 2.0;
        assert!(
            dense_width <= rest_width * 2.0 + 0.1,
            "m5/m6 should be capped near 2x m3/m4, got dense={dense_width} rest={rest_width} ratio={}",
            dense_width / rest_width
        );
        assert!(
            by_source(6).assigned_width >= by_source(6).min_width,
            "cap must not violate m6 chord-symbol minimum width: {:?}",
            by_source(6)
        );
    }
}
