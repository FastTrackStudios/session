//! Piccolo group definition

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Piccolo group
pub struct Piccolo;

impl From<Piccolo> for Group<ItemMetadata> {
    fn from(_val: Piccolo) -> Self {
        Group::builder("Piccolo")
            .patterns(vec!["piccolo", "picc"])
            .build()
    }
}
