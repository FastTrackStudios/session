//! The chord panel.
//!
//! One component, two homes: `examples/panel.rs` runs it in a desktop
//! window for fast iteration, and `fts-extensions` registers it as a
//! REAPER panel rendered by Blitz. Keeping it a plain component with no
//! props is what allows that — context, not arguments (the same rule the
//! signal UI follows).
//!
//! Layout follows ChordGun: seven columns, one per scale degree, and down
//! each column the chord types that fit the key there. Picking a cell
//! selects that chord. Nothing offered is out of key, so scanning a
//! column is a musical choice rather than a filtering exercise.
//!
//! There is no music theory in this file, and there shouldn't be. The
//! grid, the labels and the pitches all come from
//! `keyflow::chord::palette`; this decides only layout and colour.

use dioxus::prelude::*;
use keyflow::chord::palette::{ChordCandidate, grid};
use keyflow::key::Key;
use keyflow::primitives::MusicalNote;

/// The app's compiled Tailwind sheet and theme tokens, inlined rather
/// than linked: Blitz doesn't load external stylesheets reliably, and
/// this is the same sheet `apps/fasttrackstudio` inlines. `chord-tool`
/// is in input.css's `@source` list, so classes used here are generated.
const APP_TAILWIND: &str = include_str!("../../../../apps/fasttrackstudio/assets/tailwind-signal.css");
const FTS_THEME: &str = include_str!("../../../../libs/fts-ui/fts-ui/assets/fts-theme.css");

/// Host reset. Without `html,body{height:100%}` a `height:100%` root
/// resolves against `auto` and the panel collapses to its content
/// instead of filling the window — which is exactly what it did before
/// this existed. Mirrors the reset `apps/fasttrackstudio` injects.
const HOST_RESET: &str = r#"
html, body { margin:0; padding:0; height:100%; width:100%; background:#18181b; overflow:hidden; }
* { box-sizing: border-box; }
#main, body > div { height:100%; }
"#;

/// Layout-critical values stay explicit rather than leaning on Tailwind:
/// Blitz treats Tailwind as additive, so anything structural has to hold
/// up without it.
const PANEL_CSS: &str = r#"
.ct-root { font-family: system-ui, sans-serif; background:#18181b; color:#e4e4e7;
  padding:12px; display:flex; flex-direction:column; gap:12px;
  height:100%; width:100%; overflow:hidden; }
.ct-bar { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
.ct-label { font-size:11px; text-transform:uppercase; letter-spacing:.06em; color:#a1a1aa; }
.ct-select { background:#27272a; color:#e4e4e7; border:1px solid #3f3f46; border-radius:6px;
  padding:5px 8px; font-size:13px; }
.ct-step { background:#27272a; color:#e4e4e7; border:1px solid #3f3f46; border-radius:6px;
  padding:4px 9px; font-size:13px; cursor:pointer; }
.ct-step:hover { background:#3f3f46; }

.ct-grid { display:flex; gap:6px; flex:1; align-items:stretch; min-height:0; overflow:auto; }
.ct-col { flex:1 1 0; display:flex; flex-direction:column; gap:4px; min-width:0; }
.ct-head { text-align:center; padding:5px 2px; border-radius:6px; background:#27272a;
  border:1px solid #3f3f46; }
.ct-head .numeral { display:block; font-size:14px; font-weight:700; color:#e4e4e7; }
.ct-head .root { display:block; font-size:10px; color:#a1a1aa; }

.ct-cell { background:#1e3a5f; border:1px solid #3b82f6; color:#dbeafe; border-radius:6px;
  padding:6px 4px; font-size:12px; text-align:center; cursor:pointer;
  overflow:hidden; text-overflow:ellipsis; }
.ct-cell:hover { background:#1e4d7f; }
.ct-cell.sel { background:#2563eb; border-color:#93c5fd; color:#fff; font-weight:600; }

.ct-fired { font-family:ui-monospace, monospace; font-size:11px; color:#71717a;
  border-top:1px solid #27272a; padding-top:8px; }
"#;

/// Every tonic, spelled the way charts spell them.
const TONICS: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

const MAJOR_NUMERALS: [&str; 7] = ["I", "ii", "iii", "IV", "V", "vi", "vii°"];
const MINOR_NUMERALS: [&str; 7] = ["i", "ii°", "III", "iv", "v", "VI", "VII"];

/// Parse a tonic into a key of the chosen mode, falling back to C so a
/// bad string can't blank the panel.
fn build_key(tonic: &str, minor: bool) -> Key {
    let root = MusicalNote::from_string(tonic)
        .unwrap_or_else(|| MusicalNote::from_string("C").expect("C parses"));
    if minor { Key::minor(root) } else { Key::major(root) }
}

#[component]
pub fn ChordToolPanel() -> Element {
    let mut tonic = use_signal(|| "C".to_string());
    let mut minor = use_signal(|| false);
    let mut octave = use_signal(|| 4i32);
    // Which cell is chosen, as (degree_index, variation_index).
    let mut selected = use_signal(|| None::<(usize, usize)>);
    let mut fired = use_signal(Vec::<u8>::new);
    let mut fired_label = use_signal(String::new);

    let key = build_key(&tonic(), minor());
    let columns = grid(&key);
    let numerals = if minor() { MINOR_NUMERALS } else { MAJOR_NUMERALS };

    rsx! {
        document::Style { {FTS_THEME} }
        document::Style { {APP_TAILWIND} }
        document::Style { {HOST_RESET} }
        document::Style { {PANEL_CSS} }
        div { class: "ct-root",
            div { class: "ct-bar",
                span { class: "ct-label", "Key" }
                select {
                    class: "ct-select",
                    value: "{tonic}",
                    onchange: move |e| { tonic.set(e.value()); selected.set(None); },
                    for t in TONICS {
                        option { value: "{t}", "{t}" }
                    }
                }
                select {
                    class: "ct-select",
                    value: if minor() { "minor" } else { "major" },
                    onchange: move |e| { minor.set(e.value() == "minor"); selected.set(None); },
                    option { value: "major", "major" }
                    option { value: "minor", "minor" }
                }
                span { class: "ct-label", "Octave" }
                button { class: "ct-step", onclick: move |_| octave -= 1, "-" }
                span { "{octave}" }
                button { class: "ct-step", onclick: move |_| octave += 1, "+" }
            }

            div { class: "ct-grid",
                for (d, column) in columns.iter().enumerate() {
                    div { class: "ct-col",
                        div { class: "ct-head",
                            span { class: "numeral", "{numerals[d]}" }
                            span { class: "root",
                                {column.first().map(|c: &ChordCandidate| c.label.clone()).unwrap_or_default()}
                            }
                        }
                        for (v, chord) in column.iter().enumerate() {
                            {
                                let notes = chord.notes(octave());
                                let label = chord.label.clone();
                                let is_sel = selected() == Some((d, v));
                                rsx! {
                                    div {
                                        class: if is_sel { "ct-cell sel" } else { "ct-cell" },
                                        onclick: move |_| {
                                            selected.set(Some((d, v)));
                                            fired.set(notes.clone());
                                            fired_label.set(label.clone());
                                        },
                                        "{chord.label}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "ct-fired",
                if fired().is_empty() {
                    "pick a chord — columns are scale degrees, rows are the types that fit the key"
                } else {
                    "{fired_label()} → {fired():?}"
                }
            }
        }
    }
}
