//! Interaction layer for editing and selection.
//!
//! This module handles user interaction with the score:
//! - Hit testing
//! - Selection management
//! - Editing operations
//! - Undo/redo

use crate::engraver::scene::SemanticId;
use kurbo::Point;

/// Selection state.
///
/// Holds the set of currently-selected scene-graph elements, identified by
/// `SemanticId`. Multi-select is supported; uniqueness is enforced by
/// equality on `SemanticId`.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Currently selected objects
    pub selected: Vec<SemanticId>,
}

impl Selection {
    /// Create a new empty selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if anything is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// Number of selected objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Whether `id` is currently selected.
    #[must_use]
    pub fn contains(&self, id: &SemanticId) -> bool {
        self.selected.contains(id)
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// Select a single object (replaces any existing selection).
    pub fn select(&mut self, id: SemanticId) {
        self.selected.clear();
        self.selected.push(id);
    }

    /// Add to the selection (for multi-select). No-op if already selected.
    pub fn add(&mut self, id: SemanticId) {
        if !self.selected.contains(&id) {
            self.selected.push(id);
        }
    }

    /// Toggle selection of `id`: remove if present, add if not.
    pub fn toggle(&mut self, id: SemanticId) {
        if let Some(pos) = self.selected.iter().position(|x| x == &id) {
            self.selected.remove(pos);
        } else {
            self.selected.push(id);
        }
    }
}

/// Cursor position in the score.
#[derive(Debug, Clone)]
pub struct Cursor {
    /// Current position in score coordinates
    pub position: Point,
    /// Whether the cursor is visible
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            position: Point::ZERO,
            visible: true,
        }
    }
}

/// Editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    /// Normal selection mode
    #[default]
    Select,
    /// Note entry mode
    NoteEntry,
    /// Rest entry mode
    RestEntry,
}

/// Interaction state for the editor.
#[derive(Debug, Default)]
pub struct InteractionState {
    /// Current selection
    pub selection: Selection,
    /// Cursor position
    pub cursor: Cursor,
    /// Current edit mode
    pub mode: EditMode,
}

impl InteractionState {
    /// Create a new interaction state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::id::ElementType;

    fn id(n: u64) -> SemanticId {
        SemanticId::new(ElementType::Chord, n)
    }

    #[test]
    fn selection_select_replaces() {
        let mut s = Selection::new();
        s.select(id(1));
        s.select(id(2));
        assert_eq!(s.len(), 1);
        assert!(s.contains(&id(2)));
    }

    #[test]
    fn selection_add_dedupes() {
        let mut s = Selection::new();
        s.add(id(1));
        s.add(id(1));
        s.add(id(2));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn selection_toggle_round_trips() {
        let mut s = Selection::new();
        s.toggle(id(1));
        assert!(s.contains(&id(1)));
        s.toggle(id(1));
        assert!(s.is_empty());
    }
}
