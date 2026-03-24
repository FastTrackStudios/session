//! Session Shell
//!
//! The complete session UI shell: top bar with branding and connection badge,
//! dock layout with Navigator | Performance + Transport panels, and
//! connection state screens (connecting / disconnected).
//!
//! Both the standalone session apps and the full FastTrackStudio app use this
//! shell — they just provide the connection state from their own backends.

use std::rc::Rc;

use dock_dioxus::{DockProvider, DockRoot, PanelRenderer, PanelRendererRegistry, DOCK_WORKSPACE};
use dock_proto::builder::DockLayoutBuilder as B;
use dock_proto::panel::PanelId;
use dock_proto::workspace::{DockWorkspace, WindowBounds};
use dock_proto::PanelRegistry;

use crate::layouts::top_bar::{ConnectionState, VERSION};
use crate::prelude::*;

/// The main session shell component.
///
/// Renders the full session UI: top bar, connection states, and dock layout.
/// The caller is responsible for driving the `connection_state` signal from
/// whatever backend they use (REAPER socket, WebSocket, etc.).
///
/// # Usage
///
/// ```rust,ignore
/// use session_ui::{SessionShell, ConnectionState};
///
/// let connection_state = use_signal(|| ConnectionState::Connecting);
/// rsx! {
///     SessionShell { connection_state }
/// }
/// ```
#[component]
pub fn SessionShell(
    /// Reactive connection state driven by the host app.
    connection_state: Signal<ConnectionState>,
) -> Element {
    // Initialize dock layout once
    use_hook(|| {
        let layout = B::horizontal()
            .left(B::tile(PanelId::Navigator))
            .right(
                B::vertical()
                    .top(B::tile(PanelId::Performance))
                    .bottom(B::tile(PanelId::Transport))
                    .ratio(80.0)
                    .build_node(),
            )
            .ratio(20.0)
            .build();

        let mut registry = PanelRegistry::new();
        PanelId::register_all(&mut registry);

        let workspace = DockWorkspace::with_main_window(
            layout,
            "Session",
            WindowBounds::new(0.0, 0.0, 1400.0, 900.0, "main"),
            registry,
        );

        *DOCK_WORKSPACE.write() = workspace;
    });

    // Panel renderer — register session panels
    let render_panel = use_hook(|| {
        let mut registry = PanelRendererRegistry::new();
        crate::register_panels(&mut registry);

        let registry = Rc::new(registry);
        PanelRenderer::new(move |panel_id| registry.render(panel_id))
    });

    rsx! {
        div { class: "h-screen flex flex-col bg-background text-foreground",

            // ── Top bar ────────────────────────────────────────────
            div {
                class: "h-10 flex-shrink-0 border-b border-border bg-card flex items-center px-4",

                // Left: branding
                div { class: "flex items-center gap-1.5",
                    span { class: "text-sm font-bold bg-gradient-to-r from-blue-400 to-violet-400 bg-clip-text text-transparent", "FTS" }
                    span { class: "text-sm italic text-card-foreground", "Session" }
                }

                div { class: "ml-auto" }

                // Right: version + connection badge
                div { class: "flex items-center gap-3",
                    span { class: "text-xs text-muted-foreground font-mono", "{VERSION}" }
                    ConnectionBadge { state: connection_state() }
                }
            }

            // ── Main content ───────────────────────────────────────
            if connection_state() == ConnectionState::Connected {
                DockProvider { render_panel: render_panel.clone(),
                    div { class: "flex-1 overflow-hidden relative",
                        DockRoot {}
                    }
                }
            } else if connection_state() == ConnectionState::Disconnected {
                div { class: "flex-1 flex flex-col items-center justify-center gap-4",
                    div { class: "w-6 h-6 rounded-full bg-red-400/20 flex items-center justify-center",
                        div { class: "w-2.5 h-2.5 rounded-full bg-red-400" }
                    }
                    p { class: "text-sm text-red-400 font-medium", "Disconnected" }
                    p { class: "text-xs text-muted-foreground", "Waiting for connection\u{2026}" }
                }
            } else {
                // Connecting
                div { class: "flex-1 flex flex-col items-center justify-center gap-4",
                    div {
                        class: "w-8 h-8 border-2 border-yellow-400 border-t-transparent rounded-full animate-spin",
                    }
                    p { class: "text-sm text-muted-foreground", "Connecting\u{2026}" }
                }
            }
        }
    }
}

/// Connection status badge with spinner for connecting state.
#[component]
pub fn ConnectionBadge(state: ConnectionState) -> Element {
    let (bg_class, text_class, dot_class, label) = match state {
        ConnectionState::Connected => (
            "bg-green-500/20",
            "text-green-400",
            "bg-green-400",
            "Connected",
        ),
        ConnectionState::Connecting => (
            "bg-yellow-500/20",
            "text-yellow-400",
            "",
            "Connecting\u{2026}",
        ),
        ConnectionState::Disconnected => (
            "bg-red-500/20",
            "text-red-400",
            "bg-red-400",
            "Disconnected",
        ),
    };

    rsx! {
        div {
            class: "flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium {bg_class} {text_class}",

            match state {
                ConnectionState::Connecting => rsx! {
                    div {
                        class: "w-3 h-3 border-2 border-current border-t-transparent rounded-full animate-spin",
                    }
                },
                _ => rsx! {
                    div { class: "w-2 h-2 rounded-full {dot_class}" }
                },
            }

            "{label}"
        }
    }
}
