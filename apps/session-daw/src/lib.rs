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

pub mod animate;
pub mod arrange_edit;
pub mod arrangement;
pub mod art;
pub mod balance;
pub mod cursor;
pub mod engine;
pub mod expression;
pub mod folder_item;
pub mod fps;
pub mod frame;
pub mod guide_instrument;
/// Lifts WebKitGTK's 60 fps rAF cap. Linux-only: it binds `webkit2gtk`.
/// (WKWebView on macOS has its own cap — 72 fps on a 144 Hz panel — and
/// its own switch; not done yet.)
#[cfg(target_os = "linux")]
pub mod frame_rate;
pub mod gesture;
pub mod headless;
pub mod hit;
pub mod icons;
pub mod keys;
pub mod layout;
pub mod live;
pub mod mcp;
pub mod midi;
pub mod mousemap;
pub mod notice;
pub mod num;
pub mod open;
pub mod overlay;
pub mod patch_list;
pub mod plan;
pub mod prepare;
pub mod pointer;
pub mod profile;
pub mod rails;
pub mod rename;
pub mod repeat;
pub mod routes;
pub mod routing;
pub mod row;
pub mod ruler;
pub mod scrollbar;
pub mod settings;
pub mod simulate;
pub mod strip;
pub mod take_window;
pub mod tcp;
pub mod tempo_map;
pub mod text;
pub mod theme;
pub mod tone;
pub mod widget;
