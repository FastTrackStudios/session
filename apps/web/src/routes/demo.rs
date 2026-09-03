//! `/demo` — the real thing, running in the browser.
//!
//! Boots the in-process backend (see [`crate::demo_backend`]), installs
//! it as session-ui's `Session` singleton, pumps its `#[subscribe]`
//! streams into session-ui's global signals (mirroring
//! `apps/desktop/src/session_view.rs::SessionEventBridge`), then renders
//! the real desktop performance view: sidebar, `PerformanceLayout`,
//! transport bar.

use dioxus::prelude::*;
use session_ui::{PerformanceLayout, PerformanceSidebar, TransportPanel};

use crate::demo_backend;

#[component]
pub fn Demo() -> Element {
    let mut booted = use_signal(|| false);
    let mut boot_error = use_signal(|| None::<String>);

    use_future(move || async move {
        let handle = match demo_backend::boot().await {
            Ok(h) => h,
            Err(e) => {
                boot_error.set(Some(format!("{e:?}")));
                return;
            }
        };

        // `Session::init` only accepts one client per process — fine here
        // since `demo_backend::boot()` itself is idempotent and always
        // hands back the same handle.
        if session_ui::Session::init(handle.client.clone()).is_err() {
            tracing::debug!("session-ui Session already initialized");
        }

        // Events stream: setlist structure + per-song transport.
        let events_stream_client = handle.stream_client.clone();
        let events_client = handle.client.clone();
        spawn(async move {
            let (tx, mut rx) = vox::channel::<session::SetlistEvent>();
            spawn(async move {
                if let Err(e) = events_stream_client.events(tx).await {
                    tracing::warn!("events subscription ended: {e:?}");
                }
            });
            match events_client.setlist().await {
                Ok(setlist) => {
                    session_ui::apply_setlist_event(&session::SetlistEvent::SetlistChanged(
                        setlist,
                    ));
                }
                Err(e) => tracing::warn!("initial setlist snapshot failed: {e:?}"),
            }
            while let Ok(Some(ev)) = rx.recv().await {
                session_ui::apply_setlist_event(ev.get());
            }
        });

        // Active-indices stream: which song/section is current.
        let indices_stream_client = handle.stream_client.clone();
        let seek_client = handle.client.clone();
        spawn(async move {
            let (tx, mut rx) = vox::channel::<session::ActiveIndices>();
            spawn(async move {
                if let Err(e) = indices_stream_client.active_indices(tx).await {
                    tracing::warn!("active_indices subscription ended: {e:?}");
                }
            });
            // Open on song 0 / section 0, fired concurrently so this future
            // is already polling `rx` when the seek's cursor publish lands.
            spawn(async move {
                if let Err(e) = seek_client.seek_to_section(0, 0).await {
                    tracing::warn!("initial seek to song 0 failed: {e:?}");
                }
            });
            while let Ok(Some(ai)) = rx.recv().await {
                session_ui::apply_active_indices(ai.get());
            }
        });

        booted.set(true);
    });

    if let Some(err) = boot_error.read().as_ref() {
        return rsx! {
            div { class: "h-screen w-screen flex flex-col items-center justify-center gap-2 bg-zinc-950 text-zinc-100 text-center px-8",
                span { class: "text-xl font-bold", "Demo failed to start" }
                span { class: "text-sm text-zinc-400 max-w-lg", "{err}" }
            }
        };
    }

    if !booted() || session_ui::SETLIST_STRUCTURE.read().songs.is_empty() {
        return rsx! {
            div { class: "h-screen w-screen flex items-center justify-center bg-zinc-950 text-zinc-100",
                span { class: "text-lg font-semibold", "Loading the demo setlist\u{2026}" }
            }
        };
    }

    rsx! {
        div { class: "h-screen w-screen flex flex-row bg-zinc-950 text-zinc-100",
            div { class: "w-[280px] flex-none min-h-0 border-r border-zinc-800 flex",
                PerformanceSidebar {}
            }
            div { class: "flex-1 min-w-0 min-h-0 flex flex-col",
                div { class: "relative flex-1 min-h-0 flex", PerformanceLayout {} }
                div { class: "h-[92px] flex-none border-t border-zinc-800",
                    TransportPanel { show_recording: false }
                }
            }
        }
    }
}
