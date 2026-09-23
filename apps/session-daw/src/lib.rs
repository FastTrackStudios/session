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
pub mod chart_panel;
pub mod cursor;
pub mod engine;
pub mod expression;
pub mod folder_item;
pub mod fps;
#[cfg(feature = "native")]
pub mod frame;
pub mod guide_instrument;
/// Lifts WebKitGTK's 60 fps rAF cap. Linux-only: it binds `webkit2gtk`.
/// (WKWebView on macOS has its own cap — 72 fps on a 144 Hz panel — and
/// its own switch; not done yet.)
#[cfg(target_os = "linux")]
pub mod frame_rate;
pub mod gesture;
#[cfg(feature = "native")]
pub mod headless;
pub mod hit;
pub mod icons;
pub mod keys;
pub mod layout;
pub mod live;
pub mod mcp;
pub mod midi;
pub mod mixer_panel;
pub mod mousemap;
pub mod notice;
pub mod num;
pub mod options;
#[cfg(feature = "native")]
pub mod open;
/// On the web there is no in-process tokio runtime: the engine clients
/// (`engine::Transport`, `Applier`, `Meters`, …) find none and run
/// read-only, the way they do natively before a session is open. The web
/// host drives the engine its own way (`web_host`).
#[cfg(not(feature = "native"))]
pub mod open {
    /// Never constructed: [`runtime`] has none to give.
    pub struct Runtime(());

    impl Runtime {
        pub fn block_on<F: core::future::Future>(&self, _future: F) -> F::Output {
            unreachable!("no runtime is ever handed out on the web")
        }
    }

    #[must_use]
    pub fn runtime() -> Option<&'static Runtime> {
        None
    }
}
pub mod overlay;
pub mod panel;
pub mod progress;
pub mod setlist;
pub mod setup;
pub mod shell;
#[cfg(feature = "native")]
pub mod patch_list;
pub mod plan;
pub mod prepare;
pub mod pointer;
pub mod profile;
pub mod rails;
#[cfg(feature = "native")]
pub mod session_file;
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
pub mod studio;
pub mod take_window;
pub mod tcp;
pub mod tempo_map;
pub mod text;
pub mod theme;
pub mod tone;
pub mod tool;
pub mod toolbar;
pub mod which_key;
pub mod transport_bar;
pub mod widget;
pub mod zoom;
#[cfg(feature = "web")]
pub mod web_engine;
#[cfg(feature = "web")]
pub mod web_audio;
#[cfg(feature = "web")]
pub mod web_host;
