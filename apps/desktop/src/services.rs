//! Local Session Services
//!
//! Discovers a running REAPER instance via its Unix socket, connects via vox,
//! initializes the DAW singleton, and builds the setlist from open projects.

use std::path::{Path, PathBuf};

use daw::sync::LocalCaller;
use daw::{Daw, ErasedCaller};
use eyre::{bail, Result};
use session::{
    SetlistServiceClient, SetlistServiceDispatcher, SetlistServiceImpl, SongServiceImpl,
};
use session_ui::Session;

const SOCKET_DIR: &str = "/tmp";
const SOCKET_PREFIX: &str = "fts-daw-";
const SOCKET_SUFFIX: &str = ".sock";

/// Initialize session services by connecting to a running REAPER instance.
///
/// 1. Discovers `/tmp/fts-daw-{pid}.sock` sockets
/// 2. Connects via vox RPC
/// 3. Initializes the global `Daw` singleton
/// 4. Creates in-process SetlistService and builds from open projects
/// 5. Initializes `Session::init()` so UI components can access the setlist client
pub async fn init_session_services() -> Result<()> {
    // Connect to REAPER
    let caller = discover_and_connect().await?;
    Daw::init(caller)?;
    tracing::info!("DAW initialized");

    // Create setlist service and build from REAPER's open projects
    let setlist = SetlistServiceImpl::new();
    let _song = SongServiceImpl::new();

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
