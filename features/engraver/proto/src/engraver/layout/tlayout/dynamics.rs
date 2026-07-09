//! Dynamics layout implementation.
//!
//! Handles layout of dynamic markings (pp, p, mp, mf, f, ff, etc.),
//! including positioning relative to notes and staff.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for dynamics.
pub mod glyphs {
    /// Piano (p)
    pub const DYNAMIC_PIANO: char = '\u{E520}';
    /// Mezzo (m)
    pub const DYNAMIC_MEZZO: char = '\u{E521}';
    /// Forte (f)
    pub const DYNAMIC_FORTE: char = '\u{E522}';
    /// Rinforzando (r)
    pub const DYNAMIC_RINFORZANDO: char = '\u{E523}';
    /// Sforzando (s)
    pub const DYNAMIC_SFORZANDO: char = '\u{E524}';
    /// Z (for sfz, fz)
    pub const DYNAMIC_Z: char = '\u{E525}';
    /// Niente (n) - circle for dynamics
    pub const DYNAMIC_NIENTE: char = '\u{E526}';

    // Combined dynamics (pre-composed)
    /// Pianississimo (ppp)
    pub const DYNAMIC_PPP: char = '\u{E52A}';
    /// Pianissimo (pp)
    pub const DYNAMIC_PP: char = '\u{E52B}';
    /// Mezzo-piano (mp)
    pub const DYNAMIC_MP: char = '\u{E52C}';
    /// Mezzo-forte (mf)
    pub const DYNAMIC_MF: char = '\u{E52D}';
    /// Forte-piano (fp)
    pub const DYNAMIC_FP: char = '\u{E534}';
    /// Fortissimo (ff)
    pub const DYNAMIC_FF: char = '\u{E52F}';
    /// Fortississimo (fff)
    pub const DYNAMIC_FFF: char = '\u{E530}';
    /// Sforzando (sfz)
    pub const DYNAMIC_SFZ: char = '\u{E539}';
    /// Sforzato-piano (sfp)
    pub const DYNAMIC_SFP: char = '\u{E537}';
    /// Sforzatissimo (sffz)
    pub const DYNAMIC_SFFZ: char = '\u{E53B}';
    /// Forzando (fz)
    pub const DYNAMIC_FZ: char = '\u{E535}';
    /// Rinforzando (rf)
    pub const DYNAMIC_RF: char = '\u{E53C}';
    /// Rinforzando forte (rfz)
    pub const DYNAMIC_RFZ: char = '\u{E53D}';
}

/// Dynamic type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicType {
    /// Pianississimo (ppp)
    Ppp,
    /// Pianissimo (pp)
    Pp,
    /// Piano (p)
    P,
    /// Mezzo-piano (mp)
    Mp,
    /// Mezzo-forte (mf)
    #[default]
    Mf,
    /// Forte (f)
    F,
    /// Fortissimo (ff)
    Ff,
    /// Fortississimo (fff)
    Fff,
    /// Sforzando (sfz)
    Sfz,
    /// Sforzato-piano (sfp)
    Sfp,
    /// Forzando (fz)
    Fz,
    /// Forte-piano (fp)
    Fp,
    /// Rinforzando (rf)
    Rf,
    /// Rinforzando forte (rfz)
    Rfz,
    /// Sforzatissimo (sffz)
    Sffz,
    /// Custom text dynamic
    Other,
}

impl DynamicType {
    /// Get the SMuFL glyph for this dynamic.
    #[must_use]
    pub const fn glyph(&self) -> Option<char> {
        match self {
            Self::Ppp => Some(glyphs::DYNAMIC_PPP),
            Self::Pp => Some(glyphs::DYNAMIC_PP),
            Self::P => Some(glyphs::DYNAMIC_PIANO),
            Self::Mp => Some(glyphs::DYNAMIC_MP),
            Self::Mf => Some(glyphs::DYNAMIC_MF),
            Self::F => Some(glyphs::DYNAMIC_FORTE),
            Self::Ff => Some(glyphs::DYNAMIC_FF),
            Self::Fff => Some(glyphs::DYNAMIC_FFF),
            Self::Sfz => Some(glyphs::DYNAMIC_SFZ),
            Self::Sfp => Some(glyphs::DYNAMIC_SFP),
            Self::Fz => Some(glyphs::DYNAMIC_FZ),
            Self::Fp => Some(glyphs::DYNAMIC_FP),
            Self::Rf => Some(glyphs::DYNAMIC_RF),
            Self::Rfz => Some(glyphs::DYNAMIC_RFZ),
            Self::Sffz => Some(glyphs::DYNAMIC_SFFZ),
            Self::Other => None,
        }
    }

    /// Get the display text for this dynamic.
    #[must_use]
    pub const fn text(&self) -> &'static str {
        match self {
            Self::Ppp => "ppp",
            Self::Pp => "pp",
            Self::P => "p",
            Self::Mp => "mp",
            Self::Mf => "mf",
            Self::F => "f",
            Self::Ff => "ff",
            Self::Fff => "fff",
            Self::Sfz => "sfz",
            Self::Sfp => "sfp",
            Self::Fz => "fz",
            Self::Fp => "fp",
            Self::Rf => "rf",
            Self::Rfz => "rfz",
            Self::Sffz => "sffz",
            Self::Other => "",
        }
    }

    /// Get the approximate width in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::Ppp | Self::Fff | Self::Sffz => 2.8,
            Self::Pp | Self::Ff | Self::Sfz | Self::Sfp | Self::Rfz => 2.0,
            Self::Mp | Self::Mf | Self::Fp | Self::Rf | Self::Fz => 1.6,
            Self::P | Self::F => 0.9,
            Self::Other => 1.0,
        }
    }
}

/// Dynamics placement relative to staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicsPlacement {
    /// Below the staff (default)
    #[default]
    Below,
    /// Above the staff
    Above,
}

/// Horizontal alignment for dynamics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicsAlign {
    /// Left-aligned with note
    Left,
    /// Center on notehead (default)
    #[default]
    Center,
    /// Right-aligned with note
    Right,
}

/// Dynamics layout parameters.
#[derive(Debug, Clone)]
pub struct DynamicsParams {
    /// Unique identifier
    pub id: u64,
    /// Dynamic type
    pub dynamic_type: DynamicType,
    /// Custom text (for DynamicType::Other)
    pub custom_text: Option<String>,
    /// Placement (above/below staff)
    pub placement: DynamicsPlacement,
    /// Horizontal alignment
    pub align: DynamicsAlign,
    /// X position of associated note/segment
    pub x: f64,
    /// Width of associated note
    pub note_width: f64,
    /// Whether to center on notehead using optical center
    pub center_on_notehead: bool,
}

impl Default for DynamicsParams {
    fn default() -> Self {
        Self {
            id: 0,
            dynamic_type: DynamicType::Mf,
            custom_text: None,
            placement: DynamicsPlacement::Below,
            align: DynamicsAlign::Center,
            x: 0.0,
            note_width: 0.0,
            center_on_notehead: true,
        }
    }
}

/// Layout a dynamic marking.
#[must_use]
pub fn layout_dynamic(params: &DynamicsParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();

    // Dynamic-specific font size (typically larger than lyrics)
    let font_size = spatium * 2.2;
    let dynamic_width = params.dynamic_type.width() * spatium;
    let dynamic_height = font_size * 1.0;

    // Calculate X position based on alignment
    let x = match params.align {
        DynamicsAlign::Left => params.x,
        DynamicsAlign::Center => {
            let note_center = params.x + params.note_width / 2.0;
            note_center - dynamic_width / 2.0
        }
        DynamicsAlign::Right => params.x + params.note_width - dynamic_width,
    };

    // Calculate Y position based on placement
    let base_offset = spatium * 2.5; // Distance from staff
    let y = match params.placement {
        DynamicsPlacement::Below => base_offset,
        DynamicsPlacement::Above => -base_offset - dynamic_height,
    };

    let mut commands = Vec::new();

    // Draw the dynamic using SMuFL glyph or text
    if let Some(glyph) = params.dynamic_type.glyph() {
        commands.push(PaintCommand::glyph(
            glyph,
            Point::new(x, y + dynamic_height * 0.8),
            font_size,
            Color::BLACK,
        ));
    } else if let Some(text) = &params.custom_text {
        commands.push(PaintCommand::text(
            text.clone(),
            "serif", // Dynamics text font
            font_size,
            Point::new(x, y + dynamic_height * 0.8),
            Color::BLACK,
        ));
    } else {
        // Fallback to dynamic type text
        commands.push(PaintCommand::text(
            params.dynamic_type.text().to_string(),
            "serif", // Dynamics text font
            font_size,
            Point::new(x, y + dynamic_height * 0.8),
            Color::BLACK,
        ));
    }

    // Calculate bounding box
    let bbox = Rect::new(x, y, x + dynamic_width, y + dynamic_height);

    // Create shape for collision detection
    let shape = Shape::from_rect(bbox);

    // Create layout data
    let layout = LayoutData::new(Point::new(x, y), bbox, shape);

    // Create scene node with semantic ID
    let semantic_id = SemanticId::new(ElementType::Dynamic, params.id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("dynamic", params.dynamic_type.text().to_string());

    (layout, node)
}

/// Compute vertical position for dynamics, avoiding collisions with staff elements.
///
/// This is a simplified version of MuseScore's collision avoidance.
pub fn compute_dynamic_y_with_collision(
    base_y: f64,
    dynamic_height: f64,
    staff_skyline_max: f64,
    placement: DynamicsPlacement,
    min_distance: f64,
) -> f64 {
    match placement {
        DynamicsPlacement::Below => {
            // Ensure we're below any staff elements
            let min_y = staff_skyline_max + min_distance;
            base_y.max(min_y)
        }
        DynamicsPlacement::Above => {
            // Ensure we're above any staff elements (negative Y)
            let max_y = -staff_skyline_max - min_distance - dynamic_height;
            base_y.min(max_y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::style::MStyle;

    fn test_ctx() -> LayoutContext<'static> {
        use crate::engraver::layout::context::LayoutContextOwned;
        let owned = Box::leak(Box::new(LayoutContextOwned::new_minimal(MStyle::default())));
        owned.as_context()
    }

    #[test]
    fn test_layout_dynamic_mf() {
        let ctx = test_ctx();
        let params = DynamicsParams {
            id: 1,
            dynamic_type: DynamicType::Mf,
            x: 100.0,
            note_width: 10.0,
            ..Default::default()
        };

        let (layout, node) = layout_dynamic(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_all_dynamic_types() {
        let ctx = test_ctx();
        let types = [
            DynamicType::Ppp,
            DynamicType::Pp,
            DynamicType::P,
            DynamicType::Mp,
            DynamicType::Mf,
            DynamicType::F,
            DynamicType::Ff,
            DynamicType::Fff,
            DynamicType::Sfz,
            DynamicType::Sfp,
            DynamicType::Fz,
            DynamicType::Fp,
        ];

        for dynamic_type in types {
            let params = DynamicsParams {
                id: 1,
                dynamic_type,
                x: 100.0,
                note_width: 10.0,
                ..Default::default()
            };

            let (layout, node) = layout_dynamic(&params, &ctx);
            assert!(
                !layout.bbox.is_zero_area(),
                "{:?} should have valid bounding box",
                dynamic_type
            );
            assert!(node.id.is_some());
        }
    }

    #[test]
    fn test_dynamic_placement() {
        let ctx = test_ctx();

        let params_below = DynamicsParams {
            id: 1,
            dynamic_type: DynamicType::F,
            placement: DynamicsPlacement::Below,
            x: 100.0,
            ..Default::default()
        };
        let (layout_below, _) = layout_dynamic(&params_below, &ctx);

        let params_above = DynamicsParams {
            id: 2,
            dynamic_type: DynamicType::F,
            placement: DynamicsPlacement::Above,
            x: 100.0,
            ..Default::default()
        };
        let (layout_above, _) = layout_dynamic(&params_above, &ctx);

        // Above should have smaller Y than below
        assert!(layout_above.position.y < layout_below.position.y);
    }

    #[test]
    fn test_dynamic_alignment() {
        let ctx = test_ctx();
        let note_x = 100.0;
        let note_width = 20.0;

        let params_left = DynamicsParams {
            id: 1,
            dynamic_type: DynamicType::F,
            align: DynamicsAlign::Left,
            x: note_x,
            note_width,
            ..Default::default()
        };
        let (layout_left, _) = layout_dynamic(&params_left, &ctx);

        let params_center = DynamicsParams {
            id: 2,
            dynamic_type: DynamicType::F,
            align: DynamicsAlign::Center,
            x: note_x,
            note_width,
            ..Default::default()
        };
        let (layout_center, _) = layout_dynamic(&params_center, &ctx);

        let params_right = DynamicsParams {
            id: 3,
            dynamic_type: DynamicType::F,
            align: DynamicsAlign::Right,
            x: note_x,
            note_width,
            ..Default::default()
        };
        let (_layout_right, _) = layout_dynamic(&params_right, &ctx);

        // Left < Center < Right (approximately, depending on width)
        assert!(layout_left.position.x <= layout_center.position.x);
    }

    #[test]
    fn test_custom_dynamic() {
        let ctx = test_ctx();
        let params = DynamicsParams {
            id: 1,
            dynamic_type: DynamicType::Other,
            custom_text: Some("molto cresc.".to_string()),
            x: 100.0,
            ..Default::default()
        };

        let (layout, node) = layout_dynamic(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
    }
}
