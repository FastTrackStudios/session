//! The Session DAW window.
//!
//! ```sh
//! just daw                       # Set in Stone, from the practice staging
//! just daw unbreakable
//! cargo run -p session-daw -- "/path/to/song.rpp"
//! ```
//!
//! Opens a REAPER project and lets you move around inside it. The window
//! appears first and the project fills in behind it — see [`open`] for
//! why that ordering is the whole difference between "opens instantly"
//! and "stares at nothing for half a minute".
//!
//! The panels live in `daw_ui::studio`; this is launch, arguments and
//! the loader thread, and nothing else.

#[cfg(target_os = "linux")]
use session_daw::frame_rate;
use session_daw::{open, theme};

use std::path::PathBuf;

use daw_ui::studio::Studio;
use daw_ui::theming::ThemeProvider;
use dioxus::desktop::tao::dpi::LogicalPosition;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;

/// Where the practice staging leaves its copies. `just daw` prepares one
/// and passes its path; this is the fallback for a bare `cargo run`.
const PRACTICE_ENV: &str = "SESSION_DAW_PROJECT";

/// Where to put the window, as `x,y` in logical desktop coordinates.
///
/// For measuring: a benchmark that opens on the screen you are working
/// on is a benchmark you cannot leave running. `just daw-stress` and the
/// probe sweeps put their windows on the right-hand display.
///
/// **Honoured on X11/XWayland only.** A Wayland client cannot place its
/// own surface — the protocol has no such request, deliberately — so on
/// a native Wayland session this is ignored and the compositor decides.
/// Run the measurement under XWayland (`DISPLAY=:0 GDK_BACKEND=x11`) if
/// you want the placement to stick; it made no measurable difference to
/// the frame rate when that was tested.
const POSITION_ENV: &str = "FTS_WINDOW_POS";

fn main() {
    // A subscriber, or every `tracing` line in the window goes nowhere —
    // including the frame-rate readings, which are the point of having
    // the meter at all.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,daw_ui::studio::fps=info".into()),
        )
        .init();

    let Some(path) = project_path() else {
        eprintln!(
            "session-daw needs a project.\n\n  \
             cargo run -p session-daw -- <song.rpp>\n  \
             {PRACTICE_ENV}=<song.rpp> cargo run -p session-daw\n\n\
             `just daw` prepares the practice staging and passes it for you."
        );
        std::process::exit(2);
    };

    let label = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());

    // The project loads on a worker thread while the window is already
    // up. The UI waits for the facade rather than assuming it — see
    // `daw_ui::studio::project::fetch_when_ready`.
    std::thread::Builder::new()
        .name("session-daw-load".into())
        .spawn(move || match open::open_and_serve(&path) {
            Ok(opened) => tracing::info!(
                project.name = opened.name,
                project.tracks = opened.track_count,
                "project open"
            ),
            Err(e) => tracing::error!(error = %e, "the project did not open"),
        })
        .expect("spawn the project loader");

    let mut window = WindowBuilder::new()
        .with_title(format!("Session — {label}"))
        .with_inner_size(LogicalSize::new(1600.0, 900.0));
    if let Some((x, y)) = window_position() {
        window = window.with_position(LogicalPosition::new(x, y));
    }
    dioxus::LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window).with_menu(None))
        .launch(Themed);
}

/// The window, under its theme.
///
/// A wrapper rather than a prop on [`Studio`], because a root component
/// must be launchable as a bare `fn() -> Element` — and because the
/// theme is genuinely context: every component that draws reads it from
/// there rather than being handed it.
#[component]
fn Themed() -> Element {
    // Before anything renders: WebKit caps script-driven rendering
    // updates near 60 fps by default, and this window wants the display's
    // real refresh. Best-effort — see `frame_rate`.
    #[cfg(target_os = "linux")]
    use_hook(|| {
        use wry::WebViewExtUnix;
        let webview = dioxus::desktop::window().webview.webview();
        frame_rate::unlock(&webview);
    });
    let theme = use_hook(theme::resolve);
    rsx! {
        ThemeProvider { theme, Studio {} }
    }
}

/// Where to open the window, if asked. `x,y`, or nothing.
fn window_position() -> Option<(f64, f64)> {
    let raw = std::env::var(POSITION_ENV).ok()?;
    let (x, y) = raw.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// The project to open: the first argument, or the environment.
fn project_path() -> Option<PathBuf> {
    std::env::args()
        .nth(1)
        .filter(|a| !a.is_empty())
        .or_else(|| std::env::var(PRACTICE_ENV).ok())
        .map(PathBuf::from)
        .filter(|p| p.exists())
}
