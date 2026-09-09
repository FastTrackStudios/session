//! The workstation in a WRY webview, via dioxus-desktop.
//!
//! ```sh
//! cargo run -p expression-editor-standalone --features webview \
//!     --example webview -- song.rpp --drums --size 1920x1080
//! ```
//!
//! The same components as `--example workstation`, rendered by a browser
//! engine instead of Blitz. This is the design vehicle: devtools, real
//! CSS, and a DOM that does not fall over when a pane adds and removes
//! nodes — which is where the panels get shaped before they are ported
//! back to the native renderer.
//!
//! Everything below the UI is unchanged and native: the project loads,
//! the daw facade runs in-process and the audio engine plays, exactly as
//! it does under `--example workstation`. Only the renderer differs.

use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use expression_editor_standalone::cli::ArgsError;
use expression_editor_standalone::workstation::{
    WorkstationApp, bootstrap_daw_blocking, stage_workstation,
};
use expression_editor_standalone::{Args, Runner};

fn main() {
    let args = match Args::from_env() {
        Ok(a) => a,
        Err(e @ (ArgsError::Help | ArgsError::List)) => {
            print!("{e}");
            return;
        }
        Err(e) => {
            eprintln!("{e}\n\n{}", expression_editor_standalone::cli::USAGE);
            std::process::exit(2);
        }
    };

    let runner = match Runner::open(&args.source, &args.target, args.viewport(), args.mode) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let Some(standalone) = runner.daw.clone() else {
        eprintln!("the workstation needs a project — open a .rpp, not a demo scene");
        std::process::exit(1);
    };

    // The in-process daw facade the arrange + mixer panels read, and
    // live meters for the strips.
    if let Err(e) = bootstrap_daw_blocking(&standalone) {
        eprintln!("daw bootstrap failed: {e}");
        std::process::exit(1);
    }
    let track_count =
        daw::service::Tracks::all(&standalone, daw::service::ProjectContext::Current).len();
    standalone.set_meters(daw::standalone::metering::Meters::new(track_count));

    // Real playback: the audio engine renders the project graph into
    // the default output (PipeWire on Linux) and drives the transport
    // clock sample-accurately. Kept alive for the window's life —
    // dropping it stops the stream. Failure is not fatal: the soft
    // clock still moves the playhead, just silently, and a machine
    // with no output device should still open the editor.
    let project_guid =
        daw::service::Projects::info(&standalone, daw::service::ProjectContext::Current)
            .map(|i| i.guid)
            .unwrap_or_default();
    // Inside the bootstrap's runtime: the engine spawns tasks on
    // construction, and a plain `main` has no reactor of its own.
    match expression_editor_standalone::workstation::in_daw_runtime(|| {
        standalone.attach_audio_engine(&project_guid)
    }) {
        Ok(engine) => {
            Box::leak(Box::new(engine));
        }
        Err(e) => eprintln!("no audio engine ({e}); transport will run silent"),
    }

    println!("{} — workstation", runner.label);
    stage_workstation(
        runner.loaded.into_editor(),
        runner.host,
        (args.width as f64, args.height as f64),
    );

    let window = WindowBuilder::new()
        .with_title(format!("FastTrackStudio — {}", runner.label))
        .with_inner_size(LogicalSize::new(args.width as f64, args.height as f64));
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(WorkstationApp);
}
