//! Acoustic guitar group definition

use super::mandolin::Mandolin;
use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Acoustic guitar group
pub struct AcousticGuitar;

impl From<AcousticGuitar> for Group<ItemMetadata> {
    fn from(_val: AcousticGuitar) -> Self {
        Group::builder("Acoustic")
            .prefix("AG")
            .patterns(vec![
                // Generic acoustic patterns
                "acoustic",
                "ag",  // Common abbreviation for Acoustic Guitar
                "aco", // Another common abbreviation
                "acc",
                "nylon",
                "classical",
                "fingerpick",
                // Gibson acoustic models
                "J160",
                "J-160",
                "J45",
                "J-45",
                "J200",
                "J-200",
                "Hummingbird",
                // Martin acoustic models
                "D28",
                "D-28",
                "D18",
                "D-18",
                "OM28",
                "OM-28",
                "HD28",
                "HD-28",
                // Taylor acoustic models
                "Taylor",
                "814ce",
                "714ce",
                "214ce",
                // Other acoustic brands/models
                "Framus",
                "Guild",
                "Takamine",
                "Ovation",
                "Seagull",
                "Yamaha FG",
                "Epiphone",
            ])
            .group(Mandolin)
            .build()
    }
}
