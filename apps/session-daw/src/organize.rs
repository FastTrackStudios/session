//! Organize mode's toolbar — the song's structure, placed from above the
//! arrangement.
//!
//! The same buttons, in the same order, as REAPER's "Organize 1" toolbar
//! (`nix/reaper-config/reaper-menu.ini`): the structural markers and the
//! section regions, then the time signatures. Each runs the session's own
//! action, so REAPER and this window insert the same thing:
//!
//! - a **section** fills the arrangement's time selection, or — with none —
//!   its default length from the edit cursor
//!   (`session::keyflow::actions`, `infer_insert_bounds`);
//! - a **marker** goes at the edit cursor;
//! - a **time signature** starts at the measure holding the edit cursor;
//!   with Shift held it lasts that one measure and the signature before it
//!   comes back after (`session::keyflow::time_signature`).
//!
//! The actions read the ENGINE's cursor and selection, and the arrangement
//! keeps its own, so each press hands the arrangement's over first
//! ([`crate::cursor::current`]). Then the guide is regenerated — its cues
//! follow the sections — and the arrangement reads the session back.

use dioxus::prelude::*;
use session::section_kinds::{KeyflowAction, MarkerKind, SectionKind};

use crate::shell::{DIM, RULE, TEXT};

/// One button of the toolbar.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Button {
    Marker(MarkerKind),
    Section(SectionKind),
    TimeSig(i32, i32),
}

/// The structure buttons, as the REAPER toolbar has them.
const STRUCTURE: [(Button, &str); 12] = [
    (Button::Marker(MarkerKind::CountIn), "Count-In"),
    (Button::Marker(MarkerKind::Start), "=START"),
    (Button::Marker(MarkerKind::SongStart), "SONGSTART"),
    (Button::Section(SectionKind::Intro), "INTRO"),
    (Button::Section(SectionKind::Verse), "VERSE"),
    (Button::Section(SectionKind::PreChorus), "PRE-CH"),
    (Button::Section(SectionKind::Chorus), "CHORUS"),
    (Button::Section(SectionKind::Bridge), "BRIDGE"),
    (Button::Section(SectionKind::Outro), "OUTRO"),
    (Button::Section(SectionKind::End), "ENDING"),
    (Button::Marker(MarkerKind::SongEnd), "SONGEND"),
    (Button::Marker(MarkerKind::End), "=END"),
];

impl Button {
    fn color(self) -> String {
        match self {
            Self::Marker(kind) => kind.css_color(),
            Self::Section(kind) => kind.css_color(),
            Self::TimeSig(..) => DIM.to_owned(),
        }
    }

    fn title(self) -> String {
        match self {
            Self::Marker(kind) => format!("Insert a {kind:?} marker at the edit cursor"),
            Self::Section(kind) => format!(
                "Insert a {kind:?} region over the time selection — or, with none, its default length from the edit cursor"
            ),
            Self::TimeSig(num, den) => format!(
                "Insert {num}/{den} at the measure under the edit cursor — Shift: for that one measure only"
            ),
        }
    }
}

/// The toolbar: the structure, a gap, the time signatures.
#[component]
pub fn OrganizeToolbar() -> Element {
    let signatures = session::keyflow::time_signature::TIME_SIGNATURES;
    rsx! {
        div {
            style: "flex:none; display:flex; flex-wrap:wrap; align-items:center; gap:4px; \
                    padding:6px 8px; background:#141518; border-bottom:1px solid {RULE};",
            for (button, label) in STRUCTURE {
                ToolButton { button, label: label.to_owned() }
            }
            div { style: "width:14px; flex:none;" }
            for &(num, den) in signatures {
                ToolButton { button: Button::TimeSig(num, den), label: format!("{num}/{den}") }
            }
        }
    }
}

#[component]
fn ToolButton(button: Button, label: String) -> Element {
    let color = button.color();
    rsx! {
        button {
            title: button.title(),
            style: "height:24px; padding:0 9px; border-radius:5px; border:1px solid {RULE}; \
                    border-left:3px solid {color}; background:#1c1e22; color:{TEXT}; \
                    font-size:11px; font-weight:600; cursor:pointer; white-space:nowrap;",
            onclick: move |event| press(button, event.modifiers().shift()),
            "{label}"
        }
    }
}

/// Run a button's action on the engine, as the arrangement has the cursor
/// and selection, then bring the guide and the arrangement up to date.
fn press(button: Button, shift: bool) {
    use daw::service::transport::service::Transport as _;
    use daw::service::ProjectContext;
    crate::open::with_engine(|daw| {
        let project = ProjectContext::Current;
        if let Some(edit) = crate::cursor::current() {
            let handed = match edit.selection {
                Some(span) => daw.set_time_selection(project.clone(), span.start, span.end),
                None => daw.clear_time_selection(project.clone()),
            };
            if let Err(e) = handed.and_then(|()| daw.set_position(project.clone(), edit.at)) {
                tracing::warn!(error = %e, "organize: the edit cursor could not be handed to the engine");
            }
        }
        match button {
            Button::Marker(kind) => {
                session::keyflow::actions::dispatch(daw, KeyflowAction::InsertMarker(kind));
            }
            Button::Section(kind) => {
                session::keyflow::actions::dispatch(daw, KeyflowAction::InsertSection(kind));
            }
            Button::TimeSig(num, den) => {
                if let Err(e) = session::keyflow::time_signature::insert_time_signature(daw, num, den, shift) {
                    tracing::warn!(error = %e, num, den, "organize: the time signature was refused");
                }
            }
        }
        // The cues follow the sections; the click follows the meter.
        if let Err(e) = session::guide::Guide::new(daw.clone())
            .with_instrument(crate::prepare::GUIDE_INSTRUMENT)
            .generate(session::guide::GuideScope::All)
        {
            tracing::debug!(error = %e, "organize: no guide to regenerate yet");
        }
    });
    crate::studio::request_resync();
}
