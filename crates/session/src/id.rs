//! Stable ids: every entity is addressed by id, never by position.
//!
//! The session mints no ids itself: the caller does (a UUID, a REAPER
//! GUID, an id derived from what it imports), so the core needs no
//! randomness and a re-import can be idempotent. Studio's
//! `session-format` `EntityId` (a UUID string) and `SessionModel`'s guid
//! strings are both valid ids here, unchanged.

use std::fmt;

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        // Transparent: on every wire (JSON, vox) an id is the string it
        // wraps, so it works as a map key too.
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, facet::Facet)]
        #[facet(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// This id, as written.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// An extension: `daw`, `keyflow`, `ignition`, `signal`, `lyrics`.
    /// The same id the extension has in Architect (its manifest's `id`).
    ExtensionId
);
id!(
    /// A lane of the timeline.
    LaneId
);
id!(
    /// A timed item on a lane.
    ItemId
);
id!(
    /// A marker or a region.
    MarkId
);
