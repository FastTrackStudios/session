//! WebSocket connection to the session desktop gateway.
//!
//! Connects to the desktop app's gateway-ws server via vox over WebSocket.
//! The desktop pushes setlist/transport events; the browser makes RPC calls
//! for navigation and control.
//!
//! ## Connection flow
//!
//! 1. Derive WebSocket URL from the page hostname (same host, port 3030)
//! 2. Connect via `vox-websocket::WsLink`
//! 3. Establish vox session with `WebClientServiceDispatcher` handler
//! 4. Initialize `Session` singleton with `SetlistServiceClient`
//! 5. Fetch initial setlist, then wait for push events

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;
use session::{SetlistServiceClient, WebClientServiceDispatcher};
use vox_websocket::WsLink;

use crate::web_client_handler::WebClientHandler;
use session_ui::{
    ConnectionState, Session, ACTIVE_INDICES, PLAYBACK_STATE, SETLIST_STRUCTURE, SONG_CHARTS,
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

/// Derive WebSocket URL from the current page hostname.
/// Uses port 3030 (gateway default) unless overridden via `?ws=` query param.
fn get_ws_url() -> String {
    web_sys::window()
        .and_then(|w| {
            let location = w.location();

            // Check for explicit ?ws= override
            if let Ok(search) = location.search() {
                for param in search.trim_start_matches('?').split('&') {
                    if let Some(url) = param.strip_prefix("ws=") {
                        return Some(url.to_string());
                    }
                }
            }

            // Default: same hostname, port 3030
            if let Ok(hostname) = location.hostname() {
                return Some(format!("ws://{}:3030/ws", hostname));
            }

            None
        })
        .unwrap_or_else(|| "ws://localhost:3030/ws".to_string())
}

/// Start the connection loop with automatic reconnection.
fn start_connection(mut state_signal: Signal<ConnectionState>) {
    let ws_url = get_ws_url();

    wasm_bindgen_futures::spawn_local(async move {
        let mut attempt = 0u32;
        let initial_backoff = Duration::from_millis(500);
        let max_backoff = Duration::from_secs(10);

        loop {
            state_signal.set(ConnectionState::Connecting);
            log(&format!(
                "[session-web] Connecting to {} (attempt {})...",
                ws_url,
                attempt + 1
            ));

            match try_connect_and_run(&ws_url, &mut state_signal).await {
                Ok(()) => {
                    log("[session-web] Connection closed, reconnecting...");
                    attempt = 0;
                }
                Err(e) => {
                    log(&format!("[session-web] Connection failed: {}", e));
                }
            }

            state_signal.set(ConnectionState::Disconnected);

            let backoff = initial_backoff.mul_f64(1.5f64.powi(attempt as i32));
            let backoff = backoff.min(max_backoff);
            log(&format!("[session-web] Reconnecting in {:?}...", backoff));

            gloo_timers::future::TimeoutFuture::new(backoff.as_millis() as u32).await;
            attempt = attempt.saturating_add(1);
        }
    });
}

/// Attempt to connect and run until the connection drops.
async fn try_connect_and_run(
    ws_url: &str,
    state_signal: &mut Signal<ConnectionState>,
) -> Result<(), String> {
    let link = WsLink::connect(ws_url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {e}"))?;

    log("[session-web] WebSocket connected, initiating vox handshake...");

    let handler = WebClientServiceDispatcher::new(WebClientHandler);
    let handshake_result = vox::HandshakeResult {
        role: vox::SessionRole::Initiator,
        our_settings: vox::ConnectionSettings {
            parity: vox::Parity::Odd,
            max_concurrent_requests: 64,
        },
        peer_settings: vox::ConnectionSettings {
            parity: vox::Parity::Even,
            max_concurrent_requests: 64,
        },
        peer_supports_retry: true,
        session_resume_key: None,
        peer_resume_key: None,
        our_schema: vec![],
        peer_schema: vec![],
    };
    let (caller, _session_handle) =
        vox::initiator_conduit(vox::BareConduit::new(link), handshake_result)
            .establish::<vox::DriverCaller>(handler)
            .await
            .map_err(|e| format!("Handshake failed: {e:?}"))?;

    log("[session-web] Connection established!");
    state_signal.set(ConnectionState::Connected);

    // Initialize Session singleton with SetlistServiceClient
    let handle = vox::ErasedCaller::new(caller);
    let setlist_client = SetlistServiceClient::new(handle);
    let _ = Session::init(setlist_client);

    log("[session-web] Session initialized");

    // Fetch initial setlist
    if let Err(e) = fetch_setlist().await {
        log(&format!("[session-web] Failed to fetch setlist: {e}"));
    }

    // Keep alive — poll connection health
    let connection_lost = Rc::new(Cell::new(false));
    let lost_clone = connection_lost.clone();

    let session = Session::get();
    let client = session.setlist().clone();

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(5000).await;
            if client.get_audio_latency_info().await.is_err() {
                log("[session-web] Connection health check failed");
                lost_clone.set(true);
                break;
            }
        }
    });

    loop {
        gloo_timers::future::TimeoutFuture::new(100).await;
        if connection_lost.get() {
            break;
        }
    }

    Ok(())
}

/// Fetch the current setlist from the server and populate UI signals.
async fn fetch_setlist() -> Result<(), String> {
    log("[session-web] Fetching setlist...");

    let session = Session::get();
    let client = session.setlist();

    client
        .build_from_open_projects()
        .await
        .map_err(|e| format!("build_from_open_projects: {e:?}"))?;

    let setlist = client
        .get_setlist()
        .await
        .map_err(|e| format!("get_setlist: {e:?}"))?;

    log(&format!(
        "[session-web] Setlist '{}' with {} songs",
        setlist.name,
        setlist.songs.len()
    ));

    let songs = setlist.songs.clone();
    *SETLIST_STRUCTURE.write() = setlist;

    // Set initial active song
    match client.get_active_song().await {
        Ok(active) => {
            if let Some(idx) = songs
                .iter()
                .position(|s| s.project_guid == active.project_guid)
            {
                let mut indices = ACTIVE_INDICES.write();
                indices.song_index = Some(idx);
                indices.section_index = Some(0);
            }
        }
        Err(_) => {
            if !songs.is_empty() {
                let mut indices = ACTIVE_INDICES.write();
                indices.song_index = Some(0);
                indices.section_index = Some(0);
            }
        }
    }

    Ok(())
}

/// Dioxus hook: returns (connection_state, connect_callback).
pub fn use_connection() -> (Signal<ConnectionState>, Callback<()>) {
    let connection_state = use_signal(|| ConnectionState::Disconnected);

    let connect = use_callback(move |_: ()| {
        start_connection(connection_state);
    });

    (connection_state, connect)
}
