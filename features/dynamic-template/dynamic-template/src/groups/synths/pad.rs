//! Synth pad group definition

use crate::item_metadata::ItemMetadata;
use crate::item_metadata::prelude::ItemMetadataGroupExt;
use monarchy::Group;

/// Pad synth group
pub struct Pad;

impl From<Pad> for Group<ItemMetadata> {
    fn from(_val: Pad) -> Self {
        Group::builder("Pad")
            .patterns(vec!["pad", "pads", "ambient"])
            .layers(super::layers_dimension())
            .channel(super::channel_dimension())
            .build()
    }
}
