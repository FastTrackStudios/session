//! Per-extension data: what each extension keeps in the session, under its
//! own id, merged the way it chooses.
//!
//! An extension's namespace is a set of named entries. Each entry's kind is
//! its merge rule, chosen when the extension first writes it:
//!
//! | entry      | two people changing it at once                              | for                         |
//! |------------|-------------------------------------------------------------|-----------------------------|
//! | `Register` | last writer wins, for the whole value                       | a setting, a small record   |
//! | `Map`      | per key: different keys both land, one key is last-writer   | a mixer, a set of cues      |
//! | `Text`     | per character: both people's typing lands                   | Keyflow source, lyrics text |
//!
//! Timed things (a cue at bar 9, a chord, a clip) are not entries: they are
//! [`crate::Item`]s on the extension's own [`crate::Lane`]s, so every view
//! that follows the timeline can see *that* something is there and *when*,
//! without knowing what.
//!
//! Undo covers all of it: an entry is part of the one Architect document,
//! so an extension's step is undone like any other (see the design,
//! "Undo").

use std::collections::BTreeMap;

use crate::value::Value;

/// One named entry in an extension's namespace.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[repr(u8)]
pub enum Entry {
    /// One value; concurrent writes: the last one wins.
    Register(Value),
    /// Keys to values; concurrent writes to different keys both land.
    Map(BTreeMap<String, Value>),
    /// Text; concurrent edits merge per character.
    Text(String),
}

/// What one extension keeps in the session.
#[derive(Debug, Clone, PartialEq, Default, facet::Facet)]
pub struct ExtensionData {
    /// The extension's own schema version for this data: it migrates its
    /// own entries, the session never does.
    pub version: u32,
    pub entries: BTreeMap<String, Entry>,
}

impl ExtensionData {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// Sets register `name` to `value` (replacing whatever `name` was).
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.entries.insert(name.into(), Entry::Register(value));
    }

    /// Edits the map `name` (made empty first if it is not a map yet).
    pub fn edit_map(
        &mut self,
        name: impl Into<String>,
        edit: impl FnOnce(&mut BTreeMap<String, Value>),
    ) {
        let entry = self
            .entries
            .entry(name.into())
            .or_insert_with(|| Entry::Map(BTreeMap::new()));
        if !matches!(entry, Entry::Map(_)) {
            *entry = Entry::Map(BTreeMap::new());
        }
        if let Entry::Map(map) = entry {
            edit(map);
        }
    }

    /// Sets text `name` to `text`.
    pub fn set_text(&mut self, name: impl Into<String>, text: impl Into<String>) {
        self.entries.insert(name.into(), Entry::Text(text.into()));
    }

    /// The register `name`'s value.
    #[must_use]
    pub fn register(&self, name: &str) -> Option<&Value> {
        match self.entries.get(name)? {
            Entry::Register(v) => Some(v),
            Entry::Map(_) | Entry::Text(_) => None,
        }
    }

    /// The text `name`.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.entries.get(name)? {
            Entry::Text(t) => Some(t),
            Entry::Register(_) | Entry::Map(_) => None,
        }
    }

    /// The map `name`.
    #[must_use]
    pub fn map(&self, name: &str) -> Option<&BTreeMap<String, Value>> {
        match self.entries.get(name)? {
            Entry::Map(m) => Some(m),
            Entry::Register(_) | Entry::Text(_) => None,
        }
    }
}
