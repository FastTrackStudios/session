//! Click track, count-in, and section guide generation algorithms.
//!
//! This module contains the pure-math scheduling logic ported from the legacy
//! FTS Guide plugin. It works entirely in quarter-note space — no sample-rate
//! or buffer-size dependencies.

pub mod click_scheduler;
pub mod count_in_pattern;
pub mod generator;
pub mod section_cue_scheduler;

pub use click_scheduler::ClickScheduler;
pub use count_in_pattern::CountInPattern;
pub use generator::GuideGenerator;
pub use section_cue_scheduler::SectionCueScheduler;
