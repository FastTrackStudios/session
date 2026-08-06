//! The chord panel.
//!
//! One component, two homes: `examples/panel.rs` runs it in a desktop
//! window for fast iteration, and `fts-extensions` registers it as a
//! REAPER panel rendered by Blitz. Keeping it a plain component with no
//! props is what allows that — context, not arguments (the same rule the
//! signal UI follows).
//!
//! There is no music theory in this file, and there shouldn't be. Every
//! pitch on screen comes from `keyflow::chord::realize`.

use dioxus::prelude::*;
use keyflow::chord::realize::{ChordSize, chord_notes, scale_chords};
use keyflow::key::Key;
use keyflow::primitives::MusicalNote;

/// Blitz doesn't load external stylesheets reliably, so everything is
/// inline or embedded — the same constraint the signal UI documents.
const PANEL_CSS: &str = r#"
.ct-root { font-family: system-ui, sans-serif; background:#18181b; color:#e4e4e7;
  padding:12px; display:flex; flex-direction:column; gap:12px; height:100%; }
.ct-row { display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
.ct-label { font-size:11px; text-transform:uppercase; letter-spacing:.06em; color:#a1a1aa; min-width:64px; }
.ct-btn { background:#27272a; color:#e4e4e7; border:1px solid #3f3f46; border-radius:6px;
  padding:6px 10px; font-size:13px; cursor:pointer; }
.ct-btn:hover { background:#3f3f46; }
.ct-btn.sel { background:#2563eb; border-color:#3b82f6; color:#fff; }
.ct-degrees { display:flex; gap:6px; }
.ct-degree { flex:1; display:flex; flex-direction:column; align-items:center; gap:2px;
  background:#27272a; border:1px solid #3f3f46; border-radius:8px; padding:10px 6px; cursor:pointer; }
.ct-degree:hover { background:#3f3f46; }
.ct-degree .num { font-size:10px; color:#a1a1aa; letter-spacing:.08em; }
.ct-degree .name { font-size:15px; font-weight:600; }
.ct-notes { font-family:ui-monospace, monospace; font-size:11px; color:#71717a; }
"#;

/// Roman numerals for the seven degrees, cased by the chord's own quality
/// so a minor ii reads `ii` and a diminished vii reads `vii°`.
fn numeral(index: usize, quality_is_minor: bool, diminished: bool) -> String {
    const UPPER: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
    const LOWER: [&str; 7] = ["i", "ii", "iii", "iv", "v", "vi", "vii"];
    let base = if quality_is_minor || diminished {
        LOWER[index]
    } else {
        UPPER[index]
    };
    if diminished {
        format!("{base}°")
    } else {
        base.to_string()
    }
}

/// Parse a tonic name into a major key, falling back to C so a bad
/// string can't blank the panel.
fn key_from(tonic: &str) -> Key {
    Key::major(
        MusicalNote::from_string(tonic)
            .unwrap_or_else(|| MusicalNote::from_string("C").expect("C parses")),
    )
}

#[component]
pub fn ChordToolPanel() -> Element {
    let mut tonic = use_signal(|| "C".to_string());
    let mut size = use_signal(ChordSize::default);
    let mut octave = use_signal(|| 4i32);
    let mut last = use_signal(Vec::<u8>::new);

    let key = key_from(&tonic());
    let chords = scale_chords(&key, size());

    rsx! {
        document::Style { {PANEL_CSS} }
        div { class: "ct-root",
            div { class: "ct-row",
                span { class: "ct-label", "Key" }
                for note in ["C", "D", "E", "F", "G", "A", "B"] {
                    button {
                        class: if tonic() == note { "ct-btn sel" } else { "ct-btn" },
                        onclick: move |_| tonic.set(note.to_string()),
                        "{note}"
                    }
                }
            }
            div { class: "ct-row",
                span { class: "ct-label", "Chord" }
                for s in [ChordSize::Triad, ChordSize::Seventh, ChordSize::Ninth, ChordSize::Eleventh, ChordSize::Thirteenth] {
                    button {
                        class: if size() == s { "ct-btn sel" } else { "ct-btn" },
                        onclick: move |_| size.set(s),
                        "{s.label()}"
                    }
                }
            }
            div { class: "ct-row",
                span { class: "ct-label", "Octave" }
                button { class: "ct-btn", onclick: move |_| octave -= 1, "-" }
                span { "{octave}" }
                button { class: "ct-btn", onclick: move |_| octave += 1, "+" }
            }
            div { class: "ct-degrees",
                for (i, chord) in chords.iter().enumerate() {
                    {
                        let degree = (i + 1) as u8;
                        let is_minor = format!("{:?}", chord.quality).contains("Minor");
                        let is_dim = format!("{:?}", chord.quality).contains("Diminished");
                        let label = numeral(i, is_minor, is_dim);
                        let notes = chord_notes(&key, degree, size(), octave());
                        // The handler rebuilds the Key from signals rather
                        // than capturing the outer one — it outlives this
                        // render, and the key can change under it.
                        let fire = move |_| {
                            let key = key_from(&tonic());
                            last.set(chord_notes(&key, degree, size(), octave()));
                        };
                        rsx! {
                            div { class: "ct-degree", onclick: fire,
                                span { class: "num", "{label}" }
                                span { class: "name", "{chord.normalized}" }
                                span { class: "ct-notes", "{notes.len()}" }
                            }
                        }
                    }
                }
            }
            div { class: "ct-notes", "last fired: {last():?}" }
        }
    }
}
