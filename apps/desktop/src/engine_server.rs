//! `session-desktop --engine`: a headless process serving the session
//! setlist over a LAN-reachable `/vox` WebSocket, so a phone/tablet/other
//! computer on the same network can drive the transport — the same
//! `SetlistServiceClient` wire surface the desktop UI and
//! `session_remote_view.rs`'s wasm browser remote both already use.
//!
//! Serves whichever engine is actually running: Live Mode's
//! `session_engine::router()` (the in-process `daw-standalone` setlist
//! directly), or Recording Mode's `reaper_lan_proxy` (which forwards each
//! call through the existing connection `reaper_engine.rs` holds to a
//! real REAPER — session-desktop only ever gets a *client* to that
//! service, so there's no local router to re-mount; see that module's own
//! doc for why it's a hand-written proxy instead).
//!
//! Binds `0.0.0.0`, not `127.0.0.1` — the whole point is reachability
//! from other devices on the network, not just this machine.

use architect::axum_ws;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

const DEFAULT_PORT: u16 = 4040;

#[derive(Clone)]
struct EngineState;

async fn vox_handler(ws: WebSocketUpgrade, State(_state): State<EngineState>) -> Response {
    ws.on_upgrade(move |socket| async move {
        if let Some(engine) = crate::session_engine::engine() {
            axum_ws::serve_router(socket, engine.router()).await;
            return;
        }
        if let Some(proxy) = crate::reaper_lan_proxy::proxy() {
            axum_ws::serve_router(socket, crate::reaper_lan_proxy::router(proxy)).await;
            return;
        }
        tracing::warn!(
            "--engine: /vox connection before either engine (Live Mode or Recording Mode) \
             finished booting"
        );
    })
    .into_response()
}

async fn index_handler() -> Response {
    // `web-dist/` (see the `web-stage` just recipe) isn't staged into this
    // build yet, or `embed-web` wasn't enabled — either way there's no
    // browser UI to serve, but the /vox endpoint above still works with
    // any vox client (a native app, a test harness, a future page).
    axum::response::Html(
        "<!doctype html><title>session-desktop --engine</title>\
         <body style=\"font-family:system-ui;padding:2rem;max-width:40rem\">\
         <h1>session-desktop --engine</h1>\
         <p>Serving the setlist over <code>/vox</code>. No browser UI is \
         staged into this build yet — build one with <code>just web-stage</code> \
         and rebuild with <code>--features embed-web</code> to serve it here.</p>\
         </body>",
    )
    .into_response()
}

/// Serve `/vox` (+ a placeholder `/` until a web build is staged) on
/// `port` (default 4040), on every network interface.
///
/// Caller's job to boot the session engine first (`session_engine::
/// bootstrap_blocking()`) — it manages its own dedicated runtime
/// internally and must not be called from inside this one.
///
/// Never returns under normal operation — the HTTP server runs until the
/// process is killed, same as any other long-running daemon.
pub async fn run(port: Option<u16>) -> eyre::Result<()> {
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/vox", get(vox_handler))
        .with_state(EngineState);

    let port = port.unwrap_or(DEFAULT_PORT);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| eyre::eyre!("binding 0.0.0.0:{port}: {e}"))?;

    println!("session-desktop --engine: listening on:");
    for addr in lan_addresses(port) {
        println!("  http://{addr}  (open this on another device on the same network)");
    }
    println!("  ws://0.0.0.0:{port}/vox");

    axum::serve(listener, app)
        .await
        .map_err(|e| eyre::eyre!("engine server: {e}"))?;
    Ok(())
}

/// Every non-loopback IPv4 address this machine has, each paired with
/// `port` — usually just one (the LAN interface), occasionally more (VPN,
/// a second NIC). Best-effort: falls back to `localhost` alone if enumerating
/// interfaces fails, so the server still prints *something* useful.
fn lan_addresses(port: u16) -> Vec<String> {
    let mut addrs: Vec<String> = Vec::new();
    if let Ok(interfaces) = local_ip_interfaces() {
        for ip in interfaces {
            addrs.push(format!("{ip}:{port}"));
        }
    }
    if addrs.is_empty() {
        addrs.push(format!("localhost:{port}"));
    }
    addrs
}

/// Non-loopback IPv4 addresses via `getifaddrs(3)` — no extra crate needed
/// for something this small. Unix-only, matching the rest of Recording
/// Mode's own Unix-only socket-discovery code in this app.
#[cfg(unix)]
fn local_ip_interfaces() -> std::io::Result<Vec<std::net::Ipv4Addr>> {
    use std::net::Ipv4Addr;

    let mut result = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `ifap` is a valid out-pointer for getifaddrs; freeifaddrs is
    // called on every path once it succeeds, matching the man page's
    // ownership contract.
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut cursor = ifap;
    while !cursor.is_null() {
        // SAFETY: `cursor` is non-null and was populated by getifaddrs;
        // `ifa_addr` may legitimately be null for some interface types.
        let ifa = unsafe { &*cursor };
        if !ifa.ifa_addr.is_null() {
            // SAFETY: sockaddr fields are readable for the lifetime of
            // this loop iteration; only sockaddr_in (AF_INET) is
            // interpreted, matching its actual size.
            let family = unsafe { (*ifa.ifa_addr).sa_family };
            if i32::from(family) == libc::AF_INET {
                let sockaddr_in = ifa.ifa_addr.cast::<libc::sockaddr_in>();
                let ip = unsafe { (*sockaddr_in).sin_addr.s_addr };
                let ip = Ipv4Addr::from(u32::from_be(ip));
                if !ip.is_loopback() {
                    result.push(ip);
                }
            }
        }
        cursor = ifa.ifa_next;
    }
    // SAFETY: `ifap` was successfully populated by the getifaddrs call
    // above and hasn't been freed yet.
    unsafe { libc::freeifaddrs(ifap) };
    Ok(result)
}

#[cfg(not(unix))]
fn local_ip_interfaces() -> std::io::Result<Vec<std::net::Ipv4Addr>> {
    Ok(Vec::new())
}
