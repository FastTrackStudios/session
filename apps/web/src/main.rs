//! session.fasttrackstudio.app
//!
//! Two things: a landing page that shows what Session is, and a live demo
//! setlist — the real in-process `session` + `daw-standalone` backend
//! running in the browser, no server, no account. `/demo` is the same
//! `PerformanceLayout` the desktop app ships, driven by the real setlist
//! RPC surface (see [`demo_backend`]) rather than a picture of it.

mod demo_backend;
mod routes;

use dioxus::prelude::*;

#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    Home {},
    #[route("/demo")]
    Demo {},
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

use routes::{Demo, Home, NotFound};

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        // INFO, not TRACE — dioxus traces every template/signal/mount, and
        // the demo's transport re-renders continuously once playing.
        tracing_wasm::set_as_global_default_with_config(
            tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(tracing::Level::INFO)
                .build(),
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    tracing_subscriber::fmt::init();

    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Title { "Session — your setlist, live" }
        // Dark-only site: force it ahead of any CSS load so native chrome
        // (form controls, scrollbars) never flashes light before
        // tailwind.css's `color-scheme: dark` takes over.
        document::Meta { name: "color-scheme", content: "dark" }
        // The desktop app's own launcher icon (apps/desktop/assets/icon.svg)
        // — one FTS icon across every surface, not a bespoke site mark.
        document::Link { rel: "icon", r#type: "image/svg+xml", href: asset!("/assets/favicon.svg") }
        document::Link { rel: "preconnect", href: "https://fonts.googleapis.com" }
        document::Link {
            rel: "preconnect",
            href: "https://fonts.gstatic.com",
            crossorigin: "anonymous",
        }
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.googleapis.com/css2?family=Geist:wght@300..800&family=Geist+Mono:wght@400..600&display=swap",
        }
        document::Link { rel: "stylesheet", href: asset!("/assets/tailwind.css") }
        Router::<Route> {}
    }
}
