//! chord-tool — fire scale chords and notes into a project.
//!
//! A port of ChordGun (benjohnson2001/pandabot) in the sense that it does
//! the same job, not that it carries the same code across. ChordGun ships
//! its own scales, chord tables and degree arithmetic; this tree already
//! has all of that in `keyflow`, so [`theory`] resolves a scale degree
//! through keyflow instead. What actually needed porting is the workflow —
//! pick a key and a chord size, hear a degree, drop it at the cursor,
//! advance by the grid.
//!
//! The Lua original's `interface/` (Frame, Label, Dropdown, HitArea drawn
//! into a gfx buffer) has no counterpart here either: the panel is a
//! Dioxus component rendered by Blitz, living in `fts-extensions`.

pub mod theory;

pub use theory::{ChordSize, chord_notes, scale_chords, scale_note};
