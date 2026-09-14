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

use dynamic_template::scenes::Selector;
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

    /// Every rig entry as a `(selector, entry)` pair, in file order.
    ///
    /// The performer and the source kind become the selector's
    /// `performer` and `kind`; a key's `/`-separated segments are the
    /// group path over its last segment, the multi-mic — so `kick/in`
    /// is the `in` mic under the `kick` piece and `di` is the `di` mic
    /// with no path. Nothing is inferred beyond the split: the scene
    /// engine's matcher decides what a segment means against a
    /// session's taxonomy.
    // r[impl flow.patch-list.plan]
    #[must_use]
    pub fn entries(&self) -> Vec<Lowered> {
        let mut out = Vec::new();
        for (performer, rigs) in &self.performers {
            for (kind, rig) in rigs {
                for (key, entry) in rig {
                    out.push(Lowered {
                        performer: performer.clone(),
                        kind: kind.clone(),
                        key: key.clone(),
                        selector: selector_of(performer, kind, key),
                        entry: entry.clone(),
                    });
                }
            }
        }
        out
    }
}

/// One rig entry, flattened: where it sits in the list and the
/// selector it lowers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    /// The performer the rig belongs to.
    pub performer: String,
    /// The kind of source (`guitar`, `drums`, `bass`, `vocal`, `keys`).
    pub kind: String,
    /// The rig key as written (`di`, `amp-a/57`, `kick/in`).
    pub key: String,
    /// The same, in the scene engine's vocabulary.
    pub selector: Selector,
    /// What the channel or mic is patched to.
    pub entry: Entry,
}

impl Entry {
    /// The input role, when the entry names one — a MIDI entry is its
    /// own resolution and has none.
    #[must_use]
    pub fn role(&self) -> Option<&str> {
        match self {
            Self::Role(role) => Some(role),
            Self::Midi { .. } => None,
        }
    }
}

/// The selector for one rig key.
fn selector_of(performer: &str, kind: &str, key: &str) -> Selector {
    let mut segments: Vec<String> = key.split('/').map(str::to_owned).collect();
    let multi_mic = segments.pop();
    Selector {
        group: segments,
        kind: Some(kind.to_owned()),
        performer: Some(performer.to_owned()),
        multi_mic,
        ..Selector::default()
    }
}
