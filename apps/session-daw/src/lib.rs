//! The Session DAW window, in two renderings.
//!
//! - `main.rs` — the dioxus/WebView studio. What runs today.
//! - `bin/vello.rs` — the GPU arrangement, drawn through `anyrender`
//!   onto a wgpu surface. Where this is going.
//!
//! They share everything below the UI: opening a project, standing up
//! the `daw` facade, and resolving the theme. Only the drawing differs,
//! which is the point of keeping both runnable while the second comes
//! up.

pub mod arrangement;
#[cfg(target_os = "linux")]
pub mod cursor;
pub mod frame_rate;
pub mod gesture;
pub mod headless;
pub mod hit;
pub mod layout;
pub mod mcp;
pub mod num;
pub mod open;
pub mod art;
pub mod profile;
pub mod rails;
pub mod ruler;
pub mod settings;
pub mod tcp;
pub mod text;
pub mod theme;
pub mod tone;
