//! Leaf helpers for [`ChartLayoutEngine`].
//!
//! Section labels, measure numbers, count-in text, barline + page-frame
//! helpers — anything that's a one- or two-line delegate to another
//! module, or a small piece of self-contained geometry. Splits the
//! engine impl into something readable without changing behaviour.

use kurbo::Point;
use peniko::Color;

use crate::Chart;
use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::tlayout::{
    BarlineType, ClefType, MarginLabelParams, layout_margin_label,
    rehearsal_mark::RehearsalMarkStyle,
};
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;
use crate::sections::SectionType;

use super::{ChartLayoutEngine, count_in_renderer, page_rendering, section_layout};

impl ChartLayoutEngine {
    /// Create a section label scene node.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_section_label(
        &self,
        section: &crate::sections::Section,
        page_x: f64,
        margin_width: f64,
        staff_y: f64,
        staff_height: f64,
        letter: Option<char>,
        ctx: &LayoutContext<'_>,
        id: u64,
    ) -> SceneNode {
        let mut container = SceneNode::group(SemanticId::new(ElementType::RehearsalMark, id));
        if let Some(label_text) = section.metadata.get("repeat_pass.labels") {
            container
                .metadata
                .insert("section_label_text".to_string(), label_text.clone());
            let mut y = staff_y;
            let pass_gap = repeat_pass_label_gap(staff_height);
            for pass_label in repeat_pass_label_parts(label_text) {
                let section_type = section_type_for_pass_label(pass_label)
                    .unwrap_or_else(|| section.section_type.clone());
                let (section_type_name, abbreviation) = self.section_type_to_strings(&section_type);
                let (layout, label_node) = layout_margin_label(
                    &MarginLabelParams {
                        section_type: section_type_name,
                        abbreviation,
                        number: None,
                        letter: None,
                        comment: None,
                        label_override: Some(pass_label.to_string()),
                        page_x,
                        margin_width,
                        staff_y: y,
                        staff_height,
                        style: self.get_section_theme(&section_type),
                        ..Default::default()
                    },
                    ctx,
                );
                container.add_child(label_node);
                y += layout.height + pass_gap;
            }
            return container;
        }

        let (section_type, abbreviation) = self.section_type_to_strings(&section.section_type);
        let (_, label_node) = layout_margin_label(
            &MarginLabelParams {
                section_type,
                abbreviation,
                number: section.number,
                letter,
                comment: section.comment.clone(),
                page_x,
                margin_width,
                staff_y,
                staff_height,
                style: self.get_section_theme(&section.section_type),
                ..Default::default()
            },
            ctx,
        );
        container.add_child(label_node);
        container
    }

    pub(super) fn repeat_pass_dynamic_slots(
        &self,
        section: &crate::sections::Section,
        page_x: f64,
        margin_width: f64,
        staff_y: f64,
        staff_height: f64,
        ctx: &LayoutContext<'_>,
    ) -> Vec<f64> {
        let Some(label_text) = section.metadata.get("repeat_pass.labels") else {
            return Vec::new();
        };

        let mut y = staff_y;
        let pass_gap = repeat_pass_label_gap(staff_height);
        let mut slots = Vec::new();
        for pass_label in repeat_pass_label_parts(label_text) {
            let section_type = section_type_for_pass_label(pass_label)
                .unwrap_or_else(|| section.section_type.clone());
            let (section_type_name, abbreviation) = self.section_type_to_strings(&section_type);
            let (layout, _) = layout_margin_label(
                &MarginLabelParams {
                    section_type: section_type_name,
                    abbreviation,
                    number: None,
                    letter: None,
                    comment: None,
                    label_override: Some(pass_label.to_string()),
                    page_x,
                    margin_width,
                    staff_y: y,
                    staff_height,
                    style: self.get_section_theme(&section_type),
                    ..Default::default()
                },
                ctx,
            );
            slots.push(y + layout.height + pass_gap * 0.72);
            y += layout.height + pass_gap;
        }
        slots
    }

    /// Create a measure number scene node.
    ///
    /// Renders the measure number above the staff, positioned at the start
    /// of the measure. Based on MuseScore's measure number placement:
    /// left-aligned to the visible staff start, 2 spatiums above staff.
    pub(super) fn create_measure_number(
        &self,
        measure_number: i32,
        measure_x: f64,
        staff_y: f64,
        id: u64,
    ) -> SceneNode {
        let spatium = self.config.spatium;
        let y_offset = -2.0 * spatium;
        let font_size = spatium * 1.6;

        let text_command = PaintCommand::text(
            measure_number.to_string(),
            "FreeSans",
            font_size,
            Point::new(measure_x, staff_y + y_offset),
            Color::from_rgb8(100, 100, 100),
        );

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::MeasureNumber, id),
            vec![text_command],
        );
        node.metadata
            .insert("measure_number".to_string(), measure_number.to_string());
        node
    }

    /// Compute section letters for consecutive repeats of the same section.
    pub(super) fn compute_section_letters(
        &self,
        sections: &[crate::ChartSection],
    ) -> std::collections::HashMap<usize, char> {
        section_layout::compute_section_letters(sections)
    }

    /// Map the proto-side ChartClef onto the engraver's ClefType. Defaults
    /// to Bass for charts that don't declare a clef.
    pub(super) fn chart_clef_for(&self, chart: &Chart) -> ClefType {
        use crate::chart::ChartClef;
        match chart.initial_clef {
            // No clef declared → bass by default.
            Some(ChartClef::Bass) | None => ClefType::Bass,
            Some(ChartClef::Alto) => ClefType::Alto,
            Some(ChartClef::Tenor) => ClefType::Tenor,
            // Percussion has no SMuFL clef glyph in our font set yet —
            // fall back to Treble so prefix layout still measures sensibly.
            Some(ChartClef::Treble) | Some(ChartClef::Percussion) => ClefType::Treble,
        }
    }

    /// Proto-side clef (untranslated) for callers that need to feed
    /// `melody_pitch_to_line_for_clef` or other pitch-mapping code.
    pub(super) fn chart_proto_clef_for(&self, chart: &Chart) -> crate::chart::ChartClef {
        chart.initial_clef.unwrap_or(crate::chart::ChartClef::Bass)
    }

    pub(super) fn section_type_to_strings(&self, section_type: &SectionType) -> (String, String) {
        (section_type.full_name(), section_type.abbreviation())
    }

    pub(super) fn get_section_theme(&self, section_type: &SectionType) -> RehearsalMarkStyle {
        section_layout::get_section_theme(section_type)
    }

    pub(super) fn draw_staff_lines(&self, x: f64, y: f64, width: f64) -> Vec<PaintCommand> {
        page_rendering::draw_staff_lines(x, y, width, self.config.spatium)
    }

    pub(super) fn draw_barline(
        &self,
        x: f64,
        y: f64,
        height: f64,
        barline_type: BarlineType,
    ) -> SceneNode {
        page_rendering::draw_barline(x, y, height, barline_type, self.config.spatium)
    }

    /// Pick the [`BarlineType`] for the line that *closes* a measure, given
    /// its style + end-repeat decoration. End-repeat wins over stylistic
    /// choices (matches MuseScore's BarLineType precedence).
    pub(super) fn end_barline_type(measure: &crate::chart::types::Measure) -> BarlineType {
        use crate::chart::notations::{BarlineStyle, RepeatMark};
        match measure.end_repeat {
            RepeatMark::Backward => return BarlineType::EndRepeat,
            RepeatMark::Forward | RepeatMark::None => {}
        }
        match measure.end_barline {
            BarlineStyle::LightHeavy | BarlineStyle::HeavyHeavy => BarlineType::End,
            BarlineStyle::LightLight => BarlineType::Double,
            BarlineStyle::Dashed => BarlineType::Dashed,
            BarlineStyle::HeavyLight | BarlineStyle::Normal | BarlineStyle::None => {
                BarlineType::Single
            }
        }
    }

    pub(super) fn add_page_background(
        &self,
        root: &mut SceneNode,
        page_x: f64,
        page_y: f64,
        page_width: f64,
        page_height: f64,
    ) {
        if self.config.snippet_mode || !self.config.use_page_offsets {
            page_rendering::add_snippet_background(root, page_x, page_y, page_width, page_height);
        } else {
            page_rendering::add_page_background(root, page_x, page_y, page_width, page_height);
        }
    }

    /// Add page footer with "Created with FastTrackStudio" text. Skipped
    /// for titleless charts (snippets).
    pub(super) fn add_page_footer(
        &self,
        root: &mut SceneNode,
        page_x: f64,
        page_y: f64,
        page_width: f64,
        page_height: f64,
        metadata: &crate::SongMetadata,
    ) {
        if metadata.title.is_none() {
            return;
        }
        page_rendering::add_page_footer(root, page_x, page_y, page_width, page_height);
    }

    /// Title header on first page (title, subtitle, composer, tempo, count-in snippet).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_title_header(
        &self,
        root: &mut SceneNode,
        page_x: f64,
        page_y: f64,
        page_width: f64,
        metadata: &crate::SongMetadata,
        tempo: Option<&crate::time::Tempo>,
        count_in_measures: usize,
        time_signature: (u8, u8),
        has_pushed_first_chord: bool,
        count_in_labels: Vec<String>,
    ) -> (f64, Vec<count_in_renderer::CountInBeatGeometry>) {
        let count_in_config = if count_in_measures > 0 {
            Some(page_rendering::CountInHeaderConfig {
                num_measures: count_in_measures,
                beats_per_measure: time_signature.0,
                beat_unit: time_signature.1,
                has_pushed_chord: has_pushed_first_chord,
                measure_numbers: count_in_labels,
            })
        } else {
            None
        };

        page_rendering::add_title_header_with_count_in(page_rendering::TitleHeaderParams {
            root,
            page_x,
            page_y,
            page_width,
            margins: &self.config.margins,
            spatium: self.config.spatium,
            metadata,
            tempo,
            count_in: count_in_config.as_ref(),
        })
    }
}

fn repeat_pass_label_parts(labels: &str) -> impl Iterator<Item = &str> {
    labels
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

fn repeat_pass_label_gap(staff_height: f64) -> f64 {
    staff_height * 0.65
}

fn section_type_for_pass_label(label: &str) -> Option<SectionType> {
    let prefix = label.split_whitespace().next()?;
    SectionType::parse(prefix).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::context::{LayoutConfiguration, LayoutContext};
    use crate::engraver::style::MStyle;
    use crate::sections::Section;
    use std::sync::Arc;

    fn make_engine() -> ChartLayoutEngine {
        let style = Box::leak(Box::new(MStyle::default()));
        ChartLayoutEngine::new(style, Arc::new(Vec::new()), Arc::new(Vec::new()))
    }

    fn make_ctx() -> LayoutContext<'static> {
        let style = Box::leak(Box::new(MStyle::default()));
        LayoutContext::new_for_test(LayoutConfiguration::default(), style)
    }

    fn collect_text(node: &SceneNode, out: &mut Vec<String>) {
        for command in &node.commands {
            if let PaintCommand::Text { text, .. } = command {
                out.push(text.clone());
            }
        }
        for child in &node.children {
            collect_text(child, out);
        }
    }

    #[test]
    fn repeat_pass_labels_render_as_separate_margin_cards() {
        let engine = make_engine();
        let ctx = make_ctx();
        let mut section = Section::new(SectionType::Chorus);
        section.set_metadata("repeat_pass.labels", "CH 1\nCH 2");

        let node = engine.create_section_label(&section, 0.0, 72.0, 100.0, 20.0, None, &ctx, 1);

        assert_eq!(
            node.children.len(),
            2,
            "each repeat pass should get its own card"
        );
        let mut text = Vec::new();
        collect_text(&node, &mut text);
        assert_eq!(text, vec!["CH 1", "CH 2"]);
    }

    #[test]
    fn measure_numbers_align_to_staff_start() {
        let engine = make_engine();
        let node = engine.create_measure_number(19, 144.0, 80.0, 1);

        let [PaintCommand::Text { position, .. }] = node.commands.as_slice() else {
            panic!("measure number should render as text");
        };
        assert_eq!(
            position.x, 144.0,
            "measure number should not shift left into the section-card lane"
        );
    }
}
