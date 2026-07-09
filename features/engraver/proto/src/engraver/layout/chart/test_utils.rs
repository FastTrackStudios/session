//! Query utilities for headless layout testing.
//!
//! These utilities enable position verification tests without GPU rendering.

use super::*;
use crate::engraver::scene::id::ElementType;
use crate::engraver::scene::paint::{PaintCommand, TextAnchor};
use crate::engraver::scene::traverse::SceneNodeExt;
use kurbo::{Affine, Point};

/// Element position extracted from scene graph.
#[derive(Debug, Clone)]
pub struct ElementPosition {
    pub element_type: ElementType,
    pub id: u64,
    pub world_x: f64,
    pub world_y: f64,
    pub bounds: Option<Rect>,
    /// Page number (1-indexed), if the element was tagged during layout.
    pub page: Option<u32>,
}

/// Extract all elements of a specific type with their world positions.
///
/// Note: For elements like ChordSymbol where positions are baked into
/// paint commands rather than node transforms, this returns the transform
/// position only. Use `find_elements_with_content_bounds` for paint-based positions.
pub fn find_elements_by_type(scene: &SceneNode, element_type: ElementType) -> Vec<ElementPosition> {
    scene
        .iter_with_transforms()
        .filter_map(|(node, transform)| {
            let id = node.id.as_ref()?;
            if id.element_type != element_type {
                return None;
            }

            // Get world position from transform
            let world_origin = transform * Point::ORIGIN;

            // Extract page metadata if present
            let page = node
                .metadata
                .get("page")
                .and_then(|s| s.parse::<u32>().ok());

            Some(ElementPosition {
                element_type: id.element_type,
                id: id.id,
                world_x: world_origin.x,
                world_y: world_origin.y,
                bounds: if node.bounds.is_zero_area() {
                    None
                } else {
                    Some(node.bounds)
                },
                page,
            })
        })
        .collect()
}

/// Count elements of a specific type in the scene graph.
pub fn count_elements_by_type(scene: &SceneNode, element_type: ElementType) -> usize {
    scene
        .iter_with_transforms()
        .filter(|(node, _)| {
            node.id
                .as_ref()
                .is_some_and(|id| id.element_type == element_type)
        })
        .count()
}

/// Check if two x-positions are within tolerance (for alignment verification).
pub fn x_positions_aligned(x1: f64, x2: f64, tolerance: f64) -> bool {
    (x1 - x2).abs() <= tolerance
}

/// Find elements by type and ID.
pub fn find_element_by_id(
    scene: &SceneNode,
    element_type: ElementType,
    id: u64,
) -> Option<ElementPosition> {
    find_elements_by_type(scene, element_type)
        .into_iter()
        .find(|e| e.id == id)
}

/// Find elements by type on a specific page.
pub fn find_elements_on_page(
    scene: &SceneNode,
    element_type: ElementType,
    page: u32,
) -> Vec<ElementPosition> {
    find_elements_by_type(scene, element_type)
        .into_iter()
        .filter(|e| e.page == Some(page))
        .collect()
}

/// Group elements by page number.
pub fn group_elements_by_page(
    elements: &[ElementPosition],
) -> std::collections::HashMap<u32, Vec<&ElementPosition>> {
    use std::collections::HashMap;
    let mut grouped: HashMap<u32, Vec<&ElementPosition>> = HashMap::new();
    for elem in elements {
        if let Some(page) = elem.page {
            grouped.entry(page).or_default().push(elem);
        }
    }
    grouped
}

/// A visual paint item with a deterministic world-space collision box.
#[derive(Debug, Clone)]
pub struct VisualCollisionItem {
    pub node_index: usize,
    pub element_type: ElementType,
    pub element_id: u64,
    pub metadata_type: Option<String>,
    pub content: Option<String>,
    pub bounds: Rect,
}

/// A pair of visual scene items whose collision boxes overlap.
#[derive(Debug, Clone)]
pub struct VisualCollision {
    pub a: VisualCollisionItem,
    pub b: VisualCollisionItem,
    pub overlap: Rect,
}

/// Empty vertical band between two rendered systems.
#[derive(Debug, Clone)]
pub struct DeadVerticalSpace {
    pub page: u32,
    pub upper_system: usize,
    pub lower_system: usize,
    pub y0: f64,
    pub y1: f64,
    pub height: f64,
}

fn text_bounds(text: &str, font_size: f64, position: Point, anchor: TextAnchor) -> Rect {
    let width = text.chars().count() as f64 * font_size * 0.48;
    let (x0, x1) = match anchor {
        TextAnchor::Start => (position.x, position.x + width),
        TextAnchor::Middle => (position.x - width * 0.5, position.x + width * 0.5),
        TextAnchor::End => (position.x - width, position.x),
    };
    Rect::new(
        x0,
        position.y - font_size * 0.65,
        x1,
        position.y + font_size * 0.1,
    )
}

fn paint_collision_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Text {
            text,
            font_size,
            position,
            anchor,
            ..
        } => Some(text_bounds(text, *font_size, *position, *anchor)),
        PaintCommand::Glyph { position, size, .. } => {
            let half = size * 0.6;
            Some(Rect::new(
                position.x - half,
                position.y - half,
                position.x + half,
                position.y + half,
            ))
        }
        other => other.bounding_box(),
    }
}

pub fn visual_collision_items(scene: &SceneNode) -> Vec<VisualCollisionItem> {
    scene
        .iter_with_transforms()
        .enumerate()
        .flat_map(
            |(node_index, (node, transform)): (usize, (&SceneNode, Affine))| {
                let Some(id) = node.id.as_ref() else {
                    return Vec::new();
                };
                if id.element_type == ElementType::StaffLines {
                    return Vec::new();
                }

                node.commands
                    .iter()
                    .filter_map(move |command| {
                        let bounds =
                            transform.transform_rect_bbox(paint_collision_bounds(command)?);
                        (!bounds.is_zero_area()).then(|| VisualCollisionItem {
                            node_index,
                            element_type: id.element_type,
                            element_id: id.id,
                            metadata_type: node.metadata.get("element_type").cloned(),
                            content: match command {
                                PaintCommand::Text { text, .. } => Some(text.clone()),
                                PaintCommand::Glyph { codepoint, .. } => {
                                    Some(codepoint.to_string())
                                }
                                _ => None,
                            },
                            bounds,
                        })
                    })
                    .collect::<Vec<_>>()
            },
        )
        .collect()
}

fn merge_intervals(mut intervals: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in intervals {
        if end <= start {
            continue;
        }
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    merged
}

fn note_construction_element(element_type: ElementType) -> bool {
    matches!(
        element_type,
        ElementType::Note
            | ElementType::NoteHead
            | ElementType::Stem
            | ElementType::Beam
            | ElementType::Flag
            | ElementType::LedgerLine
            | ElementType::Dot
    )
}

fn allowed_internal_collision(a: &VisualCollisionItem, b: &VisualCollisionItem) -> bool {
    a.node_index == b.node_index
        || (note_construction_element(a.element_type) && note_construction_element(b.element_type))
}

fn rect_overlap(a: Rect, b: Rect) -> Option<Rect> {
    let overlap = Rect::new(
        a.x0.max(b.x0),
        a.y0.max(b.y0),
        a.x1.min(b.x1),
        a.y1.min(b.y1),
    );
    (overlap.width() > 0.5 && overlap.height() > 0.5).then_some(overlap)
}

/// Detect disallowed visual collisions between rendered symbols.
///
/// This intentionally ignores staff lines and normal note-construction contact
/// points (noteheads/stems/beams/flags/ledger lines/dots). Everything else is
/// treated as a distinct symbol and should not overlap in world space.
pub fn find_disallowed_visual_collisions(scene: &SceneNode) -> Vec<VisualCollision> {
    let items = visual_collision_items(scene);
    let mut collisions = Vec::new();

    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let a = &items[i];
            let b = &items[j];
            if allowed_internal_collision(a, b) {
                continue;
            }
            if let Some(overlap) = rect_overlap(a.bounds, b.bounds) {
                collisions.push(VisualCollision {
                    a: a.clone(),
                    b: b.clone(),
                    overlap,
                });
            }
        }
    }

    collisions
}

/// Find tall empty vertical bands between adjacent systems.
///
/// This scans the actual rendered ink, not nominal layout boxes. If a band
/// contains no visible symbol ink anywhere across the page content width, it is
/// dead space that a later compaction pass can potentially remove.
pub fn find_dead_vertical_space_between_systems(
    result: &ChartLayoutResult,
    min_height: f64,
) -> Vec<DeadVerticalSpace> {
    let items = visual_collision_items(&result.scene);
    let mut dead_spaces = Vec::new();

    for page in &result.pages {
        for pair in page.systems.windows(2) {
            let upper = &pair[0];
            let lower = &pair[1];
            let scan_y0 = page.y_offset + upper.y;
            let scan_y1 = page.y_offset + lower.y;
            if scan_y1 <= scan_y0 {
                continue;
            }

            let x0 = page.x_offset + page.margins.left;
            let x1 = page.x_offset + page.width - page.margins.right;
            let occupied = items
                .iter()
                .filter(|item| item.bounds.x1 > x0 && item.bounds.x0 < x1)
                .filter(|item| item.bounds.y1 > scan_y0 && item.bounds.y0 < scan_y1)
                .map(|item| (item.bounds.y0.max(scan_y0), item.bounds.y1.min(scan_y1)))
                .collect::<Vec<_>>();

            let mut cursor = scan_y0;
            for (start, end) in merge_intervals(occupied) {
                if start - cursor >= min_height {
                    dead_spaces.push(DeadVerticalSpace {
                        page: page.number,
                        upper_system: upper.index,
                        lower_system: lower.index,
                        y0: cursor,
                        y1: start,
                        height: start - cursor,
                    });
                }
                cursor = cursor.max(end);
            }

            if scan_y1 - cursor >= min_height {
                dead_spaces.push(DeadVerticalSpace {
                    page: page.number,
                    upper_system: upper.index,
                    lower_system: lower.index,
                    y0: cursor,
                    y1: scan_y1,
                    height: scan_y1 - cursor,
                });
            }
        }
    }

    dead_spaces
}
