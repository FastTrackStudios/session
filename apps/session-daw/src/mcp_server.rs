//! The running window's Action List, served to a model over MCP.
//!
//! Whatever this window can do is an action in its Action List
//! (`crate::actions`); this puts a model on it while you work — on
//! `http://127.0.0.1:47811/mcp` (`FTS_MCP_PORT` to move it, `FTS_MCP=0`
//! for none), behind a token. The address and the token are written to
//! `~/.config/fts/session-mcp.json` for a client to read — Claude Code:
//!
//! ```sh
//! claude mcp add --transport http session http://127.0.0.1:47811/mcp \
//!     --header "Authorization: Bearer $(jq -r .token ~/.config/fts/session-mcp.json)"
//! ```
//!
//! Only this machine reaches it: it listens on loopback, wants the token,
//! and refuses a browser's request from anywhere else
//! (`architect_mcp::http`).

use std::path::PathBuf;

/// Where it listens unless `FTS_MCP_PORT` says otherwise.
pub const DEFAULT_PORT: u16 = 47_811;

/// What a model is told about this app when it connects.
const INSTRUCTIONS: &str = "This is FastTrackStudio Session, the DAW the user mixes in, with the \
song they have open. Its tracks are read with SESSION_TRACKS: each has a GUID and a folder path \
(`Drums/Drum Kit/Kick/Sum/In`); actions that take a track take any of its GUID, its path or a tail \
of the path (`Kick/Sum/Trig`). The kit follows the golden template: each piece (Kick, Snare, \
Tom 1…) has its close mics, its `Trig` (what says the drum was hit — a recorded trigger or the \
kit's notes; it keys the piece's gate and is never heard) and its `Sample` (what plays from the \
trigger), with a `Fund` (or the kick's `Sub`) band-pass beside them. DRUMS_HITS lists a drum's \
hits — run it on the piece's Trig — with times, bars and beats and levels, paged. SESSION_* \
actions are the window's own: what its keys do. TRACK_SET changes a track's fader, pan, mute or \
solo.";

/// Where it is being served, and the token to reach it.
#[derive(Clone, Debug)]
pub struct Serving {
    pub url: String,
    pub token: String,
}

/// Serve the Action List on the engine's runtime, unless `FTS_MCP=0`;
/// `None` when it is off or the port is taken (said in the log).
pub fn start() -> Option<Serving> {
    if std::env::var("FTS_MCP").is_ok_and(|v| v == "0") {
        return None;
    }
    let runtime = crate::open::runtime()?;
    let port = std::env::var("FTS_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(error) => {
            tracing::warn!(mcp.port = port, error = %error, "mcp: not served, the port is taken");
            return None;
        }
    };
    if listener.set_nonblocking(true).is_err() {
        return None;
    }
    let token = std::env::var("FTS_MCP_TOKEN")
        .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string());
    let url = format!("http://127.0.0.1:{port}/mcp");
    let server = architect_mcp::Server::new(
        "FastTrackStudio Session",
        env!("CARGO_PKG_VERSION"),
        crate::actions::list().clone(),
    )
    .with_instructions(INSTRUCTIONS);
    let guarded = Some(token.clone());
    runtime.spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(error) => {
                tracing::warn!(error = %error, "mcp: the listener would not start");
                return;
            }
        };
        if let Err(error) = architect_mcp::http::serve(listener, server, guarded).await {
            tracing::warn!(error = %error, "mcp: stopped");
        }
    });
    let serving = Serving { url, token };
    write_config(&serving);
    tracing::info!(
        mcp.url = %serving.url,
        mcp.actions = crate::actions::list().len(),
        "mcp: serving the Action List"
    );
    Some(serving)
}

/// `~/.config/fts/session-mcp.json`: where a client finds it.
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("fts").join("session-mcp.json"))
}

/// Say where it is, readable by this user only — the token is in it.
fn write_config(serving: &Serving) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_err()
    {
        return;
    }
    let text = serde_json::json!({ "url": serving.url, "token": serving.token }).to_string();
    if std::fs::write(&path, text).is_err() {
        tracing::warn!("mcp: could not write where it is served");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}
