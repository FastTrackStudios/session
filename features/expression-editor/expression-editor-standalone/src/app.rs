//! The root component, and the hand-off that gets a document into it.
//!
//! `open_standalone_with_state` takes a bare `fn() -> Element`, and the
//! REAPER panel mounts a component with no props for the same reason:
//! a root that needed props would be a root only one host could build.
//! So the loaded document is *staged* before the window opens and the
//! component takes it on mount.
//!
//! Staging is a take, not a read: the document moves into the
//! component's signal and the slot is left empty. A component that
//! re-mounted would otherwise silently reset the user's edits back to
//! the loaded state.

use std::sync::Mutex;

use dioxus::prelude::*;
use expression_editor_core::{Editor, ExpressionDoc, TimeBase, Viewport};
use expression_editor_ui::{ExpressionEditor, theme};

use crate::drum_host::SharedDrumHost;

/// The document waiting for a window.
static STAGED: Mutex<Option<Editor>> = Mutex::new(None);

/// The drum host waiting beside it, when the window is a drum
/// workspace. Staged the same way and for the same reason: the root
/// takes no props.
static STAGED_HOST: Mutex<Option<SharedDrumHost>> = Mutex::new(None);

/// Hand a loaded document to the next [`App`] that mounts.
pub fn stage(editor: Editor) {
    *STAGED.lock().unwrap() = Some(editor);
}

/// Stage a drum workspace: the document *and* its write half, so the
/// panel's Apply and the slip drag land on the daw.
// r[impl drums.quantize.apply]
pub fn stage_with_host(editor: Editor, host: SharedDrumHost) {
    *STAGED.lock().unwrap() = Some(editor);
    *STAGED_HOST.lock().unwrap() = Some(host);
}

/// Take the staged document, if there is one.
pub fn take_staged() -> Option<Editor> {
    STAGED.lock().unwrap().take()
}

/// Take the staged drum host, if the staged document came with one.
pub fn take_staged_host() -> Option<SharedDrumHost> {
    STAGED_HOST.lock().unwrap().take()
}

/// The lane a gesture named, as its first visible member's track index.
fn lane_track(ed: &Editor, lane: &str) -> Option<usize> {
    use expression_editor_core::kit::LaneRole;
    let role = LaneRole::ALL.into_iter().find(|r| r.label() == lane)?;
    ed.tracks
        .role_members(role)
        .iter()
        .filter_map(|g| ed.tracks.index_of_guid(g))
        .find(|&i| ed.tracks.track(i).is_some_and(|t| !t.hidden))
}

/// Draw a hand-added hit: a slice note in the lane's document, from the
/// refined onset to the next hit. The daw is untouched — this is the
/// hit *list* changing, which is all the spec allows before a drag or
/// Apply. r[impl drums.manual.add-remove]
fn add_hit_note(editor: &mut Signal<Editor>, lane: &str, at: f64) {
    let mut ed = editor.write();
    let Some(track) = lane_track(&ed, lane) else {
        return;
    };
    // The active document is the only writable one; selecting the hit's
    // lane is what a click there does anyway.
    ed.switch_track(track);
    let ups = ed.doc.time_base.units_per_second(ed.bpm);
    let start = at * ups;
    let end = ed
        .doc
        .notes
        .iter()
        .map(|n| n.start)
        .filter(|&s| s > start + 1e-6)
        .fold(ed.doc.end, f64::min);
    let id = expression_editor_core::doc::NoteId(
        ed.doc.notes.iter().map(|n| n.id.0).max().unwrap_or(0) + 1,
    );
    let mut note = expression_editor_core::Note::new(id, start, end.max(start + 1.0), 1);
    // A hand-placed hit is intent — full weight, like the host's list.
    note.velocity = 1.0;
    note.weight = 1.0;
    ed.doc.push(note);
}

/// Erase a hit's slice note from the lane's document.
/// r[impl drums.manual.add-remove]
fn remove_hit_note(editor: &mut Signal<Editor>, lane: &str, hit: f64) {
    let mut ed = editor.write();
    let Some(track) = lane_track(&ed, lane) else {
        return;
    };
    ed.switch_track(track);
    let ups = ed.doc.time_base.units_per_second(ed.bpm);
    let target = hit * ups;
    let tol = 0.015 * ups;
    ed.doc.notes.retain(|n| (n.start - target).abs() > tol);
}

/// After an edit lands on the daw, re-read the kit and swap every
/// member's document, so the lanes draw the audio where it now is —
/// the drawing must follow the writes or it shows where hits *were*.
fn refresh_docs(editor: &mut Signal<Editor>, host: &crate::drum_host::SharedDrumHost) {
    let docs = host.refresh();
    let mut ed = editor.write();
    for (guid, doc) in docs {
        ed.reload_track_doc(&guid, doc);
    }
}

/// Re-find the fills after an edit and update the bands.
///
/// Separate from [`refresh_docs`] because it must run *after* it: the
/// host drops its cached fills during `refresh`, so asking any earlier
/// returns the fills of the audio as it used to be.
// r[impl drums.fills.draw]
fn refresh_fills(fills: &mut Signal<Vec<(f64, f64)>>, host: &crate::drum_host::SharedDrumHost) {
    let found = host.fills(&expression_editor_core::fills::FillConfig::default());
    fills.set(found.iter().map(|f| (f.start, f.end)).collect());
}

/// An empty document, for the case where nothing was staged.
///
/// Better than panicking: a window that opens empty is diagnosable, and
/// the runner has already printed what it loaded.
pub(crate) fn fallback() -> Editor {
    let doc = ExpressionDoc::new(TimeBase::Ppq { ppq: 960.0 }, 0.0, 960.0 * 8.0);
    Editor::new(doc, Viewport::new(1100.0, 520.0))
}

/// The host-backed callbacks an [`ExpressionEditor`] mount takes — one
/// bundle so the plain editor window and the workstation cannot wire
/// them differently.
pub(crate) struct HostCallbacks {
    pub on_change: Option<EventHandler<expression_editor_ui::QuantizePanel>>,
    pub on_apply: Option<EventHandler<expression_editor_ui::QuantizePanel>>,
    pub on_save: Option<EventHandler<()>>,
    pub on_hit: Option<EventHandler<expression_editor_ui::stack::HitGesture>>,
    pub on_undo: Option<EventHandler<()>>,
}

/// Build the callbacks against a host, or all-`None` without one (a
/// demo scene: the panel is then purely visual). Must be called from a
/// component body — the handlers capture the caller's signals.
pub(crate) fn host_callbacks(
    mut editor: Signal<Editor>,
    host: Option<SharedDrumHost>,
    mut bins: Signal<Vec<expression_editor_ui::quantize_panel::Bin>>,
    mut previews: Signal<Vec<expression_editor_ui::quantize_panel::HitPreview>>,
    mut fills: Signal<Vec<(f64, f64)>>,
) -> HostCallbacks {
    // The fills as loaded, so the bands are on screen before the user
    // touches the panel — they are what a quantize will leave alone, and
    // seeing that afterwards is too late to be useful.
    // r[impl drums.fills.draw]
    if let Some(h) = host.clone() {
        let found = h.fills(&expression_editor_core::fills::FillConfig::default());
        fills.set(found.iter().map(|f| (f.start, f.end)).collect());
    }
    // Undo the daw's edit, not the document's. Every gesture here wrote
    // through a `begin_undo_block`/`end_undo_block` pair and none of it
    // touched the document's own history, so the document stack has
    // nothing to rewind and rewinding it would leave the slip on disk
    // while the picture claimed otherwise.
    // r[impl drums.manual.undo]
    let on_undo = host.clone().map(|h| {
        EventHandler::new(move |()| {
            if h.undo() {
                refresh_docs(&mut editor, &h);
                refresh_fills(&mut fills, &h);
            }
        })
    });
    let on_change = host.clone().map(|h| {
        EventHandler::new(move |p: expression_editor_ui::QuantizePanel| {
            let (b, pv) = h.preview(&p);
            bins.set(b);
            previews.set(pv);
        })
    });
    // r[impl drums.quantize.apply]
    let on_apply = host.clone().map(|h| {
        EventHandler::new(
            move |p: expression_editor_ui::QuantizePanel| match h.apply(&p) {
                Ok(done) => {
                    tracing::info!(pieces = done.pieces, items = done.items, "quantized kit");
                    refresh_docs(&mut editor, &h);
                    refresh_fills(&mut fills, &h);
                }
                Err(e) => tracing::warn!(error = ?e, "quantize refused"),
            },
        )
    });
    // r[impl drums.save.new-file]
    let on_save = host.clone().map(|h| {
        EventHandler::new(move |_| match h.save() {
            Ok(path) => tracing::info!(path = %path.display(), "saved as a new .rpp"),
            Err(e) => tracing::warn!(error = %e, "save failed"),
        })
    });
    // The hand gestures, all four, through one dispatcher — they share
    // a host, a group and an undo discipline, so they share a handler.
    // r[impl drums.manual.slip]
    // r[impl drums.manual.stretch]
    // r[impl drums.manual.add-remove]
    let on_hit = host.map(|h| {
        EventHandler::new(move |g: expression_editor_ui::stack::HitGesture| {
            use expression_editor_ui::stack::HitGesture as G;
            match g {
                G::Slip { hit, next, delta } => {
                    let next = if next.is_finite() { next } else { h.take_secs };
                    let cfg = expression_editor_audio::quantize::SplitConfig {
                        leading_pad_secs: 0.005,
                        crossfade_secs: 0.005,
                    };
                    match h.slip(hit, next, delta, cfg) {
                        Ok(done) => {
                            tracing::info!(pieces = done.pieces, "slipped hit");
                            refresh_docs(&mut editor, &h);
                            refresh_fills(&mut fills, &h);
                    refresh_fills(&mut fills, &h);
                        }
                        Err(e) => tracing::warn!(error = ?e, "slip refused"),
                    }
                }
                G::Stretch {
                    hit,
                    prev,
                    next,
                    delta,
                    both,
                } => {
                    let prev = if prev.is_finite() { prev } else { 0.0 };
                    let next = if next.is_finite() { next } else { h.take_secs };
                    match h.stretch(hit, prev, next, delta, both) {
                        Ok(done) => {
                            tracing::info!(items = done.items, "stretched hit");
                            refresh_docs(&mut editor, &h);
                            refresh_fills(&mut fills, &h);
                    refresh_fills(&mut fills, &h);
                        }
                        Err(e) => tracing::warn!(error = ?e, "stretch refused"),
                    }
                }
                // r[impl drums.manual.split]

                G::Split { at } => {

                    let cfg = expression_editor_audio::quantize::SplitConfig {

                        leading_pad_secs: 0.005,

                        crossfade_secs: 0.005,

                    };

                    match h.split(at, cfg) {

                        Ok(done) => {

                            tracing::info!(items = done.items, "split kit");

                            refresh_docs(&mut editor, &h);

                            refresh_fills(&mut fills, &h);

                        }

                        Err(e) => tracing::warn!(error = ?e, "split refused"),

                    }

                }

                G::Add { lane, at } => {
                    let landed = h.add_hit(at, 0.05);
                    tracing::info!(%lane, at, landed, "added hit");
                    add_hit_note(&mut editor, &lane, landed);
                }
                G::Remove { lane, hit } => {
                    h.remove_hit(hit);
                    tracing::info!(%lane, hit, "removed hit");
                    remove_hit_note(&mut editor, &lane, hit);
                }
            }
        })
    });
    HostCallbacks {
        on_change,
        on_apply,
        on_save,
        on_hit,
        on_undo,
    }
}

/// The whole window: the editor, and nothing else.
///
/// No arrangement view, no mixer, no transport. The editor is the
/// product being built here; the composed window with the arrange view
/// and the mixer is [`crate::workstation`].
#[component]
pub fn App() -> Element {
    let editor = use_signal(|| take_staged().unwrap_or_else(fallback));
    let host = use_signal(take_staged_host);
    // The panel's data channel: bins and previews recomputed by the
    // host on every control change. Empty without a host — the panel
    // is then purely visual, which is what a demo scene wants.
    let bins = use_signal(Vec::new);
    let previews = use_signal(Vec::new);
    // The fills, as (start, end) seconds, for the bands the stack draws.
    let fills = use_signal(Vec::<(f64, f64)>::new);
    let HostCallbacks {
        on_change,
        on_apply,
        on_save,
        on_hit,
        on_undo,
    } = host_callbacks(editor, host.read().clone(), bins, previews, fills);
    rsx! {
        style {
            // Blitz sizes the root from these; without them the editor
            // lays out at its intrinsic height and the status bar
            // leaves the frame.
            "html, body {{ width: 100%; height: 100%; margin: 0; padding: 0; \
              overflow: hidden; background: {theme::BG}; }}"
        }
        div {
            // `vh`/`vw` rather than `100%`: percentage heights resolve
            // against the parent, and a headless Blitz mount gives
            // `body` no resolved height, so the editor would lay out to
            // its content and leave a band of background under the
            // status bar in every screenshot. The viewport units are
            // the window either way.
            style: "width: 100vw; height: 100vh;",
            ExpressionEditor {
                editor,
                quantize_bins: bins(),
                quantize_previews: previews(),
                on_quantize_change: on_change,
                on_quantize_apply: on_apply,
                on_hit,
                on_save,
                on_undo,
                fills: fills(),
            }
        }
    }
}
