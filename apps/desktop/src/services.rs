//! Local Session Services
//!
//! Discovers a running REAPER instance via its Unix socket, connects via vox,
//! initializes the DAW singleton, and builds the setlist from open projects.

use std::path::{Path, PathBuf};

use daw::sync::LocalCaller;
use daw::{Daw, ErasedCaller};
use eyre::{bail, Result};
use session::{
    SetlistServiceClient, SetlistServiceDispatcher, SetlistServiceImpl,
    SongServiceDispatcher, SongServiceImpl,
    setlist_service_service_descriptor, song_service_service_descriptor,
};
use session_ui::Session;

use crate::gateway;

const SOCKET_DIR: &str = "/tmp";
const SOCKET_PREFIX: &str = "fts-daw-";
const SOCKET_SUFFIX: &str = ".sock";

/// Start the WebSocket gateway immediately (does not require REAPER).
///
/// Returns the gateway binding info once the server is listening.
/// The gateway serves the web app and accepts WebSocket RPC connections
/// regardless of whether REAPER is connected.
pub async fn start_gateway() -> Result<gateway::GatewayInfo> {
    let setlist = SetlistServiceImpl::new();
    let song = SongServiceImpl::new();

    let handler = gateway::RoutedHandler::new()
        .with(
            &setlist_service_service_descriptor(),
            SetlistServiceDispatcher::new(setlist),
        )
        .with(
            &song_service_service_descriptor(),
            SongServiceDispatcher::new(song),
        );

    let bind_addr = std::env::var("GATEWAY_WS_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3030".to_string());
    let static_dir = std::env::var("GATEWAY_WS_STATIC_DIR")
        .ok()
        .or_else(discover_web_static_dir);
    if let Some(ref dir) = static_dir {
        tracing::info!("Serving web app from: {dir}");
    }

    let (info_tx, info_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Err(e) = gateway::start_gateway(
            handler,
            &bind_addr,
            static_dir.as_deref(),
            info_tx,
        )
        .await
        {
            tracing::error!("WebSocket gateway error: {e}");
        }
    });

    let gw_info = info_rx
        .await
        .map_err(|_| eyre::eyre!("Gateway failed to start"))?;
    Ok(gw_info)
}

/// Connect to REAPER and initialize session services.
///
/// This can be called repeatedly (retried) until REAPER is available.
/// Once connected, initializes the DAW singleton and Session client.
pub async fn connect_to_reaper() -> Result<()> {
    let caller = discover_and_connect().await?;
    Daw::init(caller)?;
    tracing::info!("DAW initialized");

    let setlist = SetlistServiceImpl::new();

    let local = LocalCaller::new(SetlistServiceDispatcher::new(setlist)).await?;
    let client = SetlistServiceClient::new(local.erased_caller());

    // Build setlist from whatever's open in REAPER
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("{e:?}"))?;
    tracing::info!("Setlist built from open projects");

    Session::init(client)?;
    tracing::info!("Session services initialized");
    Ok(())
}

/// Scan `/tmp` for `fts-daw-*.sock` sockets, connect to the first live one.
async fn discover_and_connect() -> Result<ErasedCaller> {
    let sockets = discover_sockets();
    if sockets.is_empty() {
        bail!(
            "No REAPER sockets found in {SOCKET_DIR} — is REAPER running with the FTS extension?"
        );
    }

    tracing::info!("Found {} REAPER socket(s)", sockets.len());

    for (pid, path) in &sockets {
        tracing::debug!("Trying REAPER socket: {} (pid {pid})", path.display());
        match connect_to_daw(path).await {
            Ok(caller) => {
                tracing::info!("Connected to REAPER (pid {pid}) via {}", path.display());
                return Ok(caller);
            }
            Err(e) => {
                tracing::warn!("Failed to connect to {}: {e}", path.display());
            }
        }
    }

    bail!("Could not connect to any REAPER instance");
}

/// Scan `/tmp` for `fts-daw-*.sock` sockets and return `(pid, path)` pairs.
fn discover_sockets() -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(SOCKET_DIR) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let pid = pid_from_socket_path(&path)?;
            // Only include sockets for live processes
            if !is_process_alive(pid) {
                return None;
            }
            Some((pid, path))
        })
        .collect()
}

/// Extract the PID from a socket filename like `fts-daw-12345.sock`.
fn pid_from_socket_path(path: &Path) -> Option<u32> {
    let filename = path.file_name()?.to_str()?;
    let rest = filename.strip_prefix(SOCKET_PREFIX)?;
    let pid_str = rest.strip_suffix(SOCKET_SUFFIX)?;
    pid_str.parse().ok()
}

/// Check if a process is still alive by running `kill -0 {pid}`.
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Connect to a REAPER socket and establish a vox session.
///
/// Opens a virtual connection on the session (matching what daw-bridge expects)
/// so that the RoutedHandler can properly dispatch service calls.
async fn connect_to_daw(path: &Path) -> eyre::Result<ErasedCaller> {
    let stream = tokio::net::UnixStream::connect(path).await?;
    let link = vox_stream::StreamLink::unix(stream);
    let handshake_result = initiator_handshake_result(64);
    let (_root_caller, session) =
        vox::initiator_conduit(vox::BareConduit::new(link), handshake_result)
            .establish::<vox::DriverCaller>(())
            .await?;

    // Open a virtual connection for DAW services — the daw-bridge's RoutedHandler
    // dispatches service calls on virtual connections, not the root session.
    let conn = session
        .open_connection(
            vox::ConnectionSettings {
                parity: vox::Parity::Odd,
                max_concurrent_requests: 64,
            },
            vec![vox::MetadataEntry {
                key: "role",
                value: vox::MetadataValue::String("session-desktop"),
                flags: vox::MetadataFlags::NONE,
            }],
        )
        .await?;

    let mut driver = vox::Driver::new(conn, ());
    let caller = ErasedCaller::new(driver.caller());
    moire::task::spawn(async move { driver.run().await });

    Ok(caller)
}

/// Construct a synthetic `HandshakeResult` for initiator connections.
fn initiator_handshake_result(max_concurrent_requests: u32) -> vox::HandshakeResult {
    vox::HandshakeResult {
        role: vox::SessionRole::Initiator,
        our_settings: vox::ConnectionSettings {
            parity: vox::Parity::Odd,
            max_concurrent_requests,
        },
        peer_settings: vox::ConnectionSettings {
            parity: vox::Parity::Even,
            max_concurrent_requests,
        },
        peer_supports_retry: true,
        session_resume_key: None,
        peer_resume_key: None,
        our_schema: vec![],
        peer_schema: vec![],
    }
}

/// Try to find the web app's `dx build` output directory.
///
/// Checks (in order):
/// 1. `target/dx/session-web/release/web/public/` (release build)
/// 2. `target/dx/session-web/debug/web/public/` (debug build)
///
/// Paths are resolved relative to the cargo workspace root.
fn discover_web_static_dir() -> Option<String> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?;

    let candidates = [
        workspace_root.join("target/dx/session-web/release/web/public"),
        workspace_root.join("target/dx/session-web/debug/web/public"),
    ];

    for candidate in &candidates {
        if candidate.join("index.html").exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }

    None
}
