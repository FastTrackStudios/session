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
//!
//! That is Engine mode. In Remote and Cue (`--audio remote|cue`,
//! `--reaper [socket]`, `FTS_AUDIO_MODE`, or the top bar's audio menu last
//! time — see `session_daw::open::launch_mode`) nothing is opened here: the
//! window attaches to the system it drives and its open projects are the
//! set (`session_daw::setlist::Setlist::attach`).

#[cfg(target_os = "ios")]
mod ios_scene;
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
        || target
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("setlist"));
    if !setlist {
        return Some(vec![target.to_path_buf()]);
    }
    session_daw::setlist::read_setlist(target)
        .inspect_err(|e| tracing::error!(error = %e, "the setlist could not be read"))
        .ok()
}

/// The Open dialog: a REAPER project.
#[cfg(not(target_os = "ios"))]
fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a session")
        .add_filter("Song or setlist", &["RPP", "rpp", "setlist"])
        .pick_file()
}

/// A phone has no files to pick: a set is joined by its link.
#[cfg(target_os = "ios")]
fn pick() -> Option<PathBuf> {
    None
}

/// An error, and the choices `buttons` offers: the one picked, or `None`
/// (the window closed, or the last button, which is always "Quit").
#[cfg(not(target_os = "ios"))]
fn ask(title: &str, description: String, buttons: &[&str]) -> Option<String> {
    let dialog = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(description);
    let dialog = match buttons {
        [a, b, c] => dialog.set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            (*a).into(),
            (*b).into(),
            (*c).into(),
        )),
        [a, b] => dialog.set_buttons(rfd::MessageButtons::OkCancelCustom(
            (*a).into(),
            (*b).into(),
        )),
        _ => dialog,
    };
    match dialog.show() {
        rfd::MessageDialogResult::Custom(choice) if buttons.last() != Some(&choice.as_str()) => {
            Some(choice)
        }
        _ => None,
    }
}

/// On a phone the error is logged and the launch gives up (the page it
/// opens on says so).
#[cfg(target_os = "ios")]
fn ask(title: &str, description: String, _buttons: &[&str]) -> Option<String> {
    tracing::error!(dialog.title = title, dialog.description = %description, "launch: no dialog to ask on this platform");
    None
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
    // How this app dials a Session engine (Remote on another Session): the
    // connector lives with the app's iroh identity (`crate::remote`).
    #[cfg(feature = "session")]
    session_daw::open::set_engine_dialer(|address| {
        Box::pin(async move {
            let addr = crate::remote::EngineAddr::parse(&address).map_err(|e| eyre::eyre!(e))?;
            let engine = crate::remote::connect_engine_daw(&addr).await?;
            let daw = engine.daw.clone();
            Ok((daw, Box::new(engine) as session_daw::open::EngineConnection))
        })
    });
    let mode = session_daw::open::launch_mode();
    session_daw::open::set_mode(mode.clone());
    let remote = if mode.owns_project() {
        None
    } else {
        mode.target
    };
    let setlist = match remote {
        Some(target) => match attach(&target) {
            Some(setlist) => setlist,
            None => return,
        },
        None => match open_chosen() {
            Some(setlist) => setlist,
            None => return,
        },
    };
    run(setlist);
}

/// Attach to the system a Remote or Cue window drives. When it is not
/// there, say so and offer to try again, to open a session here in Engine
/// mode instead, or to quit — never quietly play the set here while
/// someone thinks REAPER is.
fn attach(target: &session_daw::open::RemoteTarget) -> Option<session_daw::setlist::Setlist> {
    loop {
        match session_daw::setlist::Setlist::attach(target) {
            Ok(setlist) => return Some(setlist),
            Err(e) => {
                tracing::error!(error = %e, audio.target = target.kind(), "could not attach to the system this window drives");
                let answer = ask(
                    "Could not reach it",
                    format!("{}\n\n{e}", target.describe()),
                    &["Try Again", "Open in Engine Mode", "Quit"],
                );
                match answer.as_deref() {
                    Some("Try Again") => {}
                    Some("Open in Engine Mode") => {
                        session_daw::open::set_mode(session_daw::open::ModeState::engine());
                        return open_chosen();
                    }
                    _ => return None,
                }
            }
        }
    }
}

/// Engine mode: open the chosen song or setlist into the engine.
fn open_chosen() -> Option<session_daw::setlist::Setlist> {
    let mut chosen = choose();
    let setlist = loop {
        let Some((target, songs)) = chosen else {
            return None;
        };
        match session_daw::setlist::Setlist::open(&songs) {
            Ok(setlist) => {
                remember(&target);
                break setlist;
            }
            Err(e) => {
                tracing::error!(error = %e, "could not open the session");
                let again = ask(
                    "Could not open the session",
                    format!("{}\n\n{e}", target.display()),
                    &["Open Another…", "Quit"],
                );
                if again.as_deref() != Some("Open Another…") {
                    return None;
                }
                chosen = pick().and_then(|target| songs_of(&target).map(|songs| (target, songs)));
            }
        }
    };
    Some(setlist)
}

/// The window, over `setlist`.
fn run(setlist: session_daw::setlist::Setlist) {
    // `FTS_SESSION_AUTOPLAY=1` starts playing as the window opens — for
    // measuring a session while it plays without a hand on the mouse.
    if std::env::var("FTS_SESSION_AUTOPLAY").is_ok_and(|v| v != "0") {
        session_daw::engine::transport(session_daw::engine::Move::PlayStop, 0.0);
    }

    // iOS 27 stops an app that has not adopted scenes: ours is registered
    // before UIKit starts, for Info.plist to name.
    #[cfg(target_os = "ios")]
    ios_scene::register();
    let attributes = window_attributes();
    let contexts: Vec<Box<dyn Fn() -> Box<dyn Any> + Send + Sync>> =
        vec![Box::new(move || Box::new(setlist.clone()) as Box<dyn Any>)];
    dioxus_native::launch_cfg(shell::Shell, contexts, vec![Box::new(attributes)]);
}

/// The window: our top bar IS the title bar. On macOS the native one is made
/// transparent and the content runs up under it, so the traffic lights sit
/// inside the app's own bar, as in the Claude app.
/// A phone's window is its screen: no size of our own to start from (a
/// desktop size taken literally leaves the first surface a different shape
/// from the screen, and the picture stays squeezed after the resize).
#[cfg(target_os = "ios")]
fn window_attributes() -> winit::window::WindowAttributes {
    winit::window::WindowAttributes::default().with_title("Session")
}

#[cfg(not(target_os = "ios"))]
fn window_attributes() -> winit::window::WindowAttributes {
    // `FTS_WINDOW_POS="x,y"` / `FTS_WINDOW_SIZE="WxH"` (logical pixels)
    // place the window instead of maximizing it — what `just duo` uses to
    // put two collaborating windows side by side.
    fn pair(var: &str, sep: char) -> Option<(f64, f64)> {
        let raw = std::env::var(var).ok()?;
        let (a, b) = raw.split_once(sep)?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    }
    let (pos, size) = (pair("FTS_WINDOW_POS", ','), pair("FTS_WINDOW_SIZE", 'x'));
    let (w, h) = size.unwrap_or((1600.0, 1000.0));
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title("Session")
        // Opens filling the screen unless placed; this is the size it
        // restores to when un-maximized.
        .with_maximized(pos.is_none() && size.is_none())
        .with_surface_size(winit::dpi::LogicalSize::new(w, h))
        // Small enough to be phone-sized: below ~700 wide (or ~500 tall)
        // the app takes its small-screen layout (`session_daw::compact`).
        .with_min_surface_size(winit::dpi::LogicalSize::new(320.0, 300.0));
    if let Some((x, y)) = pos {
        attributes = attributes.with_position(winit::dpi::LogicalPosition::new(x, y));
    }
    #[cfg(target_os = "macos")]
    let attributes = attributes.with_platform_attributes(Box::new(
        winit::platform::macos::WindowAttributesMacOS::default()
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true),
    ));
    attributes
}
