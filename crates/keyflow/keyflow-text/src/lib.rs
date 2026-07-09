//! Keyflow text parsing and highlighting helpers.
//!
//! This crate provides the text parser and related utilities that were
//! split out of `keyflow-proto`.

pub use keyflow_proto as proto;

pub mod chart;
pub mod chord {
    pub use keyflow_proto::chord::*;
}
pub mod key {
    pub use keyflow_proto::key::*;
}
pub mod metadata {
    pub use keyflow_proto::metadata::*;
}
pub mod parsing {
    pub use keyflow_syntax::parsing::*;
}
pub mod primitives {
    pub use keyflow_proto::primitives::*;
}
pub mod sections {
    pub use keyflow_proto::sections::*;
}
pub mod time {
    pub use keyflow_proto::time::*;
}

#[cfg(feature = "highlighting")]
pub mod highlighting {
    pub use keyflow_syntax::highlighting::*;
}

pub mod api;

/// IDE engine: structured diagnostics, completion, hover.
///
/// See module docs for the design. Powers both the in-process Dioxus editor
/// and the future LSP server.
pub mod ide;
