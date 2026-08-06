//! The chord panel.
//!
//! One component, two homes: `examples/panel.rs` runs it in a desktop
//! window for fast iteration, and `fts-extensions` registers it as a
//! REAPER panel rendered by Blitz. Keeping it a plain component with no
//! props is what allows that — context, not arguments (the same rule the
//! signal UI follows).
//!
//! There is no music theory in this file, and there shouldn't be. The
//! chords, their roles and their pitches come from
//! `keyflow::chord::palette`; this file decides only how to lay them out
//! and what colour they are.

use dioxus::prelude::*;
use keyflow::chord::palette::{ApproachKind, ChordCandidate, ChordRole, palette};
use keyflow::key::Key;
use keyflow::primitives::MusicalNote;

/// Blitz doesn't load external stylesheets reliably, so everything is
/// inline or embedded — the same constraint the signal UI documents.
///
/// The three role colours are the point of the layout: what a chord is
/// *for* should be legible before you read its name.
const PANEL_CSS: &str = r#"
.ct-root { font-family: system-ui, sans-serif; background:#18181b; color:#e4e4e7;
  padding:12px; display:flex; flex-direction:column; gap:14px; height:100%; overflow-y:auto; }
.ct-bar { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
.ct-label { font-size:11px; text-transform:uppercase; letter-spacing:.06em; color:#a1a1aa; }
.ct-select { background:#27272a; color:#e4e4e7; border:1px solid #3f3f46; border-radius:6px;
  padding:5px 8px; font-size:13px; }
.ct-step { background:#27272a; color:#e4e4e7; border:1px solid #3f3f46; border-radius:6px;
  padding:4px 9px; font-size:13px; cursor:pointer; }
.ct-step:hover { background:#3f3f46; }

.ct-group { display:flex; flex-direction:column; gap:5px; }
.ct-group-head { display:flex; align-items:baseline; gap:8px; }
.ct-group-title { font-size:11px; text-transform:uppercase; letter-spacing:.08em; font-weight:600; }
.ct-group-note { font-size:11px; color:#71717a; }
.ct-chords { display:flex; gap:6px; flex-wrap:wrap; }

.ct-chip { display:flex; flex-direction:column; align-items:center; gap:1px;
  border:1px solid; border-radius:8px; padding:7px 11px; cursor:pointer; min-width:56px; }
.ct-chip .name { font-size:14px; font-weight:600; }
.ct-chip .sub { font-size:9px; opacity:.75; letter-spacing:.04em; }

/* Diatonic — in the key. Calm, the furniture you build on. */
.ct-diatonic { background:#1e3a5f; border-color:#3b82f6; color:#dbeafe; }
.ct-diatonic:hover { background:#1e4d7f; }
/* Parallel key — borrowed colour. Warmer, a step outside. */
.ct-parallel { background:#4a2f1a; border-color:#d97706; color:#fed7aa; }
.ct-parallel:hover { background:#5f3d22; }
/* Approach — tension, pointing somewhere. Hottest of the three. */
.ct-approach { background:#4a1d2e; border-color:#e11d48; color:#fecdd3; }
.ct-approach:hover { background:#5f2439; }

.ct-fired { font-family:ui-monospace, monospace; font-size:11px; color:#71717a;
  border-top:1px solid #27272a; padding-top:8px; }
"#;

/// Every tonic, spelled the way charts spell them.
const TONICS: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

const NUMERALS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// Parse a tonic into a key of the chosen mode, falling back to C so a
/// bad string can't blank the panel.
fn build_key(tonic: &str, minor: bool) -> Key {
    let root = MusicalNote::from_string(tonic)
        .unwrap_or_else(|| MusicalNote::from_string("C").expect("C parses"));
    if minor { Key::minor(root) } else { Key::major(root) }
}

fn chip_class(role: ChordRole) -> &'static str {
    match role {
        ChordRole::Diatonic => "ct-chip ct-diatonic",
        ChordRole::ParallelKey => "ct-chip ct-parallel",
        ChordRole::Approach { .. } => "ct-chip ct-approach",
    }
}

/// The caption under a chord name. For approach chords this is the whole
/// point — the chord means nothing without saying what it targets.
fn caption(candidate: &ChordCandidate, index: usize) -> String {
    match candidate.role {
        ChordRole::Diatonic => NUMERALS.get(index).copied().unwrap_or("").to_string(),
        ChordRole::ParallelKey => "borrowed".to_string(),
        ChordRole::Approach { target_degree, kind } => {
            let route = match kind {
                ApproachKind::SecondaryDominant => "V7",
                ApproachKind::TritoneSub => "bII7",
            };
            let target = NUMERALS
                .get(usize::from(target_degree.saturating_sub(1)))
                .copied()
                .unwrap_or("");
            format!("{route}/{target}")
        }
    }
}

#[component]
fn ChordGroup(
    title: String,
    note: String,
    chords: Vec<ChordCandidate>,
    octave: i32,
    fired: Signal<Vec<u8>>,
    fired_label: Signal<String>,
) -> Element {
    rsx! {
        div { class: "ct-group",
            div { class: "ct-group-head",
                span { class: "ct-group-title", "{title}" }
                span { class: "ct-group-note", "{note}" }
            }
            div { class: "ct-chords",
                for (i, chord) in chords.iter().enumerate() {
                    {
                        let notes = chord.notes(octave);
                        let label = chord.label.clone();
                        let sub = caption(chord, i);
                        let class = chip_class(chord.role);
                        let mut fired = fired;
                        let mut fired_label = fired_label;
                        rsx! {
                            div {
                                class: "{class}",
                                onclick: move |_| {
                                    fired.set(notes.clone());
                                    fired_label.set(label.clone());
                                },
                                span { class: "name", "{chord.label}" }
                                span { class: "sub", "{sub}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn ChordToolPanel() -> Element {
    let mut tonic = use_signal(|| "C".to_string());
    let mut minor = use_signal(|| false);
    let mut octave = use_signal(|| 4i32);
    let fired = use_signal(Vec::<u8>::new);
    let fired_label = use_signal(String::new);

    let key = build_key(&tonic(), minor());
    let all = palette(&key);
    let pick = |f: fn(&ChordCandidate) -> bool| -> Vec<ChordCandidate> {
        all.iter().filter(|c| f(c)).cloned().collect()
    };
    let diatonic = pick(|c| c.role == ChordRole::Diatonic);
    let parallel = pick(|c| c.role == ChordRole::ParallelKey);
    let approach = pick(|c| matches!(c.role, ChordRole::Approach { .. }));

    rsx! {
        document::Style { {PANEL_CSS} }
        div { class: "ct-root",
            div { class: "ct-bar",
                span { class: "ct-label", "Key" }
                select {
                    class: "ct-select",
                    value: "{tonic}",
                    onchange: move |e| tonic.set(e.value()),
                    for t in TONICS {
                        option { value: "{t}", "{t}" }
                    }
                }
                select {
                    class: "ct-select",
                    value: if minor() { "minor" } else { "major" },
                    onchange: move |e| minor.set(e.value() == "minor"),
                    option { value: "major", "major" }
                    option { value: "minor", "minor" }
                }
                span { class: "ct-label", "Octave" }
                button { class: "ct-step", onclick: move |_| octave -= 1, "-" }
                span { "{octave}" }
                button { class: "ct-step", onclick: move |_| octave += 1, "+" }
            }

            ChordGroup {
                title: "Diatonic".to_string(),
                note: "in the key".to_string(),
                chords: diatonic,
                octave: octave(),
                fired,
                fired_label,
            }
            ChordGroup {
                title: "Parallel Key".to_string(),
                note: "borrowed from the parallel major/minor".to_string(),
                chords: parallel,
                octave: octave(),
                fired,
                fired_label,
            }
            ChordGroup {
                title: "Approach".to_string(),
                note: "dominants pointing at a degree — V7/x and its tritone sub".to_string(),
                chords: approach,
                octave: octave(),
                fired,
                fired_label,
            }

            div { class: "ct-fired",
                if fired().is_empty() {
                    "click a chord"
                } else {
                    "{fired_label()} → {fired():?}"
                }
            }
        }
    }
}
