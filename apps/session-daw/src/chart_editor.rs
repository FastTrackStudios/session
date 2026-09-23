//! Organize mode's chart editor: the song's `.kf`, edited live.
//!
//! The text is the editor's (`editor-view`, Blitz-native), coloured and
//! checked as keyflow as it is typed (`editor-keyflow-lang`: syntax colours,
//! live diagnostics). Each pause in typing lays the chart over the song —
//! the tempo, the markers and section regions, the key and the chords are
//! rebuilt, and the click, count and cues regenerated
//! ([`crate::prepare::apply_chart`]) — then the chart is saved to its `.kf`
//! and the song to its `.session`, the chart panel shows the new chart
//! ([`crate::chart_panel::publish_live`]) and the arrangement reads the
//! song back ([`crate::studio::request_resync`]).
//!
//! Text that does not parse changes nothing in the song: half-typed is the
//! normal state of a chart being edited, and the diagnostics say what is
//! wrong. The work runs on a thread of its own, a quarter-second or so
//! after the last keystroke, so typing never waits on it.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use dioxus::prelude::*;
use futures_util::StreamExt as _;

use crate::shell::{DIM, RULE, TEXT};
use crate::studio::StudioSession;

/// How long typing has to pause before the chart is laid over the song.
const PAUSE: Duration = Duration::from_millis(350);

/// What the last pause did.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    /// Laid over the song: its sections and tempo, for the status line.
    Applied { sections: usize, bpm: f64 },
    /// Did not parse, or the song refused it — the song is as it was.
    Refused(String),
}

/// The editor pane.
#[component]
pub fn ChartEditor() -> Element {
    let session: StudioSession = use_context();
    let file = session.chart_file.clone();
    let initial = use_hook(|| {
        file.as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default()
    });
    let state = use_signal(|| editor_state::EditorState::new(&initial));
    let mut outcome = use_signal(|| None::<Outcome>);
    // The worker's replies, back on this thread: a channel the worker can
    // send on from its own, which wakes the window when it does.
    let replies = use_coroutine(move |mut rx: UnboundedReceiver<Outcome>| async move {
        while let Some(reply) = rx.next().await {
            outcome.set(Some(reply));
        }
    });
    let worker = use_hook(|| {
        let project = crate::open::current_song().unwrap_or_default();
        start(project, file.clone(), replies.tx())
    });
    // Every change of text goes to the worker, which waits for the pause.
    let mut last = use_signal(|| initial.clone());
    use_effect(move || {
        let text = state.read().doc.to_string();
        if *last.peek() != text {
            last.set(text.clone());
            let _ = worker.send(text);
        }
    });

    let status = match outcome() {
        None => ("", DIM.to_owned()),
        Some(Outcome::Applied { .. }) => ("", "#4ac26b".to_owned()),
        Some(Outcome::Refused(_)) => ("", "#e3b341".to_owned()),
    };
    let message = match outcome() {
        None => file
            .as_ref()
            .and_then(|f| f.file_name())
            .map_or_else(|| "no chart file".to_owned(), |n| n.to_string_lossy().into_owned()),
        Some(Outcome::Applied { sections, bpm }) => format!("laid over the song — {sections} sections, {bpm} bpm"),
        Some(Outcome::Refused(why)) => format!("not applied: {why}"),
    };
    let css = keyflow_editor_lang::highlight_css(&keyflow_editor_lang::HighlightTheme::default_dark());
    rsx! {
        style { dangerous_inner_html: editor_view::EDITOR_CSS }
        style { dangerous_inner_html: "{css}" }
        style { dangerous_inner_html: EDITOR_THEME }
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                    flex-direction:column; background:#101114;",
            // Clicking in here gives this pane the keyboard: the space bar
            // types a space rather than playing.
            onmousedown: move |event| {
                crate::keys::set_editing(true);
                event.stop_propagation();
            },
            div {
                style: "flex:none; display:flex; align-items:center; gap:8px; height:26px; \
                        padding:0 10px; border-bottom:1px solid {RULE}; font-size:11px;",
                span { style: "width:7px; height:7px; border-radius:4px; background:{status.1};" }
                span {
                    style: "color:{DIM}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                    "{message}"
                }
            }
            div {
                style: "flex:1; min-height:0; overflow:auto; padding:8px 12px; color:{TEXT}; \
                        font-family: ui-monospace, Menlo, monospace; font-size:13px;",
                editor_view::Editor {
                    state,
                    decorations: Some(editor_view::DecorationSource::ptr(
                        keyflow_editor_lang::keyflow_decorations,
                    )),
                }
            }
        }
    }
}

/// The editor's own palette tokens, dark, for the stylesheet's variables.
const EDITOR_THEME: &str = "
:root {
    --background: #101114;
    --muted: #1a1c20;
    --foreground: #e5e7eb;
    --muted-foreground: #8b9099;
    --primary: #3aa0ff;
}
";

/// Start the worker that lays chart text over `project`: it takes the
/// latest text once typing has paused, applies it, saves, and replies.
fn start(project: String, file: Option<PathBuf>, reply: futures_channel::mpsc::UnboundedSender<Outcome>) -> mpsc::Sender<String> {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("session-chart-editor".into())
        .spawn(move || {
            while let Ok(mut text) = rx.recv() {
                // Coalesce: whatever arrives before the pause replaces it.
                while let Ok(newer) = rx.recv_timeout(PAUSE) {
                    text = newer;
                }
                let outcome = apply(&project, file.as_deref(), &text);
                let _ = reply.unbounded_send(outcome);
            }
        })
        .ok();
    tx
}

/// Lay `text` over the song, save it, and say how it went.
fn apply(project: &str, file: Option<&std::path::Path>, text: &str) -> Outcome {
    let chart = match keyflow::parse(text) {
        Ok(chart) => chart,
        Err(e) => return Outcome::Refused(e.to_string()),
    };
    // Another song picked while this was waiting: its engine project is not
    // this one's, and the guide works on the current song.
    if crate::open::current_song().as_deref() != Some(project) {
        return Outcome::Refused("another song is up".to_owned());
    }
    let applied = crate::open::with_engine(|daw| {
        let rebuilt = crate::prepare::apply_chart(daw, project, text, true)?;
        // Saved where the song was opened from, when that is a `.session`.
        let saved = daw
            .read_project(project, |p| std::path::PathBuf::from(&p.info.path))
            .filter(|path| crate::open::is_session(path));
        if let Some(dir) = saved
            && let Err(e) = crate::session_file::save_session(daw, project, &dir)
        {
            tracing::warn!(session.save_error = %e, "chart editor: the song could not be saved");
        }
        Ok::<_, eyre::Report>(rebuilt)
    });
    match applied {
        Ok(rebuilt) => {
            if let Some(path) = file
                && let Err(e) = std::fs::write(path, text)
            {
                tracing::warn!(error = %e, "chart editor: the chart could not be saved");
            }
            crate::chart_panel::publish_live(
                project,
                std::sync::Arc::new(chart),
                Some(rebuilt.built.song_start_seconds),
            );
            crate::studio::request_resync();
            Outcome::Applied {
                sections: rebuilt.built.sections,
                bpm: rebuilt.built.tempo_bpm,
            }
        }
        Err(e) => Outcome::Refused(e.to_string()),
    }
}
