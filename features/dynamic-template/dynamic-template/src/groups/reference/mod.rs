//! Reference track group definitions
//!
//! This group captures bounced/printed mixes and reference tracks:
//! - Master/mix prints
//! - Reference mixes
//! - 2-track bounces
//! - Rehearsal mixes
//! - Any bounced version of the song for reference

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Top-level Reference group for bounced mixes and reference tracks
pub struct Reference;

impl From<Reference> for Group<ItemMetadata> {
    fn from(_val: Reference) -> Self {
        Group::builder("Reference")
            .prefix("REF")
            .patterns(vec![
                // Master/mix patterns
                "master",
                "mix",
                "print",
                "bounce",
                "bounced",
                "stereo mix",
                "final mix",
                "rough mix",
                // Reference patterns
                "reference",
                "ref",
                "ref mix",
                "reference mix",
                // Mixdown abbreviations
                "MDN",
                "mixdown",
                "mix down",
                // 2-track patterns
                "2 track",
                "2-track",
                "2track",
                "two track",
                "stereo",
                // Rehearsal/demo patterns
                "rehearsal",
                "rehearsal mix",
                "demo",
                "demo mix",
                "scratch",
                "scratch mix",
                // Export/render patterns
                "export",
                "render",
                "rendered",
            ])
            .build()
    }
}
