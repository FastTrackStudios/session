//! Session Desktop — standalone Dioxus desktop app for session/setlist management.
//!
//! Provides the session UI (performance view, transport, setlist navigation) without
//! the full fts-control app. Runs session services in-process via LocalServices.

use std::rc::Rc;

use dioxus::desktop::{tao::window::WindowBuilder, Config};
use dioxus::prelude::*;

use dock_dioxus::{DockProvider, DockRoot, PanelRenderer, PanelRendererRegistry, DOCK_WORKSPACE};
use dock_proto::builder::DockLayoutBuilder as B;
use dock_proto::panel::PanelId;
use dock_proto::workspace::{DockWorkspace, WindowBounds};
use dock_proto::PanelRegistry;

use session_ui::{
    ConnectionState, Session, TransportState, ACTIVE_INDICES, ACTIVE_PLAYBACK_IS_PLAYING,
    ACTIVE_PLAYBACK_MUSICAL, PLAYBACK_STATE, SETLIST_STRUCTURE, SONG_CHARTS, SONG_TRANSPORT,
    VERSION,
};

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
    rsx! { SessionShell {} }
}

/// Main shell: single top bar + resizable dock panels (Navigator | Performance + Transport).
#[component]
fn SessionShell() -> Element {
    // Connection state — starts as Connecting, moves to Connected once services init
    let mut connection_state = use_signal(|| ConnectionState::Connecting);

    // Initialize services in the background, update connection state when done
    let _services = use_future(move || async move {
        match services::init_session_services().await {
            Ok(()) => {
                connection_state.set(ConnectionState::Connected);
                tracing::info!("Connection state: Connected");
            }
            Err(e) => {
                tracing::error!("Failed to initialize services: {e}");
                connection_state.set(ConnectionState::Disconnected);
            }
        }
    });

    // Subscribe to setlist events once connected — bridges service events to UI signals.
    // Retries periodically so that if no setlist exists yet (e.g. no projects open),
    // it picks up the setlist once one becomes available.
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
            // Try to (re)build the setlist from REAPER's open projects
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

            while let Ok(Some(event_ref)) = rx.recv().await {
                match &*event_ref {
                    session::SetlistEvent::SetlistChanged(setlist) => {
                        tracing::debug!("Setlist changed: {} songs", setlist.songs.len());
                        let valid_guids: std::collections::HashSet<String> = setlist
                            .songs
                            .iter()
                            .map(|song| song.project_guid.clone())
                            .collect();
                        SONG_CHARTS
                            .write()
                            .retain(|guid, _| valid_guids.contains(guid));
                        *SETLIST_STRUCTURE.write() = setlist.clone();
                    }

                    session::SetlistEvent::SongHydrated { index, song, .. } => {
                        let mut setlist = SETLIST_STRUCTURE.write();
                        if *index < setlist.songs.len() {
                            setlist.songs[*index] = song.clone();
                        }
                    }

                    session::SetlistEvent::SongChartHydrated { chart, .. } => {
                        SONG_CHARTS
                            .write()
                            .insert(chart.project_guid.clone(), chart.clone());
                    }

                    session::SetlistEvent::ActiveIndicesChanged(indices) => {
                        *PLAYBACK_STATE.write() = if indices.is_playing {
                            daw::service::PlayState::Playing
                        } else {
                            daw::service::PlayState::Stopped
                        };
                        *ACTIVE_INDICES.write() = indices.clone();
                    }

                    session::SetlistEvent::TransportUpdate(transports) => {
                        let active_song_index = ACTIVE_INDICES.read().song_index;

                        let mut transport_updates: Vec<(usize, TransportState)> =
                            Vec::with_capacity(transports.len());
                        let mut active_transport_update = None;

                        {
                            let setlist = SETLIST_STRUCTURE.read();
                            let existing = SONG_TRANSPORT.read();

                            for transport in transports {
                                let loop_region_pct =
                                    transport.loop_region.as_ref().and_then(|region| {
                                        setlist.songs.get(transport.song_index).map(|song| {
                                            let dur = song.duration();
                                            if dur > 0.0 {
                                                (
                                                    (region.start_seconds / dur).clamp(0.0, 1.0),
                                                    (region.end_seconds / dur).clamp(0.0, 1.0),
                                                )
                                            } else {
                                                (0.0, 1.0)
                                            }
                                        })
                                    });

                                let next_state = TransportState {
                                    position: transport.position.clone(),
                                    bpm: transport.bpm,
                                    time_sig_num: transport.time_sig_num as i32,
                                    time_sig_denom: transport.time_sig_denom as i32,
                                    is_playing: transport.is_playing,
                                    is_looping: transport.is_looping,
                                    loop_region: loop_region_pct,
                                };

                                let changed = existing
                                    .get(&transport.song_index)
                                    .map(|e| *e != next_state)
                                    .unwrap_or(true);

                                if changed {
                                    transport_updates.push((transport.song_index, next_state));
                                }

                                if Some(transport.song_index) == active_song_index {
                                    active_transport_update = Some((
                                        transport.progress,
                                        transport.section_progress,
                                        transport.section_index,
                                        transport.is_playing,
                                        transport.is_looping,
                                        transport.position.musical.clone(),
                                    ));
                                }
                            }
                        }

                        if !transport_updates.is_empty() {
                            let mut song_transport = SONG_TRANSPORT.write();
                            for (idx, state) in transport_updates {
                                song_transport.insert(idx, state);
                            }
                        }

                        if let Some((
                            song_progress,
                            section_progress,
                            section_index,
                            is_playing,
                            is_looping,
                            musical,
                        )) = active_transport_update
                        {
                            if *ACTIVE_PLAYBACK_MUSICAL.peek() != musical {
                                *ACTIVE_PLAYBACK_MUSICAL.write() = musical;
                            }
                            if *ACTIVE_PLAYBACK_IS_PLAYING.peek() != is_playing {
                                *ACTIVE_PLAYBACK_IS_PLAYING.write() = is_playing;
                            }

                            let old_playing = *PLAYBACK_STATE.read();
                            let new_playing = if is_playing {
                                daw::service::PlayState::Playing
                            } else {
                                daw::service::PlayState::Stopped
                            };
                            if old_playing != new_playing {
                                *PLAYBACK_STATE.write() = new_playing;
                            }

                            let indices_changed = {
                                let current = ACTIVE_INDICES.read();
                                current.song_progress != Some(song_progress)
                                    || current.section_progress != section_progress
                                    || current.section_index != section_index
                                    || current.is_playing != is_playing
                                    || current.looping != is_looping
                            };

                            if indices_changed {
                                let mut indices = ACTIVE_INDICES.write();
                                indices.song_progress = Some(song_progress);
                                indices.section_progress = section_progress;
                                indices.section_index = section_index;
                                indices.is_playing = is_playing;
                                indices.looping = is_looping;
                            }
                        }
                    }

                    session::SetlistEvent::SongEntered { index, song, .. } => {
                        tracing::debug!("Entered song {}: {}", index, song.name);
                    }

                    session::SetlistEvent::SongExited { index, .. } => {
                        tracing::debug!("Exited song {}", index);
                    }

                    session::SetlistEvent::SectionEntered {
                        song_index,
                        section_index,
                        section,
                        ..
                    } => {
                        tracing::debug!(
                            "Entered section {}.{}: {}",
                            song_index,
                            section_index,
                            section.name
                        );
                    }

                    session::SetlistEvent::SectionExited {
                        song_index,
                        section_index,
                        ..
                    } => {
                        tracing::debug!("Exited section {}.{}", song_index, section_index);
                    }

                    session::SetlistEvent::PositionChanged { .. } => {
                        // Legacy event, ignored — we use TransportUpdate now
                    }
                }
            }

            tracing::info!("Setlist event subscription ended, will retry...");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    // Initialize dock with a session-specific layout (no preset bar, no chart tabs)
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
        session_ui::register_panels(&mut registry);

        let registry = Rc::new(registry);
        PanelRenderer::new(move |panel_id| registry.render(panel_id))
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }

        div { class: "h-screen flex flex-col bg-background text-foreground",

            // ── Single top bar ───────────────────────────────────────
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

            // ── Main content ─────────────────────────────────────────
            if connection_state() == ConnectionState::Connected {
                // Resizable dock layout
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
                    p { class: "text-sm text-red-400 font-medium", "Failed to connect" }
                    p { class: "text-xs text-muted-foreground", "Check that REAPER is running with the FTS extension" }
                }
            } else {
                // Connecting — loading screen
                div { class: "flex-1 flex flex-col items-center justify-center gap-4",
                    div {
                        class: "w-8 h-8 border-2 border-yellow-400 border-t-transparent rounded-full animate-spin",
                    }
                    p { class: "text-sm text-muted-foreground", "Connecting to REAPER\u{2026}" }
                }
            }
        }
    }
}

/// Connection status badge with spinner for connecting state.
#[component]
fn ConnectionBadge(state: ConnectionState) -> Element {
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
