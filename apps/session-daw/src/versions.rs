//! The song's versions, from the top bar.
//!
//! Save it (Ctrl+S does too), or save a named checkpoint of it —
//! "Organized", before the mix — which leaves the song open as it is
//! (`crate::projects`); and the checkpoints it has. Through the Action
//! List, as a key or a model would: `SESSION_SAVE` and
//! `SESSION_CHECKPOINT`.

use dioxus::prelude::*;

use crate::shell::{BAR_BG, BAR_H, DIM, RAISED, RULE, TEXT};

/// Good, or not.
const OK: &str = "#4ac26b";
const NO: &str = "#e5a442";

/// A message's colour: good, or not.
const fn tone(good: bool) -> &'static str {
    if good { OK } else { NO }
}

/// The Versions button and its panel.
#[component]
pub fn Versions() -> Element {
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut said = use_signal(|| None::<(bool, String)>);
    let mut listed = use_signal(Vec::<(String, String)>::new);
    let mut refresh = move || listed.set(crate::actions::open_song_alternatives());
    let mut run = move |id: &str, args: String| {
        let outcome = crate::actions::list().run_json(id, args.as_bytes());
        said.set(Some(match outcome {
            Ok(bytes) => (
                true,
                serde_json::from_slice::<String>(&bytes)
                    .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned()),
            ),
            Err(e) => (false, e.to_string()),
        }));
        refresh();
    };
    let mut checkpoint = move || {
        let wanted = name.peek().trim().to_owned();
        if wanted.is_empty() {
            said.set(Some((false, "a checkpoint needs a name".to_owned())));
            return;
        }
        run(
            "SESSION_CHECKPOINT",
            serde_json::json!({ "name": wanted }).to_string(),
        );
        name.set(String::new());
    };
    let (lit, ink) = if open() {
        (RAISED, TEXT)
    } else {
        ("transparent", DIM)
    };
    rsx! {
        div {
            style: "position:relative; flex:none; display:flex; align-items:stretch;",
            onmousedown: move |event| event.stop_propagation(),
            button {
                title: "Save the song, or a checkpoint of it (Ctrl+S saves)",
                style: "flex:none; width:44px; display:flex; align-items:center; justify-content:center; \
                        padding:0; border:none; background:{lit}; color:{ink}; cursor:pointer;",
                onclick: move |_| {
                    let now = !open();
                    open.set(now);
                    if now {
                        said.set(None);
                        refresh();
                    }
                },
                lucide_dioxus::History { size: 18, color: ink }
            }
            if open() {
                div {
                    style: "position:absolute; top:{BAR_H}px; right:0; width:320px; z-index:40; \
                            display:flex; flex-direction:column; gap:10px; padding:12px; \
                            background:{BAR_BG}; border:1px solid {RULE}; border-radius:0 0 10px 10px; \
                            color:{TEXT}; font-size:13px;",
                    button {
                        style: "height:34px; border-radius:8px; border:1px solid {RULE}; background:{RAISED}; \
                                color:{TEXT}; font-family:inherit; font-size:13px; font-weight:600; cursor:pointer;",
                        onclick: move |_| run("SESSION_SAVE", String::new()),
                        "Save the song  ·  Ctrl+S"
                    }
                    span { style: "color:{DIM}; font-size:12px; line-height:1.4;",
                        "A checkpoint saves the song as it is now, beside it, under a name — and keeps you in the song."
                    }
                    div {
                        style: "display:flex; gap:8px;",
                        input {
                            style: "flex:1; min-width:0; height:32px; padding:0 10px; border-radius:8px; \
                                    border:1px solid {RULE}; background:#0f1012; color:{TEXT}; \
                                    font-family:inherit; font-size:13px;",
                            placeholder: "Name — Organized, Mix A…",
                            value: "{name}",
                            oninput: move |event| name.set(event.value()),
                            onkeydown: move |event| {
                                if event.key() == Key::Enter {
                                    checkpoint();
                                }
                            },
                        }
                        button {
                            style: "flex:none; height:32px; padding:0 12px; border-radius:8px; border:none; \
                                    background:#3aa0ff; color:#0b0c0e; font-family:inherit; font-size:13px; \
                                    font-weight:650; cursor:pointer;",
                            onclick: move |_| checkpoint(),
                            "Checkpoint"
                        }
                    }
                    if let Some((good, text)) = said() {
                        span {
                            style: "font-size:12px; line-height:1.4; color:{tone(good)}; overflow-wrap:anywhere;",
                            "{text}"
                        }
                    }
                    if !listed().is_empty() {
                        div {
                            style: "display:flex; flex-direction:column; border-top:1px solid {RULE}; padding-top:8px; gap:4px;",
                            span { style: "color:{DIM}; font-size:11px; text-transform:uppercase; letter-spacing:0.06em;", "Checkpoints" }
                            for (alt, when) in listed() {
                                div {
                                    key: "{alt}",
                                    style: "display:flex; justify-content:space-between; gap:8px;",
                                    span { "{alt}" }
                                    span { style: "color:{DIM};", "{when}" }
                                }
                            }
                            span { style: "color:{DIM}; font-size:11px;", "Open one from the start screen's Recent." }
                        }
                    }
                }
            }
        }
    }
}
