//! Segment system for vertical alignment of music elements.
//!
//! A Segment represents a vertical slice through the score at a specific tick position.
//! All elements in a segment start at the same tick. Segments are ordered by type
//! within a measure, and by tick position across the score.
//!
//! This is a Rust port of MuseScore's segment system.

use bitflags::bitflags;
use kurbo::Rect;
use serde::{Deserialize, Serialize};

use super::kerning::KerningType;
use super::shape::Shape;

bitflags! {
    /// Types of segments that can appear in a measure.
    ///
    /// The order of bits determines the visual order of segments at the same tick.
    /// Multiple segment types can be combined using bitwise OR.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SegmentType: u32 {
        /// Invalid/unset segment type
        const INVALID = 0;
        /// Barline at the beginning of a measure
        const BEGIN_BAR_LINE = 1 << 0;
        /// Clef at the header (beginning of system)
        const HEADER_CLEF = 1 << 1;
        /// Key signature
        const KEY_SIG = 1 << 2;
        /// Ambitus (range indicator)
        const AMBITUS = 1 << 3;
        /// Breath mark
        const BREATH = 1 << 4;
        /// Time signature
        const TIME_SIG = 1 << 5;
        /// Start repeat barline
        const START_REPEAT_BAR_LINE = 1 << 6;
        /// Clef announcement for start repeat
        const CLEF_START_REPEAT_ANNOUNCE = 1 << 7;
        /// Key signature announcement for start repeat
        const KEY_SIG_START_REPEAT_ANNOUNCE = 1 << 8;
        /// Time signature announcement for start repeat
        const TIME_SIG_START_REPEAT_ANNOUNCE = 1 << 9;
        /// Clef change
        const CLEF = 1 << 10;
        /// Mid-measure barline
        const BAR_LINE = 1 << 11;
        /// Time tick (for spacing calculations)
        const TIME_TICK = 1 << 12;
        /// Chord or rest
        const CHORD_REST = 1 << 13;
        /// Clef announcement for repeat
        const CLEF_REPEAT_ANNOUNCE = 1 << 14;
        /// Key signature announcement for repeat
        const KEY_SIG_REPEAT_ANNOUNCE = 1 << 15;
        /// Time signature announcement for repeat
        const TIME_SIG_REPEAT_ANNOUNCE = 1 << 16;
        /// End barline
        const END_BAR_LINE = 1 << 17;
        /// Key signature announcement (courtesy)
        const KEY_SIG_ANNOUNCE = 1 << 18;
        /// Time signature announcement (courtesy)
        const TIME_SIG_ANNOUNCE = 1 << 19;

        // Composite types
        /// All barline types
        const BAR_LINE_TYPE = Self::BEGIN_BAR_LINE.bits()
            | Self::START_REPEAT_BAR_LINE.bits()
            | Self::BAR_LINE.bits()
            | Self::END_BAR_LINE.bits();

        /// All courtesy time signature types
        const COURTESY_TIME_SIG = Self::TIME_SIG_ANNOUNCE.bits()
            | Self::TIME_SIG_REPEAT_ANNOUNCE.bits()
            | Self::TIME_SIG_START_REPEAT_ANNOUNCE.bits();

        /// All courtesy key signature types
        const COURTESY_KEY_SIG = Self::KEY_SIG_ANNOUNCE.bits()
            | Self::KEY_SIG_REPEAT_ANNOUNCE.bits()
            | Self::KEY_SIG_START_REPEAT_ANNOUNCE.bits();

        /// All courtesy clef types
        const COURTESY_CLEF = Self::CLEF_REPEAT_ANNOUNCE.bits()
            | Self::CLEF_START_REPEAT_ANNOUNCE.bits();

        /// All time signature types
        const TIME_SIG_TYPE = Self::TIME_SIG.bits() | Self::COURTESY_TIME_SIG.bits();

        /// All key signature types
        const KEY_SIG_TYPE = Self::KEY_SIG.bits() | Self::COURTESY_KEY_SIG.bits();

        /// All clef types
        const CLEF_TYPE = Self::CLEF.bits()
            | Self::HEADER_CLEF.bits()
            | Self::COURTESY_CLEF.bits();

        /// Duration segments (may have non-zero tick length)
        const DURATION_SEGMENTS = Self::CHORD_REST.bits() | Self::TIME_TICK.bits();
    }
}

impl SegmentType {
    /// Get the display name for this segment type.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match *self {
            Self::BEGIN_BAR_LINE => "BeginBarLine",
            Self::HEADER_CLEF => "HeaderClef",
            Self::KEY_SIG => "KeySig",
            Self::AMBITUS => "Ambitus",
            Self::BREATH => "Breath",
            Self::TIME_SIG => "TimeSig",
            Self::START_REPEAT_BAR_LINE => "StartRepeatBarLine",
            Self::CLEF => "Clef",
            Self::BAR_LINE => "BarLine",
            Self::TIME_TICK => "TimeTick",
            Self::CHORD_REST => "ChordRest",
            Self::END_BAR_LINE => "EndBarLine",
            Self::KEY_SIG_ANNOUNCE => "KeySigAnnounce",
            Self::TIME_SIG_ANNOUNCE => "TimeSigAnnounce",
            _ => "Unknown",
        }
    }

    /// Check if this is a ChordRest segment.
    #[must_use]
    pub const fn is_chord_rest(&self) -> bool {
        self.bits() == Self::CHORD_REST.bits()
    }

    /// Check if this is a duration segment (ChordRest or TimeTick).
    #[must_use]
    pub fn is_duration_segment(&self) -> bool {
        self.intersects(Self::DURATION_SEGMENTS)
    }

    /// Check if this is a barline segment.
    #[must_use]
    pub fn is_barline(&self) -> bool {
        self.intersects(Self::BAR_LINE_TYPE)
    }

    /// Check if this is a courtesy segment (key/time sig announcements).
    #[must_use]
    pub fn is_courtesy(&self) -> bool {
        self.intersects(Self::COURTESY_TIME_SIG | Self::COURTESY_KEY_SIG | Self::COURTESY_CLEF)
    }

    /// Check if this segment should be right-aligned.
    #[must_use]
    pub fn is_right_aligned(&self) -> bool {
        self.intersects(Self::CLEF | Self::BREATH)
    }
}

impl Default for SegmentType {
    fn default() -> Self {
        Self::INVALID
    }
}

/// Number of voices per staff.
pub const VOICES: usize = 4;

/// A vertical slice through the score at a specific tick position.
///
/// Segments hold all vertically-aligned staff elements. Each segment contains
/// elements of the same type (e.g., all clefs, all chord/rests at a tick).
///
/// Elements are stored per-track, where track = staff_idx * VOICES + voice.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Segment type
    pub seg_type: SegmentType,

    /// Tick position relative to measure start
    pub tick: i32,

    /// Duration in ticks (for ChordRest segments)
    pub ticks: i32,

    /// X position relative to measure
    pub x: f64,

    /// Width of the segment
    pub width: f64,

    /// Width offset for spacing adjustments
    pub width_offset: f64,

    /// Additional spacing (e.g., for accidentals)
    pub spacing: f64,

    /// Stretch factor for justification
    pub stretch: f64,

    /// Extra leading space
    pub extra_leading_space: f64,

    /// Minimum width required (from collision detection)
    pub min_width: f64,

    /// User-specified stretch multiplier
    pub user_stretch: f64,

    /// Kerning type for spacing calculations
    pub kerning: KerningType,

    /// Whether this segment is enabled (visible)
    pub enabled: bool,

    /// Whether this segment is visible
    pub visible: bool,

    /// Elements per track (staff * VOICES + voice)
    /// None means no element at that track
    elements: Vec<Option<ElementId>>,

    /// Shape per staff (for collision detection)
    shapes: Vec<Shape>,

    /// Annotations (dynamics, text, etc.)
    annotations: Vec<ElementId>,
}

/// Opaque identifier for a music element.
///
/// This is a placeholder - in the full implementation this would link
/// to the actual music model elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementId(pub u64);

impl Default for Segment {
    fn default() -> Self {
        Self::new(SegmentType::CHORD_REST)
    }
}

impl Segment {
    /// Create a new segment with the given type.
    #[must_use]
    pub fn new(seg_type: SegmentType) -> Self {
        Self {
            seg_type,
            tick: 0,
            ticks: 0,
            x: 0.0,
            width: 0.0,
            width_offset: 0.0,
            spacing: 0.0,
            stretch: 1.0,
            extra_leading_space: 0.0,
            min_width: 0.0,
            user_stretch: 1.0,
            kerning: KerningType::default(),
            enabled: true,
            visible: true,
            elements: Vec::new(),
            shapes: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// Create a new ChordRest segment at the given tick.
    #[must_use]
    pub fn chord_rest(tick: i32, ticks: i32) -> Self {
        Self {
            seg_type: SegmentType::CHORD_REST,
            tick,
            ticks,
            ..Default::default()
        }
    }

    /// Create a new clef segment.
    #[must_use]
    pub fn clef(tick: i32) -> Self {
        Self {
            seg_type: SegmentType::CLEF,
            tick,
            ..Default::default()
        }
    }

    /// Create a key signature segment.
    #[must_use]
    pub fn key_sig(tick: i32) -> Self {
        Self {
            seg_type: SegmentType::KEY_SIG,
            tick,
            ..Default::default()
        }
    }

    /// Create a time signature segment.
    #[must_use]
    pub fn time_sig(tick: i32) -> Self {
        Self {
            seg_type: SegmentType::TIME_SIG,
            tick,
            ..Default::default()
        }
    }

    /// Create a barline segment.
    #[must_use]
    pub fn barline(tick: i32) -> Self {
        Self {
            seg_type: SegmentType::BAR_LINE,
            tick,
            ..Default::default()
        }
    }

    /// Create an end barline segment.
    #[must_use]
    pub fn end_barline(tick: i32) -> Self {
        Self {
            seg_type: SegmentType::END_BAR_LINE,
            tick,
            ..Default::default()
        }
    }

    /// Initialize elements and shapes for a given number of staves.
    pub fn init_staves(&mut self, num_staves: usize) {
        let num_tracks = num_staves * VOICES;
        self.elements.resize(num_tracks, None);
        self.shapes.resize_with(num_staves, Shape::default);
    }

    /// Check if this segment is active (enabled and visible, not a time tick).
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.seg_type.contains(SegmentType::TIME_TICK) && self.enabled && self.visible
    }

    /// Check if this segment is empty (no elements).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.iter().all(Option::is_none) && self.annotations.is_empty()
    }

    /// Check if this segment has any elements.
    #[must_use]
    pub fn has_elements(&self) -> bool {
        self.elements.iter().any(Option::is_some)
    }

    /// Check if this segment has elements in a track range.
    #[must_use]
    pub fn has_elements_in_range(&self, min_track: usize, max_track: usize) -> bool {
        self.elements
            .iter()
            .skip(min_track)
            .take(max_track - min_track + 1)
            .any(Option::is_some)
    }

    /// Get the element at a specific track.
    #[must_use]
    pub fn element(&self, track: usize) -> Option<ElementId> {
        self.elements.get(track).copied().flatten()
    }

    /// Set an element at a specific track.
    pub fn set_element(&mut self, track: usize, element: Option<ElementId>) {
        if track >= self.elements.len() {
            self.elements.resize(track + 1, None);
        }
        self.elements[track] = element;
    }

    /// Remove the element at a specific track.
    pub fn remove_element(&mut self, track: usize) {
        if track < self.elements.len() {
            self.elements[track] = None;
        }
    }

    /// Get the shape for a specific staff.
    #[must_use]
    pub fn staff_shape(&self, staff_idx: usize) -> Option<&Shape> {
        self.shapes.get(staff_idx)
    }

    /// Get mutable reference to the shape for a specific staff.
    pub fn staff_shape_mut(&mut self, staff_idx: usize) -> Option<&mut Shape> {
        self.shapes.get_mut(staff_idx)
    }

    /// Set the shape for a specific staff.
    pub fn set_staff_shape(&mut self, staff_idx: usize, shape: Shape) {
        if staff_idx >= self.shapes.len() {
            self.shapes.resize_with(staff_idx + 1, Shape::default);
        }
        self.shapes[staff_idx] = shape;
    }

    /// Add an annotation to this segment.
    pub fn add_annotation(&mut self, element: ElementId) {
        self.annotations.push(element);
    }

    /// Get the annotations for this segment.
    #[must_use]
    pub fn annotations(&self) -> &[ElementId] {
        &self.annotations
    }

    /// Get the bounding box of this segment.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let mut bounds = Rect::ZERO;
        for shape in &self.shapes {
            if !shape.is_empty() {
                bounds = bounds.union(shape.bbox());
            }
        }
        bounds
    }

    /// Get the combined shape of all staves in this segment.
    ///
    /// This creates a single shape containing all the shapes from all staves,
    /// useful for collision detection across the entire segment.
    #[must_use]
    pub fn combined_shape(&self) -> Shape {
        if self.shapes.is_empty() {
            // No shapes - return a shape based on width
            if self.width > 0.0 {
                Shape::from_rect(Rect::new(self.x, 0.0, self.x + self.width, 1.0))
            } else {
                Shape::empty()
            }
        } else if self.shapes.len() == 1 {
            self.shapes[0].clone()
        } else {
            // Combine all shapes into one
            let mut elements = Vec::new();
            for shape in &self.shapes {
                match shape {
                    Shape::Fixed { bbox, element } => {
                        if bbox.width() > 0.0 || bbox.height() > 0.0 {
                            elements.push(super::shape::ShapeElement {
                                rect: *bbox,
                                element: *element,
                                ignore_for_layout: false,
                            });
                        }
                    }
                    Shape::Composite {
                        elements: shape_elems,
                        ..
                    } => {
                        elements.extend(shape_elems.iter().cloned());
                    }
                }
            }
            Shape::from_elements(elements)
        }
    }

    /// Get the minimum distance from the left side.
    #[must_use]
    pub fn min_left(&self) -> f64 {
        self.shapes
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.left())
            .fold(f64::INFINITY, f64::min)
    }

    /// Get the minimum distance from the right side.
    #[must_use]
    pub fn min_right(&self) -> f64 {
        self.shapes
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.right())
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Compare segments for ordering.
    /// Segments are ordered by tick, then by type.
    #[must_use]
    pub fn cmp_order(&self, other: &Self) -> std::cmp::Ordering {
        match self.tick.cmp(&other.tick) {
            std::cmp::Ordering::Equal => self.seg_type.bits().cmp(&other.seg_type.bits()),
            ord => ord,
        }
    }
}

impl PartialEq for Segment {
    fn eq(&self, other: &Self) -> bool {
        self.tick == other.tick && self.seg_type == other.seg_type
    }
}

impl Eq for Segment {}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl Ord for Segment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp_order(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_type_flags() {
        let seg_type = SegmentType::CHORD_REST;
        assert!(seg_type.is_chord_rest());
        assert!(seg_type.is_duration_segment());
        assert!(!seg_type.is_barline());
    }

    #[test]
    fn test_segment_type_composite() {
        let barline_type = SegmentType::BAR_LINE_TYPE;
        assert!(barline_type.contains(SegmentType::BEGIN_BAR_LINE));
        assert!(barline_type.contains(SegmentType::END_BAR_LINE));
        assert!(!barline_type.contains(SegmentType::CHORD_REST));
    }

    #[test]
    fn test_segment_new() {
        let seg = Segment::new(SegmentType::CHORD_REST);
        assert!(seg.seg_type.is_chord_rest());
        assert_eq!(seg.tick, 0);
        assert!(seg.is_empty());
    }

    #[test]
    fn test_segment_chord_rest() {
        let seg = Segment::chord_rest(480, 480);
        assert_eq!(seg.tick, 480);
        assert_eq!(seg.ticks, 480);
        assert!(seg.seg_type.is_chord_rest());
    }

    #[test]
    fn test_segment_elements() {
        let mut seg = Segment::new(SegmentType::CHORD_REST);
        seg.init_staves(2);

        assert!(seg.is_empty());
        assert!(!seg.has_elements());

        seg.set_element(0, Some(ElementId(1)));
        assert!(!seg.is_empty());
        assert!(seg.has_elements());
        assert_eq!(seg.element(0), Some(ElementId(1)));
        assert_eq!(seg.element(1), None);

        seg.remove_element(0);
        assert!(seg.is_empty());
    }

    #[test]
    fn test_segment_ordering() {
        let seg1 = Segment::chord_rest(0, 480);
        let seg2 = Segment::chord_rest(480, 480);
        let seg3 = Segment::clef(0);

        // Same tick, different types - order by type
        assert!(seg3 < seg1); // CLEF < CHORD_REST

        // Different ticks - order by tick
        assert!(seg1 < seg2);
    }

    #[test]
    fn test_segment_is_active() {
        let mut seg = Segment::new(SegmentType::CHORD_REST);
        assert!(seg.is_active());

        seg.enabled = false;
        assert!(!seg.is_active());

        seg.enabled = true;
        seg.visible = false;
        assert!(!seg.is_active());

        let time_tick = Segment::new(SegmentType::TIME_TICK);
        assert!(!time_tick.is_active());
    }

    #[test]
    fn test_segment_annotations() {
        let mut seg = Segment::new(SegmentType::CHORD_REST);
        assert!(seg.annotations().is_empty());

        seg.add_annotation(ElementId(100));
        seg.add_annotation(ElementId(101));

        assert_eq!(seg.annotations().len(), 2);
        assert!(seg.annotations().contains(&ElementId(100)));
    }
}
