//! Oboes group definition

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Oboes group
pub struct Oboes;

impl From<Oboes> for Group<ItemMetadata> {
    fn from(_val: Oboes) -> Self {
        Group::builder("Oboes")
            .patterns(vec!["oboe", "oboes", "ob"])
            .build()
    }
}
