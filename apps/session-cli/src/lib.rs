//! session-cli library — reusable components for Session CLI tools.
//!
//! Provides command implementations for live session control: setlist
//! navigation, song/section browsing, playback control, and loop management.
//!
//! The session CLI connects to the session cell via RPC (roam) to control
//! setlist navigation and playback. The session cell must be running
//! (it runs inside the FTS REAPER extension).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::Result;

// ============================================================================
// CLI Definitions
// ============================================================================

#[derive(Subcommand)]
pub enum SessionCommand {
    /// Show current setlist
    Setlist,
    /// List songs in the setlist
    Songs,
    /// Show song detail
    Song {
        /// Song index (0-based)
        index: usize,
    },
    /// List sections in a song
    Sections {
        /// Song index (0-based)
        song_index: usize,
    },
    /// Navigation commands
    #[command(subcommand)]
    Goto(GotoCommand),
    /// Go to next song
    Next,
    /// Go to previous song
    Previous,
    /// Go to next section
    NextSection,
    /// Go to previous section
    PrevSection,
    /// Start playback
    Play,
    /// Pause playback
    Pause,
    /// Stop playback
    Stop,
    /// Loop control
    #[command(subcommand)]
    Loop(LoopCommand),
    /// Seek to position (seconds, relative to current song)
    Seek {
        /// Position in seconds
        seconds: f64,
    },
}

#[derive(Subcommand)]
pub enum GotoCommand {
    /// Navigate to a specific song
    Song {
        /// Song index (0-based)
        index: usize,
    },
    /// Navigate to a specific section in the current song
    Section {
        /// Section index (0-based)
        index: usize,
    },
}

#[derive(Subcommand)]
pub enum LoopCommand {
    /// Loop the current song
    Song,
    /// Loop the current section
    Section,
    /// Clear loop
    Clear,
}

// ============================================================================
// Dispatch
// ============================================================================

pub async fn run(
    _socket: Option<PathBuf>,
    cmd: SessionCommand,
    _as_json: bool,
) -> Result<()> {
    // Phase 3: Connect to the session cell via RPC and dispatch commands.
    // The session service runs inside the FTS REAPER extension as a cell,
    // so the CLI connects via the same socket as daw-cli.
    let cmd_name = match cmd {
        SessionCommand::Setlist => "setlist",
        SessionCommand::Songs => "songs",
        SessionCommand::Song { .. } => "song",
        SessionCommand::Sections { .. } => "sections",
        SessionCommand::Goto(GotoCommand::Song { .. }) => "goto song",
        SessionCommand::Goto(GotoCommand::Section { .. }) => "goto section",
        SessionCommand::Next => "next",
        SessionCommand::Previous => "previous",
        SessionCommand::NextSection => "next-section",
        SessionCommand::PrevSection => "prev-section",
        SessionCommand::Play => "play",
        SessionCommand::Pause => "pause",
        SessionCommand::Stop => "stop",
        SessionCommand::Loop(LoopCommand::Song) => "loop song",
        SessionCommand::Loop(LoopCommand::Section) => "loop section",
        SessionCommand::Loop(LoopCommand::Clear) => "loop clear",
        SessionCommand::Seek { .. } => "seek",
    };
    eyre::bail!(
        "Session command '{}' not yet implemented. \
         Session CLI requires Phase 3 implementation \
         (RPC connection to the session cell running in REAPER).",
        cmd_name
    )
}
