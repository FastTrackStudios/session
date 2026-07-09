//! Music production taxonomy — canonical definitions for instrument groups,
//! song sections, and their associated metadata (colors, abbreviations).
//!
//! This crate is the single source of truth for music production vocabulary
//! used across FastTrackStudio. Both `dynamic-template` (track classification)
//! and `keyflow-proto` (song structure) import from here.
//!
//! # Modules
//!
//! - [`instruments`] — Instrument groups and sub-groups with colors
//! - [`sections`] — Song sections with colors, abbreviations, and dynamic cues
//! - [`lookup`] — Name-to-color resolution with abbreviation/alias support
//! - [`groups`] — Canonical REAPER 128-slot group partition (instrument bands)

pub mod groups;
pub mod instruments;
pub mod lookup;
pub mod sections;

// Re-export the Color type for convenience
pub use color_palette::Color;
pub use color_palette::palette;
