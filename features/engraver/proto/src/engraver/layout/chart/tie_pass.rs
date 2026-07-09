//! Cross-barline tie rendering for rhythm slashes and melody notes.
//!
//! Both passes mirror MuseScore's `Tie` element painted between
//! consecutive `ChordRest` segments. Living here keeps `mod.rs` lean.

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::segment::SegmentType;
use crate::engraver::layout::tlayout::{SlurDirection, SlurEndpoint, SlurTieConfig, layout_tie};
use crate::engraver::notation::MeasureScene;
use crate::engraver::scene::node::SceneNode;

use super::ChartLayoutEngine;
use super::types::{MeasureMelodyData, melody_pitch_to_line_for_clef};

impl ChartLayoutEngine {
    /// Add tie arcs on rhythm-slash heads marked `ChordRhythm::Slashes.tied`.
    ///
    /// For each slash chord whose data carries `tied = true`, drapes a tie
    /// arc from the right side of its last segment to the left side of the
    /// next segment (or just past the measure end when the tie crosses the
    /// barline). Mirrors MuseScore's `Tie` between rhythm-slash chord-rests.
    pub(super) fn add_slash_ties(
        &self,
        measure_result: &mut MeasureScene,
        measure: &crate::chart::types::Measure,
        ctx: &LayoutContext<'_>,
    ) {
        use crate::chord::ChordRhythm;
        let spatium = ctx.spatium();
        let tie_config = SlurTieConfig::default();

        let chord_segments: Vec<_> = measure_result
            .segments
            .iter()
            .filter(|seg| seg.seg_type.contains(SegmentType::CHORD_REST))
            .collect();

        let mut seg_cursor = 0usize;
        let staff_y = 2.0 * spatium;

        for chord in &measure.chords {
            let seg_count = match &chord.rhythm {
                ChordRhythm::Slashes { count, .. } => (*count as usize).max(1),
                _ => 1,
            };
            let tied = matches!(&chord.rhythm, ChordRhythm::Slashes { tied: true, .. });
            let last_seg_idx = seg_cursor + seg_count - 1;

            if tied {
                let from = chord_segments.get(last_seg_idx);
                let to = chord_segments.get(last_seg_idx + 1);
                if let Some(from) = from {
                    let start_x = from.x * spatium + spatium;
                    let end_x = match to {
                        Some(seg) => seg.x * spatium,
                        None => measure_result.width * spatium + spatium * 0.5,
                    };
                    let start = SlurEndpoint {
                        x: start_x,
                        y: staff_y,
                        stem_up: false,
                    };
                    let end = SlurEndpoint {
                        x: end_x,
                        y: staff_y,
                        stem_up: false,
                    };
                    let tie_layout = layout_tie(
                        &start,
                        &end,
                        SlurDirection::Down,
                        3000 + last_seg_idx as u64,
                        spatium,
                        &tie_config,
                    );
                    measure_result
                        .scene
                        .add_child(SceneNode::anonymous_leaf(tie_layout.commands));
                }
            }

            seg_cursor += seg_count;
        }
    }

    /// Add tie arcs for melody notes that cross barlines.
    pub(super) fn add_melody_ties(
        &self,
        measure_result: &mut MeasureScene,
        melody_data: &MeasureMelodyData,
        ctx: &LayoutContext<'_>,
        clef: crate::chart::ChartClef,
    ) {
        let spatium = ctx.spatium();
        let tie_config = SlurTieConfig::default();

        let chord_segments: Vec<_> = measure_result
            .segments
            .iter()
            .filter(|seg| seg.seg_type.contains(SegmentType::CHORD_REST))
            .collect();

        for (i, segment) in melody_data.segments.iter().enumerate() {
            if segment.is_rest {
                continue;
            }

            let note_y = if let Some(oct) = segment.octave {
                let (line, _) = melody_pitch_to_line_for_clef(&segment.pitch, oct, clef);
                -(line as f64) * spatium / 2.0
            } else {
                2.0 * spatium
            };

            if let Some(seg) = chord_segments.get(i) {
                let note_x = seg.x * spatium;

                if segment.tie_to_next {
                    let start = SlurEndpoint {
                        x: note_x + spatium,
                        y: note_y,
                        stem_up: false,
                    };
                    let end = SlurEndpoint {
                        x: measure_result.width * spatium + spatium * 0.5,
                        y: note_y,
                        stem_up: false,
                    };
                    let tie_layout = layout_tie(
                        &start,
                        &end,
                        SlurDirection::Down,
                        1000 + i as u64,
                        spatium,
                        &tie_config,
                    );
                    measure_result
                        .scene
                        .add_child(SceneNode::anonymous_leaf(tie_layout.commands));
                }

                if segment.tie_from_previous {
                    let start = SlurEndpoint {
                        x: -spatium * 0.5,
                        y: note_y,
                        stem_up: false,
                    };
                    let end = SlurEndpoint {
                        x: note_x,
                        y: note_y,
                        stem_up: false,
                    };
                    let tie_layout = layout_tie(
                        &start,
                        &end,
                        SlurDirection::Down,
                        2000 + i as u64,
                        spatium,
                        &tie_config,
                    );
                    measure_result
                        .scene
                        .add_child(SceneNode::anonymous_leaf(tie_layout.commands));
                }
            }
        }
    }
}
