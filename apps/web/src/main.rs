//! Session Web App
//!
//! Dioxus-based web app for controlling session playback via the gateway-ws server.
//! Shows the performance view with a navigator sidebar and transport controls.

#![cfg_attr(not(target_arch = "wasm32"), allow(unused))]

#[cfg(target_arch = "wasm32")]
mod connection;
#[cfg(target_arch = "wasm32")]
mod web_client_handler;

#[cfg(target_arch = "wasm32")]
use dioxus::prelude::*;

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
    use session_ui::{ConnectionState, PerformanceLayout, PerformanceSidebar, TransportPanel};

    let (connection_state, connect) = connection::use_connection();

    // Auto-connect on mount
    use_effect(move || {
        connect.call(());
    });

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }

        div {
            class: "flex flex-col h-screen w-full bg-background text-foreground",

            // Top bar with connection status
            div {
                class: "flex items-center justify-between px-4 py-2 border-b border-border bg-card",

                div {
                    class: "flex items-center gap-2",
                    span { class: "text-sm font-semibold", "Session" }
                }

                div {
                    class: "flex items-center gap-2",
                    match connection_state() {
                        ConnectionState::Connected => rsx! {
                            div { class: "w-2 h-2 rounded-full bg-green-500" }
                            span { class: "text-xs text-muted-foreground", "Connected" }
                        },
                        ConnectionState::Connecting => rsx! {
                            div { class: "w-2 h-2 rounded-full bg-yellow-500 animate-pulse" }
                            span { class: "text-xs text-muted-foreground", "Connecting..." }
                        },
                        ConnectionState::Disconnected => rsx! {
                            div { class: "w-2 h-2 rounded-full bg-red-500" }
                            span { class: "text-xs text-muted-foreground", "Disconnected" }
                        },
                    }
                }
            }

            if matches!(connection_state(), ConnectionState::Connected) {
                // Main area: sidebar + content
                div {
                    class: "flex flex-1 overflow-hidden",

                    // Left sidebar: navigator
                    div {
                        class: "w-64 flex-shrink-0 border-r border-border overflow-y-auto",
                        PerformanceSidebar {}
                    }

                    // Content area: performance view + transport
                    div {
                        class: "flex flex-col flex-1 overflow-hidden",

                        div {
                            class: "flex-1 overflow-hidden",
                            PerformanceLayout {}
                        }

                        // Transport controls at bottom
                        div {
                            class: "border-t border-border h-20",
                            TransportPanel {}
                        }
                    }
                }
            } else {
                // Waiting for connection
                div {
                    class: "flex-1 flex items-center justify-center",
                    div {
                        class: "text-center text-muted-foreground",
                        p { class: "text-lg", "Connecting to Session..." }
                        p { class: "text-sm mt-2", "Waiting for desktop app" }
                    }
                }
            }
        }
    }
}
