//! Session Web App
//!
//! Dioxus-based web app for controlling session playback via the gateway-ws server.
//! Uses the shared `SessionShell` from session-ui for layout and UI.

#![cfg_attr(not(target_arch = "wasm32"), allow(unused))]

#[cfg(target_arch = "wasm32")]
mod connection;
#[cfg(target_arch = "wasm32")]
mod web_client_handler;

#[cfg(target_arch = "wasm32")]
use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
use session_ui::SessionShell;

#[cfg(target_arch = "wasm32")]
const FAVICON: Asset = asset!("/assets/favicon.ico");
#[cfg(target_arch = "wasm32")]
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        tracing_wasm::set_as_global_default_with_config(
            tracing_wasm::WASMLayerConfigBuilder::default()
                .set_max_level(tracing::Level::INFO)
                .build(),
        );
        dioxus::launch(App);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("This binary only runs on wasm32. Use `dx build` to compile for WASM.");
    }
}

#[cfg(target_arch = "wasm32")]
#[component]
fn App() -> Element {
    let (connection_state, connect) = connection::use_connection();

    // Auto-connect on mount
    use_effect(move || {
        connect.call(());
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: TAILWIND_CSS }
        SessionShell { connection_state }
    }
}
