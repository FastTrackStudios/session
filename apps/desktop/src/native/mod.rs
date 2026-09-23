//! The Session app on Blitz (dioxus-native) — see `docs/app-on-blitz.md`.
//!
//! The desktop build launches this instead of the WRY app when the `native`
//! feature is on. It is the app as panels (`session_daw::studio::Arrangement`,
//! the progress bar, …) composed into views under one top bar, with the
//! window's own title bar made transparent so the traffic lights sit in ours.
//!
//! The session is opened once, before the window, on the process's
//! daw-standalone engine, prepared the way `just studio-song` prepares it
//! (organize, build from the chart, generate the click and guide), and handed
//! to every panel as context.

mod shell;

use std::any::Any;
use std::path::{Path, PathBuf};

/// The developer's example session, used when it is there and nothing
/// else has been chosen: Always On Time, from the sessions checkout beside
/// this repo.
const EXAMPLE: &str = "/Volumes/build-disk/development/sessions/Always On Time/Always On Time.RPP";

/// What to open, in order: `FTS_SESSION_SETLIST` (a setlist); then
/// `FTS_SESSION_PROJECT` (one song); what was open last time; the example,
/// if this machine has it; and otherwise the user's pick from an Open
/// dialog. `None` when the dialog is cancelled.
///
/// A song is a setlist of one, so what comes back is always a list — with
/// the path it came from, which is what is remembered.
fn choose() -> Option<(PathBuf, Vec<PathBuf>)> {
    let target = std::env::var_os("FTS_SESSION_SETLIST")
        .or_else(|| std::env::var_os("FTS_SESSION_PROJECT"))
        .map(PathBuf::from)
        .or_else(|| remembered().filter(|p| p.exists()))
        .or_else(|| Some(PathBuf::from(EXAMPLE)).filter(|p| p.is_file()))
        .or_else(pick)?;
    let songs = songs_of(&target)?;
    Some((target, songs))
}

/// The songs `target` names: a setlist's (a folder of song folders, or a
/// `.setlist` file — see `session_daw::setlist::read_setlist`), or the one
/// song it is.
fn songs_of(target: &Path) -> Option<Vec<PathBuf>> {
    let setlist = (target.is_dir() && !session_daw::open::is_session(target))
        || target.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("setlist"));
    if !setlist {
        return Some(vec![target.to_path_buf()]);
    }
    session_daw::setlist::read_setlist(target)
        .inspect_err(|e| tracing::error!(error = %e, "the setlist could not be read"))
        .ok()
}

/// The Open dialog: a REAPER project.
fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a session")
        .add_filter("Song or setlist", &["RPP", "rpp", "setlist"])
        .pick_file()
}

/// Where the last session's path is kept.
fn memory() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Session").join("last-session"))
}

fn remembered() -> Option<PathBuf> {
    let text = std::fs::read_to_string(memory()?).ok()?;
    let path = text.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn remember(project: &Path) {
    let Some(file) = memory() else { return };
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, project.as_os_str().as_encoded_bytes()));
    if let Err(e) = written {
        // Not fatal: the next launch asks again.
        tracing::warn!(error = %e, "could not remember the session");
    }
}

/// Open the setlist — every song into the one engine, the first current —
/// then the window. Returns when the window closes, or straight away when
/// nothing was chosen.
///
/// A session that fails to open says why and offers the Open dialog again,
/// rather than quitting with no window and nothing on screen.
pub fn launch() {
    let mut chosen = choose();
    let setlist = loop {
        let Some((target, songs)) = chosen else { return };
        match session_daw::setlist::Setlist::open(&songs) {
            Ok(setlist) => {
                remember(&target);
                break setlist;
            }
            Err(e) => {
                tracing::error!(error = %e, "could not open the session");
                let again = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Could not open the session")
                    .set_description(format!("{}\n\n{e}", target.display()))
                    .set_buttons(rfd::MessageButtons::OkCancelCustom(
                        "Open Another…".into(),
                        "Quit".into(),
                    ))
                    .show();
                if again != rfd::MessageDialogResult::Custom("Open Another…".into()) {
                    return;
                }
                chosen = pick().and_then(|target| songs_of(&target).map(|songs| (target, songs)));
            }
        }
    };

    // `FTS_SESSION_AUTOPLAY=1` starts playing as the window opens — for
    // measuring a session while it plays without a hand on the mouse.
    if std::env::var("FTS_SESSION_AUTOPLAY").is_ok_and(|v| v != "0") {
        session_daw::engine::transport(session_daw::engine::Move::PlayStop, 0.0);
    }

    let attributes = window_attributes();
    let contexts: Vec<Box<dyn Fn() -> Box<dyn Any> + Send + Sync>> =
        vec![Box::new(move || Box::new(setlist.clone()) as Box<dyn Any>)];
    dioxus_native::launch_cfg(shell::Shell, contexts, vec![Box::new(attributes)]);
}

/// The window: our top bar IS the title bar. On macOS the native one is made
/// transparent and the content runs up under it, so the traffic lights sit
/// inside the app's own bar, as in the Claude app.
fn window_attributes() -> winit::window::WindowAttributes {
    let attributes = winit::window::WindowAttributes::default()
        .with_title("Session")
        // Opens filling the screen; this is the size it restores to when
        // un-maximized.
        .with_maximized(true)
        .with_surface_size(winit::dpi::LogicalSize::new(1600.0, 1000.0))
        .with_min_surface_size(winit::dpi::LogicalSize::new(720.0, 480.0));
    #[cfg(target_os = "macos")]
    let attributes = attributes.with_platform_attributes(Box::new(
        winit::platform::macos::WindowAttributesMacOS::default()
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true),
    ));
    attributes
}
