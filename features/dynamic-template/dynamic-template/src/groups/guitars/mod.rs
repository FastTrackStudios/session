//! Guitar-related group definitions

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

pub mod acoustic_guitar;
pub mod banjo;
pub mod electric_guitar;
pub mod mandolin;
pub mod steel_guitar;

pub use acoustic_guitar::AcousticGuitar;
pub use banjo::Banjo;
pub use electric_guitar::ElectricGuitar;
pub use steel_guitar::SteelGuitar;

/// Top-level guitars group containing all guitar types
pub struct Guitars;

impl From<Guitars> for Group<ItemMetadata> {
    fn from(_val: Guitars) -> Self {
        Group::builder("Guitars")
            .prefix("GTR")
            .patterns(vec!["guitar", "gtr", "gui"])
            // Negative patterns to avoid matching bass guitars
            .exclude(vec!["bass_guitar", "bassguitar", "bg"])
            .group(ElectricGuitar)
            .group(AcousticGuitar)
            .group(SteelGuitar)
            .group(Banjo)
            .build()
    }
}
