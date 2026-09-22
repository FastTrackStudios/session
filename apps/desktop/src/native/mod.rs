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

mod progress;
mod shell;

use std::any::Any;
use std::path::{Path, PathBuf};

/// The developer's example session, used when it is there and nothing
/// else has been chosen: Always On Time, from the sessions checkout beside
/// this repo.
const EXAMPLE: &str = "/Volumes/build-disk/development/sessions/Always On Time/Always On Time.RPP";

/// Which session to open, in order: `FTS_SESSION_PROJECT` (and
/// `FTS_SESSION_CHART`, where an empty value skips the chart); the session
/// opened last time; the example, if this machine has it; and otherwise
/// the user's pick from an Open dialog. `None` when the dialog is
/// cancelled.
///
/// The chart, unless the environment names one, is the `.kf` beside the
/// project: a session folder carries its own.
fn choose() -> Option<(PathBuf, Option<PathBuf>)> {
    let project = std::env::var_os("FTS_SESSION_PROJECT")
        .map(PathBuf::from)
        .or_else(|| remembered().filter(|p| p.is_file()))
        .or_else(|| Some(PathBuf::from(EXAMPLE)).filter(|p| p.is_file()))
        .or_else(pick)?;
    let chart = match std::env::var_os("FTS_SESSION_CHART") {
        Some(path) if path.is_empty() => None,
        Some(path) => Some(PathBuf::from(path)),
        None => chart_beside(&project),
    };
    Some((project, chart))
}

/// The Open dialog: a REAPER project.
fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a session")
        .add_filter("REAPER project", &["RPP", "rpp"])
        .pick_file()
}

/// The one `.kf` chart in the project's folder, if there is exactly one.
/// Two would be a guess, and a wrong chart is worse than none.
fn chart_beside(project: &Path) -> Option<PathBuf> {
    let mut charts = std::fs::read_dir(project.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("kf")));
    let chart = charts.next()?;
    charts.next().is_none().then_some(chart)
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

/// Open the session, then the window. Returns when the window closes, or
/// straight away when no session was chosen.
///
/// A session that fails to open says why and offers the Open dialog again,
/// rather than quitting with no window and nothing on screen.
pub fn launch() {
    let mut chosen = choose();
    let session = loop {
        let Some((project, chart)) = chosen else { return };
        let prepare = session_daw::prepare::Prepare {
            organize: true,
            chart,
            guide: true,
        };
        match session_daw::studio::StudioSession::open(&project, &prepare) {
            Ok(session) => {
                remember(&project);
                break session;
            }
            Err(e) => {
                tracing::error!(error = %e, "could not open the session");
                let again = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Could not open the session")
                    .set_description(format!("{}\n\n{e}", project.display()))
                    .set_buttons(rfd::MessageButtons::OkCancelCustom(
                        "Open Another…".into(),
                        "Quit".into(),
                    ))
                    .show();
                if again != rfd::MessageDialogResult::Custom("Open Another…".into()) {
                    return;
                }
                chosen = pick().map(|project| {
                    let chart = chart_beside(&project);
                    (project, chart)
                });
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
        vec![Box::new(move || Box::new(session.clone()) as Box<dyn Any>)];
    dioxus_native::launch_cfg(shell::Shell, contexts, vec![Box::new(attributes)]);
}

/// The window: our top bar IS the title bar. On macOS the native one is made
/// transparent and the content runs up under it, so the traffic lights sit
/// inside the app's own bar, as in the Claude app.
fn window_attributes() -> winit::window::WindowAttributes {
    let attributes = winit::window::WindowAttributes::default()
        .with_title("Session")
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

#[cfg(test)]
mod tests {
    use super::chart_beside;

    /// The chart beside a project is used only when it is the only one.
    #[test]
    fn a_session_folder_s_one_chart_is_its_chart() {
        let dir = std::env::temp_dir().join(format!("session-chart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("Song.RPP");
        std::fs::write(&project, "").unwrap();
        assert_eq!(chart_beside(&project), None, "no chart at all");

        std::fs::write(dir.join("Song.kf"), "").unwrap();
        assert_eq!(chart_beside(&project), Some(dir.join("Song.kf")));

        std::fs::write(dir.join("Other.KF"), "").unwrap();
        assert_eq!(chart_beside(&project), None, "two charts is a guess");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
