//! session-cli library — reusable components for Session CLI tools.
//!
//! Provides:
//! - **Offline**: `combine` — merge multiple song RPP files into a single
//!   setlist project, trimmed to marker bounds (PREROLL → POSTROLL / =START → =END).
//! - **Live**: setlist navigation, playback control (requires running REAPER session).

use std::path::PathBuf;

use clap::Subcommand;
use eyre::Result;

// ============================================================================
// CLI Definitions
// ============================================================================

#[derive(Subcommand)]
pub enum SessionCommand {
    /// Combine song projects into a single setlist RPP (offline, no DAW needed)
    Combine {
        /// Path to .RPL file listing song RPP files
        input: String,
        /// Output .RPP file path (default: <input_stem> Combined.RPP)
        #[arg(short, long)]
        output: Option<String>,
        /// Gap between songs in measures (uses next song's tempo)
        #[arg(long, default_value = "2")]
        gap: u32,
        /// Trim each song to its marker bounds (PREROLL/=START/SONGSTART → POSTROLL/=END/SONGEND)
        #[arg(long, default_value = "true")]
        trim: bool,
    },
    /// Organize an RPP project into canonical FTS hierarchy (offline)
    ///
    /// Restructures tracks into: Click+Guide, Keyflow, TRACKS, Reference/Stem Split.
    /// Also organizes markers/regions into ruler lanes.
    Organize {
        /// Path to .RPP file to organize
        input: String,
        /// Output path (default: overwrite input)
        #[arg(short, long)]
        output: Option<String>,
        /// Generate Click, Count, and Guide MIDI items from regions
        #[arg(long)]
        guide: bool,
    },
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
    socket: Option<PathBuf>,
    cmd: SessionCommand,
    as_json: bool,
) -> Result<()> {
    match cmd {
        SessionCommand::Combine {
            ref input,
            ref output,
            gap,
            trim,
        } => {
            // Offline command — no DAW connection needed
            cmd_combine(input, output.as_deref(), gap, trim)
        }
        SessionCommand::Organize {
            ref input,
            ref output,
            guide,
        } => cmd_organize(input, output.as_deref(), guide),
        other => {
            // Live commands require Phase 3 RPC connection
            let cmd_name = match other {
                SessionCommand::Combine { .. } | SessionCommand::Organize { .. } => {
                    unreachable!()
                }
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
            let _ = (socket, as_json); // suppress unused warnings
            eyre::bail!(
                "Session command '{}' not yet implemented. \
                 Session CLI requires RPC connection to the session cell running in REAPER.",
                cmd_name
            )
        }
    }
}

// ============================================================================
// Combine Command
// ============================================================================

// r[impl combined.cli.combine]
/// Combine song RPP files into a single setlist project.
///
/// When `trim` is true, each song is trimmed to its marker-defined bounds:
/// - PREROLL → POSTROLL (highest priority)
/// - =START → =END
/// - SONGSTART → SONGEND
/// - First section → last section
///
/// This removes silence/content outside the performance range, producing
/// a tightly consolidated combined project.
pub fn cmd_combine(input: &str, output: Option<&str>, gap_measures: u32, trim: bool) -> Result<()> {
    use daw::file::setlist_rpp::{self, CombineOptions};
    use std::path::Path;

    let input_path = Path::new(input);
    if !input_path.exists() {
        eyre::bail!("Input file not found: {}", input);
    }

    // Determine output path
    let output_path = if let Some(out) = output {
        PathBuf::from(out)
    } else {
        let stem = input_path.file_stem().unwrap_or_default();
        let parent = input_path.parent().unwrap_or(Path::new("."));
        parent.join(format!("{} Combined.RPP", stem.to_string_lossy()))
    };

    let options = CombineOptions {
        gap_measures,
        trim_to_bounds: trim,
    };

    // Parse RPL or treat as single RPP
    let is_rpl = input_path
        .extension()
        .map_or(false, |ext| ext.eq_ignore_ascii_case("rpl"));

    let (combined_text, song_infos) = if is_rpl {
        setlist_rpp::combine_rpl(input_path, &options)?
    } else {
        setlist_rpp::combine_rpp_files(&[input_path.to_path_buf()], &options)?
    };

    std::fs::write(&output_path, &combined_text)?;

    // Print summary
    println!(
        "Combined {} songs → {}",
        song_infos.len(),
        output_path.display()
    );
    if gap_measures > 0 {
        println!("Gap: {} measure(s) between songs", gap_measures);
    }
    if trim {
        println!("Trimmed to marker bounds (PREROLL/=START/SONGSTART → POSTROLL/=END/SONGEND)");
    }
    println!();

    let mut total = 0.0;
    for (i, info) in song_infos.iter().enumerate() {
        let dur_min = (info.duration_seconds / 60.0).floor();
        let dur_sec = info.duration_seconds % 60.0;
        println!(
            "  {:>2}. {:<40} {:>6.1}s  ({:.0}:{:02.0})",
            i + 1,
            info.name,
            info.global_start_seconds,
            dur_min,
            dur_sec,
        );
        total = info.global_start_seconds + info.duration_seconds;
    }
    println!();
    println!(
        "Total: {:.0}:{:02.0}",
        (total / 60.0).floor(),
        total % 60.0,
    );

    Ok(())
}

// ============================================================================
// Organize Lanes Command
// ============================================================================

// r[impl combined.cli.organize]
/// Organize an RPP project into canonical FTS hierarchy.
///
/// Restructures tracks into:
/// - Click + Guide/ (Click, Loop, Count, Guide)
/// - Keyflow/ (CHORDS, LINES, HITS)
/// - TRACKS/ (content tracks)
/// - Reference/ (Mix + Stem Split/)
///
/// Also organizes markers/regions into FTS ruler lanes.
pub fn cmd_organize(input: &str, output: Option<&str>, generate_guide: bool) -> Result<()> {
    use daw::file::setlist_rpp;
    use std::path::Path;

    let input_path = Path::new(input);
    if !input_path.exists() {
        eyre::bail!("Input file not found: {}", input);
    }

    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| input_path.to_path_buf());

    // Parse the project
    let content = std::fs::read_to_string(input_path)?;
    let mut project = daw::file::parse_project_text(&content)?;

    let original_track_count = project.tracks.len();
    let source_dir = input_path.parent().unwrap_or(Path::new("."));

    // Resolve relative media paths to absolute
    for track in &mut project.tracks {
        resolve_track_paths(track, source_dir);
    }

    // Reorganize tracks into FTS hierarchy
    project.tracks = daw::file::types::organize_into_fts_hierarchy(project.tracks);

    // Generate guide MIDI items if requested
    if generate_guide {
        let (click, count, guide) = daw::file::guide_gen::generate_guide_items(&project);
        for track in &mut project.tracks {
            let lower = track.name.to_lowercase();
            match lower.as_str() {
                "click" => track.items.extend(click.clone()),
                "count" => track.items.extend(count.clone()),
                "guide" => track.items.extend(guide.clone()),
                _ => {}
            }
        }
    }

    // Organize ruler lanes
    setlist_rpp::organize_ruler_lanes(&mut project);

    // Serialize
    let organized = setlist_rpp::project_to_rpp_text(&project);
    std::fs::write(&output_path, &organized)?;

    println!(
        "Organized {} tracks into FTS hierarchy → {}",
        original_track_count,
        output_path.display()
    );
    println!("  Click + Guide, Keyflow, TRACKS, Reference/Stem Split");

    Ok(())
}

/// Resolve relative media paths in a track's items to absolute paths.
fn resolve_track_paths(track: &mut daw::file::types::track::Track, source_dir: &std::path::Path) {
    for item in &mut track.items {
        // Resolve parsed take sources
        for take in &mut item.takes {
            if let Some(ref mut source) = take.source {
                if !source.file_path.is_empty()
                    && !std::path::PathBuf::from(&source.file_path).is_absolute()
                {
                    let absolute = source_dir.join(&source.file_path);
                    source.file_path = absolute.to_string_lossy().to_string();
                }
            }
        }

        // Resolve FILE paths in raw_content
        if !item.raw_content.is_empty() {
            let mut patched = Vec::new();
            for line in item.raw_content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("FILE ") {
                    let file_path = trimmed
                        .trim_start_matches("FILE ")
                        .trim_matches('"');
                    if !std::path::PathBuf::from(file_path).is_absolute() {
                        let absolute = source_dir.join(file_path);
                        patched.push(format!("FILE \"{}\"", absolute.to_string_lossy()));
                        continue;
                    }
                }
                patched.push(line.to_string());
            }
            item.raw_content = patched.join("\n");
        }
    }
}
