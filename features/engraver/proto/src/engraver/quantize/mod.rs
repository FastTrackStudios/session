//! MIDI-to-notation rhythm quantization.
//!
//! This module provides tools for converting raw MIDI tick durations into
//! standard notation durations, including automatic detection of tuplets
//! (triplets, quintuplets, etc.) and handling of timing tolerance.
//!
//! # Overview
//!
//! The quantization process has three main components:
//!
//! 1. **Configuration** ([`QuantizeConfig`]) - Controls PPQ resolution, tolerance,
//!    and which tuplet types to detect.
//!
//! 2. **Detection** ([`quantize_duration`], [`quantize_duration_batch`]) -
//!    Converts tick durations to notation durations, identifying the best match
//!    from standard and tuplet candidates.
//!
//! 3. **Grouping** ([`detect_tuplet_groups`]) - Identifies complete tuplet groups
//!    (e.g., three consecutive triplet eighths) for proper notation rendering.
//!
//! # Example
//!
//! ```ignore
//! use engraver::quantize::{QuantizeConfig, quantize_duration_batch, detect_tuplet_groups};
//!
//! // Configure for REAPER's 960 PPQ
//! let config = QuantizeConfig::reaper();
//!
//! // MIDI note durations (triplet eighths at 960 PPQ)
//! let durations = vec![320, 320, 320]; // 3 notes, each 320 ticks
//! let positions = vec![0, 320, 640];
//!
//! // Quantize to notation
//! let quantized = quantize_duration_batch(&durations, &positions, &config);
//!
//! // Detect tuplet groups
//! let groups = detect_tuplet_groups(&quantized, &positions, &config);
//!
//! // Use with MeasureBuilder
//! for group in &groups {
//!     builder = builder.tuplet_group(group.start_idx, group.end_idx, group.ratio);
//! }
//! ```
//!
//! # Supported Tuplets (matches MuseScore)
//!
//! - **Triplet (3:2)** - 3 notes in the space of 2
//! - **Quintuplet (5:4)** - 5 notes in the space of 4
//! - **Sextuplet (6:4)** - 6 notes in the space of 4
//! - **Septuplet (7:8)** - 7 notes in the space of 8
//!
//! # Tick Reference (480 PPQ)
//!
//! | Duration | Standard | Dotted | Triplet (3:2) | Quintuplet (5:4) | Septuplet (7:8) |
//! |----------|----------|--------|---------------|------------------|-----------------|
//! | Whole    | 1920     | 2880   | 1280          | 1536             | 2194            |
//! | Half     | 960      | 1440   | 640           | 768              | 1097            |
//! | Quarter  | 480      | 720    | 320           | 384              | 548             |
//! | Eighth   | 240      | 360    | 160           | 192              | 274             |
//! | Sixteenth| 120      | 180    | 80            | 96               | 137             |
//! | 32nd     | 60       | 90     | 40            | 48               | 68              |

mod config;
mod detector;
mod grouper;

pub use config::QuantizeConfig;
pub use detector::{QuantizedDuration, TupletType, quantize_duration, quantize_duration_batch};
pub use grouper::{TupletGroup, detect_tuplet_groups, merge_adjacent_groups};
