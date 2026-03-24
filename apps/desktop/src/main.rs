//! Session Desktop — standalone Dioxus desktop app for session/setlist management.
//!
//! Provides the session UI (performance view, transport, setlist navigation) without
//! the full fts-control app. Runs session services in-process via LocalServices.

use dioxus::desktop::{tao::window::WindowBuilder, Config};
use dioxus::prelude::*;

use session_ui::{ConnectionState, Session, SessionShell};

mod gateway;
mod services;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,vox_core=warn,schema_deser=off")
            }),
        )
        .init();

    tracing::info!("Starting Session Desktop");

    let cfg = Config::new().with_window(
        WindowBuilder::new()
            .with_title("Session")
            .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1400.0, 900.0)),
    );

    LaunchBuilder::desktop().with_cfg(cfg).launch(App);
}

#[component]
fn App() -> Element {
    rsx! { DesktopSessionShell {} }
}

/// Main shell: starts gateway + REAPER connection, delegates UI to `SessionShell`.
#[component]
fn DesktopSessionShell() -> Element {
    let mut connection_state = use_signal(|| ConnectionState::Disconnected);
    let mut gateway_info: Signal<Option<gateway::GatewayInfo>> = use_signal(|| None);

    // Start the gateway immediately (serves web app even without REAPER)
    let _gateway = use_future(move || async move {
        match services::start_gateway().await {
            Ok(info) => {
                tracing::info!("Gateway started on port {}", info.port);
                gateway_info.set(Some(info));
            }
            Err(e) => {
                tracing::error!("Failed to start gateway: {e}");
            }
        }
    });

    // Connect to REAPER in the background — retries until found
    let _reaper = use_future(move || async move {
        loop {
            connection_state.set(ConnectionState::Connecting);
            match services::connect_to_reaper().await {
                Ok(()) => {
                    connection_state.set(ConnectionState::Connected);
                    tracing::info!("Connected to REAPER");
                    return;
                }
                Err(e) => {
                    tracing::warn!("Waiting for REAPER: {e}");
                    connection_state.set(ConnectionState::Disconnected);
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    });

    // Subscribe to setlist events once connected
    let _subscription = use_future(move || async move {
        // Wait until connected
        loop {
            if connection_state() == ConnectionState::Connected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let session = Session::get();

        loop {
            if let Err(e) = session.setlist().build_from_open_projects().await {
                tracing::warn!("build_from_open_projects failed: {e:?}");
            }

            let (tx, mut rx) = vox::channel::<session::SetlistEvent>();

            if let Err(e) = session.setlist().subscribe(tx).await {
                tracing::error!("Failed to subscribe to setlist events: {e:?}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }

            tracing::info!("Subscribed to setlist events");

            // Periodically re-scan REAPER for new/closed projects
            let poll_session = session.clone();
            let poll_handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if let Err(e) = poll_session.setlist().build_from_open_projects().await {
                        tracing::debug!("Periodic project scan failed: {e:?}");
                    }
                }
            });

            let web_registry = gateway::web_client_registry();

            while let Ok(Some(event_ref)) = rx.recv().await {
                web_registry.broadcast(&event_ref).await;
                session_ui::apply_setlist_event(&event_ref);
            }

            poll_handle.abort();
            tracing::info!("Setlist event subscription ended, will retry...");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        SessionShell { connection_state }
        // Desktop-only: connection popover overlay
        DesktopConnectionOverlay { connection_state, gateway_info }
    }
}

/// Desktop-only connection overlay — adds gateway info popover on top of the shared shell.
/// This is rendered as a sibling to `SessionShell`, positioned absolutely over the badge area.
#[component]
fn DesktopConnectionOverlay(
    connection_state: Signal<ConnectionState>,
    gateway_info: Signal<Option<gateway::GatewayInfo>>,
) -> Element {
    let mut show_popover = use_signal(|| false);

    // Only show the overlay button area if there's gateway info to display
    let has_gateway_info = gateway_info.read().is_some();
    if !has_gateway_info {
        return rsx! {};
    }

    rsx! {
        // Invisible click-target positioned over the connection badge area (top-right)
        div {
            class: "fixed top-0 right-0 h-10 z-30 flex items-center pr-4",
            // Clickable area that matches the badge location
            button {
                class: "w-32 h-8 cursor-pointer opacity-0",
                onclick: move |_| show_popover.toggle(),
            }
        }

        if show_popover() {
            // Backdrop
            div {
                class: "fixed inset-0 z-40",
                onclick: move |_| show_popover.set(false),
            }

            // Popover panel
            div {
                class: "fixed right-4 top-12 z-50 w-72 rounded-lg border border-border bg-card shadow-xl p-3 text-sm",

                div { class: "font-medium text-card-foreground mb-3", "Connection Info" }

                // REAPER connection
                div { class: "flex items-center justify-between py-1.5",
                    span { class: "text-muted-foreground", "REAPER" }
                    match connection_state() {
                        ConnectionState::Connected => rsx! {
                            span { class: "text-green-400 font-medium", "Connected" }
                        },
                        ConnectionState::Connecting => rsx! {
                            span { class: "text-yellow-400 font-medium", "Connecting\u{2026}" }
                        },
                        ConnectionState::Disconnected => rsx! {
                            span { class: "text-red-400 font-medium", "Disconnected" }
                        },
                    }
                }

                div { class: "border-t border-border my-2" }

                if let Some(ref info) = *gateway_info.read() {
                    div { class: "space-y-1.5",
                        div { class: "text-xs font-medium text-muted-foreground uppercase tracking-wider mb-1",
                            "Web Gateway"
                        }

                        div { class: "flex items-center justify-between py-0.5",
                            span { class: "text-muted-foreground", "Port" }
                            span { class: "font-mono text-card-foreground", "{info.port}" }
                        }

                        div { class: "flex items-center justify-between py-0.5",
                            span { class: "text-muted-foreground", "Local" }
                            span { class: "font-mono text-blue-400 text-xs", "{info.local_url()}" }
                        }

                        if let Some(url) = info.network_url() {
                            div { class: "flex items-center justify-between py-0.5",
                                span { class: "text-muted-foreground", "Network" }
                                span { class: "font-mono text-blue-400 text-xs", "{url}" }
                            }
                        }

                        div { class: "flex items-center justify-between py-0.5",
                            span { class: "text-muted-foreground", "Web App" }
                            if info.serving_web_app {
                                span { class: "text-green-400", "Serving" }
                            } else {
                                span { class: "text-yellow-400", "Not built" }
                            }
                        }
                    }
                }
            }
        }
    }
}
