//! Pattern Library
//!
//! Single source of truth for test patterns that can be used both in tests
//! and in the documentation site for interactive examples.

pub mod basic;
pub mod examples;
pub mod rhythm;

/// A documented pattern for testing and documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// Unique identifier for the pattern (used in URLs)
    pub id: &'static str,
    /// Human-readable title
    pub title: &'static str,
    /// Category for grouping
    pub category: PatternCategory,
    /// The keyflow source code
    pub source: &'static str,
    /// Description of what this pattern demonstrates
    pub description: &'static str,
}

impl Pattern {
    /// Create a new pattern.
    pub const fn new(
        id: &'static str,
        title: &'static str,
        category: PatternCategory,
        source: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            title,
            category,
            source,
            description,
        }
    }
}

/// Category for grouping patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatternCategory {
    /// Full song examples demonstrating complete charts
    Examples,
    /// Basic chart structure patterns
    BasicStructure,
    /// Rhythm and duration notation
    Rhythm,
    /// Triplet patterns
    Triplets,
    /// Push/pull timing
    PushPull,
    /// Chord notation
    Chords,
    /// Section markers and navigation
    Sections,
}

impl PatternCategory {
    /// Get a human-readable label for the category.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Examples => "Examples",
            Self::BasicStructure => "Basic Structure",
            Self::Rhythm => "Rhythm",
            Self::Triplets => "Triplets",
            Self::PushPull => "Push/Pull",
            Self::Chords => "Chords",
            Self::Sections => "Sections",
        }
    }

    /// Get a URL-safe slug for the category.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Examples => "examples",
            Self::BasicStructure => "basic",
            Self::Rhythm => "rhythm",
            Self::Triplets => "triplets",
            Self::PushPull => "push-pull",
            Self::Chords => "chords",
            Self::Sections => "sections",
        }
    }

    /// Get all categories in display order.
    pub const fn all() -> &'static [PatternCategory] {
        &[
            Self::Examples,
            Self::BasicStructure,
            Self::Rhythm,
            Self::Triplets,
            Self::PushPull,
            Self::Chords,
            Self::Sections,
        ]
    }
}

/// Get all registered patterns.
pub fn all_patterns() -> Vec<&'static Pattern> {
    let mut patterns = Vec::new();
    patterns.extend(examples::patterns());
    patterns.extend(basic::patterns());
    patterns.extend(rhythm::patterns());
    patterns
}

/// Find a pattern by its ID.
pub fn find_pattern(id: &str) -> Option<&'static Pattern> {
    all_patterns().into_iter().find(|p| p.id == id)
}

/// Get patterns filtered by category.
pub fn patterns_by_category(category: PatternCategory) -> Vec<&'static Pattern> {
    all_patterns()
        .into_iter()
        .filter(|p| p.category == category)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_patterns_have_unique_ids() {
        let patterns = all_patterns();
        let mut ids: Vec<_> = patterns.iter().map(|p| p.id).collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "Duplicate pattern IDs found");
    }

    #[test]
    fn test_find_pattern() {
        let patterns = all_patterns();
        if let Some(first) = patterns.first() {
            let found = find_pattern(first.id);
            assert!(found.is_some());
            assert_eq!(found.unwrap().id, first.id);
        }
    }

    #[test]
    fn test_patterns_by_category() {
        let rhythm_patterns = patterns_by_category(PatternCategory::Rhythm);
        for pattern in rhythm_patterns {
            assert_eq!(pattern.category, PatternCategory::Rhythm);
        }
    }
}
