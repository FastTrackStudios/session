//! The Session DAW: the arrangement, the mixer and the panels around them.
//!
//! The arrangement is painted by one thing, [`widget::ArrangementWidget`],
//! mounted by [`panel::use_arrangement_panel`] and [`panel::PanelChrome`]
//! wherever the app shows it — the desktop shell on dioxus-native, the
//! browser through `web_host`. The headless tools (`bin/bench`,
//! `bin/blitz_shot`) draw through the same widget, so a benchmark or a
//! screenshot is a picture of the window rather than of a copy of it.

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
pub mod guide_instrument;
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
pub mod chart_editor;
#[cfg(feature = "native")]
pub mod organize;
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
pub mod routes;
pub mod row;
pub mod ruler;
pub mod settings;
pub mod simulate;
pub mod strip;
pub mod studio;
pub mod take_window;
pub mod tcp;
pub mod text;
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
