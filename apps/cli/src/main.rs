//! `session` — drive a real REAPER's setlist from the command line.
//!
//! The same surface `session-desktop`'s Recording Mode drives, without the
//! GUI: it dials a running REAPER extension's DAW socket
//! (`/tmp/fts-daw-{pid}.sock`), opens the `SetlistService` lanes, and calls
//! the same RPCs the transport bar does.
//!
//! # Why a CLI at all
//!
//! Opening an album is a batch operation — ten projects named by an `.RPL`
//! — and a batch operation wants a command, not a window. It is also the
//! only way to drive this over SSH, which is how these sessions actually
//! get set up on the studio Mac.
//!
//! ```text
//! session open "CRESCENDUM ALBUM LIST.RPL"   # launch/attach REAPER, open every project
//! session setlist                            # what is loaded, by song and section
//! session seek 3                             # jump to song 3, section 0
//! session seek 3 2                           # song 3, section 2
//! session status                             # which REAPER, and where the cursor is
//! ```

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use session::SetlistServiceClient;

/// The dev-rig REAPER profile, matching `session-desktop`'s Recording Mode.
/// Only used when nothing is running and we have to launch one.
const REAPER_PROFILE: &str = "fts-dev";

const SOCKET_DIR: &str = "/tmp";
const SOCKET_PREFIX: &str = "fts-daw-";
const SOCKET_SUFFIX: &str = ".sock";

#[derive(Parser)]
#[command(
    name = "session",
    about = "Drive a real REAPER's setlist from the command line"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open every project an `.RPL` names, then build a setlist from them.
    ///
    /// Launches REAPER (the `fts-dev` profile) if none is running.
    Open {
        /// The `.RPL` project list, or one or more `.RPP` files directly.
        paths: Vec<PathBuf>,
    },
    /// Print the loaded setlist: every song, and every section under it.
    Setlist,
    /// Move the cursor to a song, and optionally a section within it.
    Seek {
        /// Song index, 0-based — as printed by `session setlist`.
        song: usize,
        /// Section index within that song, 0-based. Defaults to the first.
        section: Option<usize>,
    },
    /// Which REAPER this would talk to, and where its cursor is.
    Status,
}

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(run(cli.command))
}

async fn run(command: Command) -> eyre::Result<()> {
    match command {
        Command::Open { paths } => open(&paths).await,
        Command::Setlist => setlist().await,
        Command::Seek { song, section } => seek(song, section.unwrap_or(0)).await,
        Command::Status => status().await,
    }
}

// ── Discovery and connection ────────────────────────────────────────────
//
// Deliberately the same shape as `session-desktop`'s `reaper_engine`: a
// socket per live REAPER, named by pid, newest first. Kept here rather than
// shared because the desktop's version owns a long-lived connection and a
// reconnect supervisor; a CLI process dials once and exits.

/// Every live REAPER publishing a DAW socket, newest first. A socket whose
/// process is gone is skipped, not deleted — it is not ours to unlink.
fn discover_sockets() -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(SOCKET_DIR) else {
        return Vec::new();
    };
    let mut found: Vec<(u32, PathBuf)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let pid: u32 = path
                .file_name()?
                .to_str()?
                .strip_prefix(SOCKET_PREFIX)?
                .strip_suffix(SOCKET_SUFFIX)?
                .parse()
                .ok()?;
            process_alive(pid).then_some((pid, path))
        })
        .collect();
    found.sort_by_key(|(pid, _)| std::cmp::Reverse(*pid));
    found
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // Signal 0 is the standard existence probe (`kill(2)`): it never
    // actually signals the process.
    unsafe { libc::kill(i32::try_from(pid).unwrap_or(-1), 0) == 0 }
}

/// Open the `SetlistService` lanes on a REAPER's socket.
async fn connect_setlist(socket: &Path) -> eyre::Result<SetlistServiceClient> {
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| eyre::eyre!("connecting to {}: {e}", socket.display()))?;
    let link = vox_stream::StreamLink::unix(stream);
    let connection = vox::initiator_on(link)
        .establish_connection()
        .await
        .map_err(|e| eyre::eyre!("vox handshake with {}: {e:?}", socket.display()))?;
    let client = connection
        .open_lane::<SetlistServiceClient>()
        .await
        .map_err(|e| {
            eyre::eyre!(
                "opening the SetlistService lane on {}: {e:?}\n\
                 (is this REAPER running the FTS extension?)",
                socket.display()
            )
        })?;
    // The connection owns the link; leak it so the client stays usable for
    // the rest of this short-lived process rather than tearing down mid-call.
    std::mem::forget(connection);
    Ok(client)
}

/// The newest live REAPER's `SetlistService`, or a clear reason there isn't one.
async fn connect() -> eyre::Result<SetlistServiceClient> {
    let (_, socket) = discover_sockets().into_iter().next().ok_or_else(|| {
        eyre::eyre!(
            "no running REAPER found (no {SOCKET_DIR}/{SOCKET_PREFIX}*{SOCKET_SUFFIX}).\n\
             Start REAPER with the FTS extension loaded, or run `session open <list.RPL>`\n\
             to launch one."
        )
    })?;
    connect_setlist(&socket).await
}

// ── Commands ────────────────────────────────────────────────────────────

/// Resolve the projects to open: an `.RPL` expands to the list it names,
/// anything else is taken as a project path directly.
fn resolve_projects(paths: &[PathBuf]) -> eyre::Result<Vec<PathBuf>> {
    if paths.is_empty() {
        eyre::bail!("nothing to open — pass an .RPL project list, or .RPP paths");
    }
    let mut out = Vec::new();
    for path in paths {
        let is_rpl = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rpl"));
        if is_rpl {
            let listed = dawfile_reaper::setlist_rpp::parse_rpl(path)
                .map_err(|e| eyre::eyre!("reading {}: {e}", path.display()))?;
            out.extend(listed);
        } else {
            out.push(path.clone());
        }
    }
    Ok(out)
}

async fn open(paths: &[PathBuf]) -> eyre::Result<()> {
    let projects = resolve_projects(paths)?;
    println!("{} project(s) to open", projects.len());

    // Every path is checked before REAPER is touched. Opening half an album
    // and then failing leaves a worse state than not starting.
    let missing: Vec<&PathBuf> = projects.iter().filter(|p| !p.exists()).collect();
    if !missing.is_empty() {
        for path in &missing {
            eprintln!("  missing: {}", path.display());
        }
        eyre::bail!(
            "{} of {} projects do not exist — nothing was opened",
            missing.len(),
            projects.len()
        );
    }

    let daw = match discover_sockets().into_iter().next() {
        Some((pid, socket)) => {
            println!("attaching to REAPER (pid {pid})");
            daw::cli::connect(Some(socket)).await?.daw
        }
        None => {
            println!("no REAPER running — launching the '{REAPER_PROFILE}' profile");
            let (connection, pid, socket) = daw::cli::launch_and_connect(REAPER_PROFILE).await?;
            println!("  launched pid {pid} ({})", socket.display());
            connection.daw
        }
    };

    for path in &projects {
        let name = dawfile_reaper::setlist_rpp::song_name_from_path(path);
        daw.open_project(path.to_string_lossy().into_owned())
            .await
            .map_err(|e| eyre::eyre!("opening '{name}' ({}): {e:?}", path.display()))?;
        println!("  opened {name}");
    }

    // The projects are open as REAPER tabs; this is what turns them into a
    // setlist the transport can page through.
    let client = connect().await?;
    client
        .build_from_open_projects()
        .await
        .map_err(|e| eyre::eyre!("build_from_open_projects: {e:?}"))?;
    println!();
    print_setlist(&client).await
}

async fn setlist() -> eyre::Result<()> {
    let client = connect().await?;
    print_setlist(&client).await
}

async fn print_setlist(client: &SetlistServiceClient) -> eyre::Result<()> {
    let setlist = client
        .setlist()
        .await
        .map_err(|e| eyre::eyre!("setlist: {e:?}"))?;
    if setlist.songs.is_empty() {
        println!("setlist is empty — `session open <list.RPL>` to build one");
        return Ok(());
    }
    println!("{} song(s):", setlist.songs.len());
    for (i, song) in setlist.songs.iter().enumerate() {
        println!("  {i:>2}  {}", song.name);
        for (j, section) in song.sections.iter().enumerate() {
            println!("        {j:>2}  {}", section.name);
        }
    }
    Ok(())
}

async fn seek(song: usize, section: usize) -> eyre::Result<()> {
    let client = connect().await?;
    let setlist = client
        .setlist()
        .await
        .map_err(|e| eyre::eyre!("setlist: {e:?}"))?;
    // Report the name being jumped to, not just the indices — an off-by-one
    // is otherwise invisible until the wrong song starts playing.
    let name = setlist
        .songs
        .get(song)
        .map_or_else(|| "<out of range>".to_string(), |s| s.name.clone());
    client
        .seek_to_section(song, section)
        .await
        .map_err(|e| eyre::eyre!("seek_to_section({song}, {section}): {e:?}"))?;

    println!("song {song} / section {section} — {name}");

    // Read the cursor back rather than trusting the call. A `seek_to_section`
    // that returns Ok has been accepted, not necessarily applied — it bounces
    // to REAPER's main thread and the cursor update is a second, independent
    // hop. Reporting success from the return value alone would hide exactly
    // the failure this command exists to catch. (`active_song` is on the RPC
    // client; per-section confirmation needs the subscribe stream, which a
    // one-shot CLI process has no good place to pump.)
    match client.active_song().await {
        Ok(active) if active.name == name => println!("  REAPER is on this song"),
        Ok(active) => println!(
            "  WARNING: REAPER is on '{}', not '{name}' — the seek did not land",
            active.name
        ),
        Err(e) => println!("  (could not read REAPER's position back: {e:?})"),
    }
    Ok(())
}

async fn status() -> eyre::Result<()> {
    let sockets = discover_sockets();
    if sockets.is_empty() {
        println!("no running REAPER");
        return Ok(());
    }
    for (pid, socket) in &sockets {
        println!("REAPER pid {pid}  {}", socket.display());
    }
    let client = connect().await?;
    // A REAPER that is up but has no setlist yet is the normal state right
    // after launch, not an error — `setlist()` reports it as NotFound.
    let Ok(setlist) = client.setlist().await else {
        println!("connected, but no setlist built yet — `session open <list.RPL>`");
        return Ok(());
    };
    println!("{} song(s) in the setlist", setlist.songs.len());
    match client.active_song().await {
        Ok(active) => println!("cursor: {}", active.name),
        Err(_) => println!("cursor: nowhere yet — `session seek <song>`"),
    }
    Ok(())
}
