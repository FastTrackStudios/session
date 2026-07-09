//! Parser helper types and utilities
//!
//! Contains helper structures for tracking line offsets, repeat counts,
//! and push/pull modifiers during chart parsing.

use crate::chord::{LilySyntax, PushPullAmount, PushPullBase};
use crate::parsing::TextSpan;

// region:    --- LineOffsetMap

/// Helper to compute byte offsets from line/column positions
///
/// Builds a mapping from line number (1-indexed) to byte offset,
/// enabling reconstruction of TextSpan from line-based parsing.
#[derive(Debug, Clone)]
pub struct LineOffsetMap {
    /// Byte offset of the start of each line (0-indexed array, 1-indexed line numbers)
    /// line_offsets[0] is always 0 (line 1 starts at byte 0)
    line_offsets: Vec<usize>,
}

impl LineOffsetMap {
    /// Build a line offset map from input text
    pub fn new(input: &str) -> Self {
        let mut line_offsets = vec![0]; // Line 1 starts at offset 0
        for (i, ch) in input.char_indices() {
            if ch == '\n' {
                // Next line starts at the byte after the newline
                line_offsets.push(i + 1);
            }
        }
        Self { line_offsets }
    }

    /// Get the byte offset for a given line and column (both 1-indexed)
    pub fn offset_at(&self, line: u32, column: u32) -> usize {
        let line_idx = (line.saturating_sub(1)) as usize;
        let line_start = self.line_offsets.get(line_idx).copied().unwrap_or(0);
        line_start + (column.saturating_sub(1)) as usize
    }

    /// Create a TextSpan for a token at the given line, column, and length
    pub fn span_at(&self, line: u32, column: u32, len: usize) -> TextSpan {
        let start = self.offset_at(line, column);
        TextSpan::with_location(start, len, line, column)
    }

    /// Get the number of lines tracked
    pub fn line_count(&self) -> usize {
        self.line_offsets.len()
    }
}

// endregion: --- LineOffsetMap

// region:    --- RepeatCount

/// Repeat count specification
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepeatCount {
    /// Fixed number of repeats (e.g., x4)
    Fixed(usize),
    /// Auto-calculate based on section length (x^)
    Auto,
}

// endregion: --- RepeatCount

// region:    --- PushPullModifier

/// Push/pull modifier extracted from token
#[derive(Debug, Clone, Copy, Default)]
pub struct PushPullModifier {
    /// Number of apostrophes (1-3) - used for level-based push/pull
    pub count: usize,
    /// Explicit triplet marker 't'
    pub is_triplet: bool,
    /// Explicit tuplet number from ':N' syntax
    pub tuplet: Option<u8>,
    /// Explicit duration from '_N' syntax (e.g., '_4 for quarter note)
    pub duration: Option<LilySyntax>,
    /// Whether the duration is dotted (e.g., '_4.')
    pub duration_dotted: bool,
    /// Whether the duration is a triplet (e.g., '_8t')
    pub duration_triplet: bool,
}

impl PushPullModifier {
    /// Check if this modifier is present
    pub fn is_present(&self) -> bool {
        self.count > 0 || self.duration.is_some()
    }

    /// Convert to PushPullAmount using default base from settings
    pub fn to_amount(&self, default_base: PushPullBase) -> Option<PushPullAmount> {
        // Duration-based push/pull takes precedence
        if let Some(duration) = self.duration {
            return Some(PushPullAmount::from_duration(
                duration,
                self.duration_dotted,
                self.duration_triplet,
            ));
        }

        if self.count == 0 {
            return None;
        }

        // Level-based push/pull: determine the base from explicit modifiers or default
        let base = if self.is_triplet {
            PushPullBase::Triplet
        } else if let Some(n) = self.tuplet {
            if n == 3 {
                PushPullBase::Triplet
            } else {
                PushPullBase::Tuplet(n)
            }
        } else {
            default_base
        };

        Some(PushPullAmount {
            level: self.count as u8,
            base,
        })
    }
}

// endregion: --- PushPullModifier

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_offset_map() {
        let input = "line1\nline2\nline3";
        let map = LineOffsetMap::new(input);

        // Check number of lines
        assert_eq!(map.line_count(), 3);

        // Line 1 starts at offset 0
        assert_eq!(map.offset_at(1, 1), 0);
        assert_eq!(map.offset_at(1, 3), 2); // 'n' in "line1"

        // Line 2 starts at offset 6 (after "line1\n")
        assert_eq!(map.offset_at(2, 1), 6);
        assert_eq!(map.offset_at(2, 5), 10); // '2' in "line2"

        // Line 3 starts at offset 12 (after "line1\nline2\n")
        assert_eq!(map.offset_at(3, 1), 12);

        // Test span_at helper
        let span = map.span_at(2, 1, 5);
        assert_eq!(span.start, 6);
        assert_eq!(span.len, 5);
        assert_eq!(span.line, 2);
        assert_eq!(span.column, 1);
    }
}

// endregion: --- Tests
