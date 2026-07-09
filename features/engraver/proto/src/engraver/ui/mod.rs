//! UI components for music notation rendering.
//!
//! This module provides reusable UI components like labels, capsules,
//! and other visual elements.

pub mod capsule_label;

pub use capsule_label::{
    CapsuleLabelConfig, CapsuleLabelMode, ComputedCapsuleLabel, format_rehearsal_label,
    format_rehearsal_label_with_letter,
};
