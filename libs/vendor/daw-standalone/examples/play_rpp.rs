//! Load an RPP project, decode its audio sources, and play it through
//! the default cpal output via `AudioEngine::attached_to`.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p daw-standalone --features rpp-loader --example play_rpp -- /path/to/song.rpp
//! ```
//!
//! Pipeline:
//! 1. `project_loader::load_rpp` parses the file + materializes audio
//!    sources via a `std::fs::read` resolver.
//! 2. `Standalone::attach_audio_engine(guid)` builds a cpal output
//!    stream whose callback drives `ProjectRenderer` every block.
//! 3. The transport service kicks playback via `Transport::play`.

use std::path::PathBuf;
use std::time::Duration;

use daw_proto::ProjectContext;
use daw_proto::transport::service::Transport;
use daw_standalone::media_bay::ProjectRelativeResolver;
use daw_standalone::project_loader::load_rpp_via_bay;
use daw_standalone::sync::Standalone;

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: play_rpp <rpp-file>")?;
    let rpp_path = PathBuf::from(&path);
    let rpp_text = std::fs::read_to_string(&rpp_path).map_err(|e| e.to_string())?;
    let project_dir = rpp_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let daw = Standalone::new();
    let project_name = rpp_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project");

    // Install a project-relative filesystem resolver on the bay.
    // RPP source paths are usually relative to the project dir;
    // `ProjectRelativeResolver` handles that uniformly. On WASM the
    // app installs a JS-backed resolver here instead.
    daw.media_bay()
        .set_file_resolver(Box::new(ProjectRelativeResolver::new(project_dir.clone())));

    println!("Loading {path}...");
    let (proj, audio) = load_rpp_via_bay(
        &daw,
        project_name,
        rpp_path.to_string_lossy().as_ref(),
        &rpp_text,
    )?;

    println!(
        "  tracks={} items={} takes={} markers={} regions={} tempo_points={} hw_outs={}",
        proj.track_count,
        proj.item_count,
        proj.take_count,
        proj.marker_count,
        proj.region_count,
        proj.tempo_point_count,
        proj.hw_output_count,
    );
    println!(
        "  decoded {} audio sources ({} failed, {} no source)",
        audio.loaded,
        audio.failed.len(),
        audio.skipped_no_source,
    );
    for (take, err) in &audio.failed {
        eprintln!("    ! {take}: {err}");
    }

    // Attach the cpal audio engine. The output callback now renders
    // the loaded project every block — no more separate AudioEngine
    // track list to populate.
    let _engine = daw.attach_audio_engine(&proj.project_guid)?;
    let ctx = ProjectContext::Project(proj.project_guid.clone());
    Transport::play(&daw, ctx.clone()).map_err(|e| format!("play failed: {e:?}"))?;

    println!("Playing. Ctrl-C to stop.");
    // Poll position so the user sees something happen. The cpal stream
    // is driven from its own thread; we just sleep here.
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let pos = Transport::get_position(&daw, ctx.clone());
        print!("\r  pos = {pos:7.3} s");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
}
