//! Vocal group definitions

use crate::item_metadata::prelude::*;

pub mod background_vocals;
pub mod lead_vocals;

pub use background_vocals::BackgroundVocals;
pub use lead_vocals::LeadVocals;

/// Top-level vocals group containing lead and background vocals
///
/// Not transparent - creates a Vocals folder when there are multiple vocal types
/// (e.g., Lead and BGVs). This provides clearer organization in the track list.
pub struct Vocals;

impl From<Vocals> for ItemMetadataGroup {
    fn from(_val: Vocals) -> Self {
        ItemMetadataGroup::builder("Vocals")
            .prefix("V")
            .patterns([
                "vocal", "vocals", "vox", "voc", "voca", "voice", "lv", "bv", "bg",
            ])
            // Exclude non-vocal voice effects (these should go to SFX)
            // Also exclude "cowbell" to prevent compound names like "CowbellGangVox" from matching
            .exclude(["robot", "vocoder", "talkbox", "cowbell"])
            .group(LeadVocals)
            .group(BackgroundVocals)
            .build()
    }
}
