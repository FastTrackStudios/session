//! What an extension stores: a value the session carries and never reads.
//!
//! The shape is Loro's value model (null, bool, i64, f64, string, bytes,
//! list, map), so it maps onto an Architect document one to one, and onto
//! JSON for a runtime extension. Large or binary data (an FX chain's state,
//! a MIDI take, audio) does not go in a value: it is a blob, and the value
//! holds its sha256 (`docs/design/session.md`, "Blobs").

use std::collections::BTreeMap;

/// A value an extension stores. The session does not interpret it.
#[derive(Debug, Clone, PartialEq, Default, facet::Facet)]
#[repr(u8)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<Self>),
    Map(BTreeMap<String, Self>),
}

impl Value {
    /// A map built from pairs.
    #[must_use]
    pub fn map<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Self)>) -> Self {
        Self::Map(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// The entry `key` of a map; `None` for a missing key or a non-map.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Map(m) => m.get(key),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
