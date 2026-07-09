//! Derive macro aliases for consistent type derivations.
//!
//! This module provides derive aliases using `macro_rules_attribute` to ensure
//! consistent derive patterns across the codebase and reduce boilerplate.
//!
//! # Usage
//!
//! ```ignore
//! use crate::engraver::derive_aliases::*;
//!
//! #[derive(Params!)]
//! pub struct ChordParams {
//!     pub notes: Vec<ChordNote>,
//!     pub stem_direction: Option<StemDirection>,
//! }
//!
//! #[derive(EnumVariant!)]
//! pub enum StemDirection {
//!     Up,
//!     Down,
//!     Auto,
//! }
//! ```
//!
//! # Available Aliases
//!
//! - `Params!` - Debug, Clone, Default - for layout parameter structs
//! - `LayoutOut!` - Debug, Clone - for layout output types
//! - `EnumVariant!` - Debug, Clone, Copy, PartialEq, Eq, Hash - for simple enums
//! - `EnumData!` - Debug, Clone, PartialEq - for enums with associated data
//! - `ModelType!` - Debug, Clone, Serialize, Deserialize - for data model types
//! - `StyleType!` - Debug, Clone, Copy, PartialEq, Default - for style properties
//! - `ConfigType!` - Debug, Clone, Default, Serialize, Deserialize - for configs
//! - `SceneNode!` - Debug, Clone - for scene graph types

use macro_rules_attribute::derive_alias;

// region:    --- Derive Aliases

derive_alias! {
    #[derive(Params!)] = #[derive(Debug, Clone, Default)];
    #[derive(LayoutOut!)] = #[derive(Debug, Clone)];
    #[derive(EnumVariant!)] = #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)];
    #[derive(EnumData!)] = #[derive(Debug, Clone, PartialEq)];
    #[derive(ModelType!)] = #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)];
    #[derive(StyleType!)] = #[derive(Debug, Clone, Copy, PartialEq, Default)];
    #[derive(ConfigType!)] = #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)];
    #[derive(SceneNode!)] = #[derive(Debug, Clone)];
}

// endregion: --- Derive Aliases

// region:    --- Tests

#[cfg(test)]
mod tests {
    // Derive aliases are tested implicitly when used throughout the codebase.
    // Direct testing of macro_rules_attribute derive aliases requires complex
    // re-export setup that isn't worth the complexity.
    //
    // Usage example (applied in other modules):
    // ```
    // use macro_rules_attribute::apply;
    // use crate::engraver::derive_aliases::*;
    //
    // #[apply(Params!)]
    // pub struct ChordParams { ... }
    // ```
}

// endregion: --- Tests
