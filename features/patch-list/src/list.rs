//! The album file: `patch-list.styx`.
//!
//! One shape (decision #29): performer → source kind → channel or mic →
//! input role, and the headphone buses listed the same way with who
//! each is for. A kit is the drummer's rig of kind `drums` keyed
//! `piece/mic` (`kick/in`, `snare/top`); a bass is a rig of kind `bass`
//! keyed by channel; a MIDI source is an entry that names the device
//! and channel directly instead of a role.
//!
//! Every map is insertion-ordered: the file's order is the order the
//! view lists performers in and the scene engine orders performer rows
//! by (`flow.scenes.performer-order`).

use facet::Facet;
use indexmap::IndexMap;

/// The whole album file.
// r[impl flow.patch-list.plan]
#[derive(Facet, Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchList {
    /// Each performer's rig, by performer name.
    #[facet(default)]
    pub performers: IndexMap<String, Performer>,
    /// The headphone buses, by bus name (`cody`, `engineer`,
    /// `broadcast`, `choir`).
    #[facet(default)]
    pub headphones: IndexMap<String, Bus>,
}

/// One performer: a rig per kind of source they record.
pub type Performer = IndexMap<String, Rig>;

/// A rig: each channel or mic of one kind of source, and what it is
/// patched to.
pub type Rig = IndexMap<String, Entry>;

/// What a channel or mic is patched to.
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
#[facet(rename_all = "kebab-case")]
#[repr(u8)]
pub enum Entry {
    /// A MIDI source, named directly: `@midi{device "Nord", channel 1}`.
    /// No channel means every channel.
    Midi {
        device: String,
        #[facet(default)]
        channel: Option<u8>,
    },
    /// An input role the studio profile resolves: `"DI 3"`, `"Kick In"`.
    #[facet(other)]
    Role(String),
}

/// A headphone bus: the output role it goes to, and who hears it.
#[derive(Facet, Debug, Clone, Default, PartialEq, Eq)]
pub struct Bus {
    /// The bus role in the profile's outputs table (`"HP 1"`).
    pub output: String,
    /// Who the bus is for — one name, or several for a shared bus.
    #[facet(rename = "for", default)]
    pub audience: Vec<String>,
}

impl PatchList {
    /// Parse an album file.
    ///
    /// # Errors
    ///
    /// When the text is not a patch list in this shape.
    pub fn from_styx(text: &str) -> Result<Self, facet_styx::DeserializeError> {
        facet_styx::from_str(text)
    }

    /// Write the list back as styx.
    ///
    /// Absent options are omitted, the way a hand-written file leaves
    /// them out, so what is written reads back.
    ///
    /// # Errors
    ///
    /// When the value cannot be serialized — which a list built from
    /// these types cannot fail at.
    pub fn to_styx(
        &self,
    ) -> Result<String, facet_styx::SerializeError<facet_styx::StyxSerializeError>> {
        facet_styx::to_string_with_options(
            self,
            &facet_styx::SerializeOptions::default().omit_none(),
        )
    }
}
