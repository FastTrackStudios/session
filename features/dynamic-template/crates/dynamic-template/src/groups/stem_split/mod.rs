//! Stem Split group definitions
//!
//! This group captures outputs from AI stem separation tools:
//! - Demucs: `drums.wav`, `bass.wav`, `vocals.wav`, `other.wav`, `piano.wav`
//! - LALAL.ai: `*_vocals.*`, `*_instrumental.*`, `*_drums.*`, `*_bass.*`
//! - Generic stem names when found as a cohesive set

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Stem Split group for AI-separated stems (Demucs, LALAL.ai, etc.)
pub struct StemSplit;

impl From<StemSplit> for Group<ItemMetadata> {
    fn from(_val: StemSplit) -> Self {
        Group::builder("Stem Split")
            .prefix("SS")
            .patterns(vec![
                // Demucs output patterns
                "htdemucs",
                "demucs",
                "mdx",
                "mdx_extra",
                // LALAL.ai suffix patterns
                "stem split",
                "stem-split",
                "separated",
                "isolation",
                "isolated",
                // Generic stem patterns (these match when items are
                // clearly stem-split outputs rather than live recordings)
                "other",     // Demucs "other" stem
                "no_vocals", // Vocal-removed version
                "no vocals",
                "instrumental", // Instrumental stem
                "accompaniment",
            ])
            .build()
    }
}

/// Standard stem categories recognized by stem separation tools.
const STEM_CATEGORIES: &[&str] = &[
    "drums",
    "bass",
    "vocals",
    "other",
    "piano",
    "guitar",
    "no_vocals",
    "instrumental",
    "accompaniment",
];

/// Check if a set of item names looks like stem-split outputs.
///
/// Returns `true` if 3+ items match standard stem category names from the
/// same apparent source. This helps distinguish AI-separated stems from
/// live-recorded tracks that happen to be named "drums", "bass", etc.
pub fn is_stem_split_set(items: &[String]) -> bool {
    if items.len() < 3 {
        return false;
    }

    let mut matches = 0;
    for item in items {
        let lower = item.to_lowercase();
        // Check bare name (e.g., "drums.wav" → "drums")
        let stem = lower
            .rsplit_once('.')
            .map(|(name, _ext)| name)
            .unwrap_or(&lower);
        // Also check the last path component
        let basename = stem.rsplit_once('/').map(|(_, name)| name).unwrap_or(stem);

        if STEM_CATEGORIES.iter().any(|cat| {
            basename == *cat
                || basename.ends_with(&format!("_{cat}"))
                || basename.ends_with(&format!("-{cat}"))
        }) {
            matches += 1;
        }
    }

    matches >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demucs_output_detected() {
        let items = vec![
            "drums.wav".to_string(),
            "bass.wav".to_string(),
            "vocals.wav".to_string(),
            "other.wav".to_string(),
        ];
        assert!(is_stem_split_set(&items));
    }

    #[test]
    fn lalal_ai_output_detected() {
        let items = vec![
            "Song_drums.wav".to_string(),
            "Song_bass.wav".to_string(),
            "Song_vocals.wav".to_string(),
            "Song_instrumental.wav".to_string(),
        ];
        assert!(is_stem_split_set(&items));
    }

    #[test]
    fn too_few_items_not_detected() {
        let items = vec!["drums.wav".to_string(), "bass.wav".to_string()];
        assert!(!is_stem_split_set(&items));
    }

    #[test]
    fn unrelated_items_not_detected() {
        let items = vec![
            "Lead Vocal.wav".to_string(),
            "Harmony BG.wav".to_string(),
            "Rhythm Guitar.wav".to_string(),
            "Pad.wav".to_string(),
        ];
        assert!(!is_stem_split_set(&items));
    }
}
