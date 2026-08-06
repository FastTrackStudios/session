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
//! Styled with Tailwind against the FTS theme tokens (`bg-background`,
//! `text-foreground`, `border-border`, …) rather than hand-written CSS,
//! so it inherits the app's palette instead of carrying its own. Note
//! this departs from the signal-UI rule that Tailwind is additive-only —
//! Blitz's Tailwind support is good enough now to lay out with it, but
//! if the REAPER panel renders wrong, structural classes are the first
//! place to look.
//!
//! There is no music theory in this file. The grid, the labels and the
//! pitches all come from `keyflow::chord::palette`.

use dioxus::prelude::*;
use fts_ui::components::{Select, SelectContent, SelectItem};
use keyflow::chord::palette::{ChordCandidate, grid};
use keyflow::key::Key;
use keyflow::primitives::MusicalNote;

/// The theme tokens Tailwind's colour classes resolve against, plus the
/// host reset. Inlined rather than linked because Blitz doesn't load
/// external stylesheets reliably.
const FTS_THEME: &str = include_str!("../../../../libs/fts-ui/fts-ui/assets/fts-theme.css");
const APP_TAILWIND: &str =
    include_str!("../../../../apps/fasttrackstudio/assets/tailwind-signal.css");

/// Without `html,body{height:100%}` a `h-full` root resolves against
/// `auto` and the panel collapses to its content instead of filling the
/// window. Mirrors the reset `apps/fasttrackstudio` injects.
const HOST_RESET: &str = r#"
html, body { margin:0; padding:0; height:100%; width:100%; overflow:hidden; }
* { box-sizing: border-box; }
body > div { height:100%; }
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
    let tonic = use_signal(|| "C".to_string());
    let mode = use_signal(|| "major".to_string());
    let mut octave = use_signal(|| 4i32);
    let mut selected = use_signal(|| None::<(usize, usize)>);
    let mut fired = use_signal(Vec::<u8>::new);
    let mut fired_label = use_signal(String::new);

    let minor = mode() == "minor";
    let key = build_key(&tonic(), minor);
    let columns = grid(&key);
    let numerals = if minor { MINOR_NUMERALS } else { MAJOR_NUMERALS };

    rsx! {
        document::Style { {FTS_THEME} }
        document::Style { {APP_TAILWIND} }
        document::Style { {HOST_RESET} }

        div { class: "h-full w-full flex flex-col gap-3 p-3 bg-background text-foreground overflow-hidden",

            // ── Controls ────────────────────────────────────────────
            div { class: "flex items-center gap-3 flex-wrap",
                span { class: "text-[11px] uppercase tracking-wide text-muted-foreground", "Key" }
                div { class: "w-24",
                    Select {
                        value: tonic,
                        placeholder: "C".to_string(),
                        SelectContent {
                            for (i, t) in TONICS.iter().enumerate() {
                                SelectItem { value: t.to_string(), index: i, "{t}" }
                            }
                        }
                    }
                }
                div { class: "w-28",
                    Select {
                        value: mode,
                        placeholder: "major".to_string(),
                        SelectContent {
                            SelectItem { value: "major".to_string(), index: 0, "major" }
                            SelectItem { value: "minor".to_string(), index: 1, "minor" }
                        }
                    }
                }
                span { class: "text-[11px] uppercase tracking-wide text-muted-foreground ml-2", "Octave" }
                button {
                    class: "px-2 py-1 rounded-md border border-border bg-card hover:bg-accent text-sm",
                    onclick: move |_| octave -= 1,
                    "−"
                }
                span { class: "text-sm tabular-nums w-4 text-center", "{octave}" }
                button {
                    class: "px-2 py-1 rounded-md border border-border bg-card hover:bg-accent text-sm",
                    onclick: move |_| octave += 1,
                    "+"
                }
            }

            // ── Degree × variation grid ─────────────────────────────
            div { class: "flex-1 min-h-0 flex gap-1.5 overflow-auto",
                for (d, column) in columns.iter().enumerate() {
                    div { class: "flex-1 min-w-0 flex flex-col gap-1",
                        div { class: "rounded-md border border-border bg-card px-1 py-1.5 text-center",
                            div { class: "text-sm font-bold leading-tight", "{numerals[d]}" }
                            div { class: "text-[10px] text-muted-foreground leading-tight",
                                {column.first().map(|c: &ChordCandidate| c.label.clone()).unwrap_or_default()}
                            }
                        }
                        for (v, chord) in column.iter().enumerate() {
                            {
                                let notes = chord.notes(octave());
                                let label = chord.label.clone();
                                let is_sel = selected() == Some((d, v));
                                let class = if is_sel {
                                    "rounded-md border px-1 py-1.5 text-xs text-center cursor-pointer truncate bg-primary text-primary-foreground border-primary font-semibold"
                                } else {
                                    "rounded-md border px-1 py-1.5 text-xs text-center cursor-pointer truncate bg-secondary text-secondary-foreground border-border hover:bg-accent"
                                };
                                rsx! {
                                    div {
                                        class: "{class}",
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

            // ── Readout ─────────────────────────────────────────────
            div { class: "border-t border-border pt-2 text-[11px] font-mono text-muted-foreground",
                if fired().is_empty() {
                    "pick a chord — columns are scale degrees, rows are the types that fit the key"
                } else {
                    "{fired_label()} → {fired():?}"
                }
            }
        }
    }
}
