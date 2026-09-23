//! The collaboration control in the top bar: share this song, join
//! someone's, and — once in a session — who is here, and whether everyone
//! plays on their own or together.
//!
//! Also starts a session from the environment, for the two-instance demo
//! and tests: `FTS_COLLAB_HOST=1` shares the song once it is open and
//! writes the ticket to `FTS_COLLAB_TICKET` (default: the temp dir's
//! `fts-session-ticket`); `FTS_COLLAB_JOIN` joins — a ticket, or a path
//! to a file holding one (waiting for the file to appear).

use std::time::Duration;

use dioxus::prelude::*;

use crate::shell::{ACCENT, RULE, TEXT};
use crate::studio::StudioSession;

/// Where an env-started host leaves its ticket.
fn ticket_path() -> std::path::PathBuf {
    std::env::var_os("FTS_COLLAB_TICKET")
        .map_or_else(|| std::env::temp_dir().join("fts-session-ticket"), std::path::PathBuf::from)
}

fn display_name() -> String {
    std::env::var("FTS_COLLAB_NAME")
        .ok()
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "Someone".to_owned())
}

#[component]
pub fn CollabBar() -> Element {
    let session: StudioSession = use_context();
    let mut status = use_signal(crate::collab::status);
    let mut error = use_signal(|| None::<String>);
    let mut joining = use_signal(String::new);

    // The environment's session, once the song is up.
    let chart = session.chart_file.clone();
    use_hook(move || {
        let chart = chart.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            // The song's own record of its edits, from the first moment —
            // shared or not.
            if std::env::var_os("FTS_COLLAB_JOIN").is_none()
                && let Err(e) = crate::collab::open_local(chart.clone())
            {
                tracing::warn!(collab.error = %e, "collab: the song's history is not being kept");
            }
            if std::env::var_os("FTS_COLLAB_HOST").is_some() {
                match crate::collab::host(display_name(), chart) {
                    Ok(ticket) => {
                        let path = ticket_path();
                        if let Err(e) = std::fs::write(&path, &ticket) {
                            tracing::warn!(collab.ticket_error = %e, "collab: the ticket was not written");
                        }
                        tracing::info!(collab.ticket_file = %path.display(), "collab: hosting");
                    }
                    Err(e) => tracing::warn!(collab.error = %e, "collab: could not host"),
                }
            } else if let Some(join) = std::env::var_os("FTS_COLLAB_JOIN") {
                let join = join.to_string_lossy().into_owned();
                let ticket = if join.starts_with("fts-session:") {
                    Some(join)
                } else {
                    // A file the host writes: wait for it.
                    (0..120).find_map(|_| {
                        let t = std::fs::read_to_string(&join).ok().filter(|t| t.starts_with("fts-session:"));
                        if t.is_none() {
                            std::thread::sleep(Duration::from_millis(500));
                        }
                        t
                    })
                };
                match ticket.map(|t| crate::collab::join(t.trim(), display_name())) {
                    Some(Ok(())) => tracing::info!("collab: joined"),
                    Some(Err(e)) => tracing::warn!(collab.error = %e, "collab: could not join"),
                    None => tracing::warn!("collab: no ticket to join"),
                }
            }
        });
    });

    // The status, kept current (peers come and go with nothing clicked).
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_millis(500)).await;
            let now = crate::collab::status();
            if *status.peek() != now {
                status.set(now);
            }
            let why = crate::collab::last_error();
            if *error.peek() != why {
                error.set(why);
            }
        }
    });

    let button = |active: bool| {
        let (bg, fg, border) = if active { (ACCENT, "#0b0c0e", ACCENT) } else { ("transparent", TEXT, RULE) };
        format!(
            "height:22px; padding:0 9px; border-radius:5px; border:1px solid {border}; \
             background:{bg}; color:{fg}; font-size:11px; font-weight:600; cursor:pointer;"
        )
    };
    let chart = session.chart_file.clone();
    let body = match status() {
        None => rsx! {
            button {
                style: button(false),
                title: "Share this song: others can join and edit it with you",
                onclick: move |_| crate::collab::host_in_background(display_name(), chart.clone()),
                "Share"
            }
            input {
                style: "height:20px; width:120px; padding:0 6px; border-radius:5px; border:1px solid {RULE}; \
                        background:#15171b; color:{TEXT}; font-size:11px;",
                placeholder: "paste a ticket",
                value: "{joining}",
                oninput: move |e| joining.set(e.value()),
                onmousedown: move |e| { crate::keys::set_editing(true); e.stop_propagation(); },
            }
            button {
                style: button(false),
                onclick: move |_| crate::collab::join_in_background(joining.peek().trim().to_owned(), display_name()),
                "Join"
            }
        },
        Some(live) => {
            let who = match live.peers {
                0 => "just you".to_owned(),
                1 => "1 other here".to_owned(),
                n => format!("{n} others here"),
            };
            let label = if live.hosting { "Sharing" } else { "Joined" };
            let shared = live.shared_transport;
            let ticket = live.ticket.clone();
            rsx! {
                span {
                    style: "font-size:11px; color:{TEXT}; white-space:nowrap;",
                    title: "{ticket}",
                    span { style: "color:#4ac26b;", "● " }
                    "{label} · {who}"
                }
                button {
                    style: button(shared),
                    title: if shared { "Everyone plays together — click to play on your own" } else { "Everyone plays on their own — click to play together" },
                    onclick: move |_| { crate::collab::set_shared_transport(!shared); status.set(crate::collab::status()); },
                    if shared { "Playing together" } else { "Playing apart" }
                }
                button {
                    style: button(false),
                    onclick: move |_| { crate::collab::leave(); status.set(None); },
                    "Leave"
                }
            }
        }
    };
    rsx! {
        div {
            style: "display:flex; align-items:center; gap:6px; margin-left:10px;",
            {body}
            if let Some(why) = error() {
                span { style: "font-size:11px; color:#e3b341; max-width:220px; overflow:hidden; \
                               text-overflow:ellipsis; white-space:nowrap;", title: "{why}", "{why}" }
            }
        }
    }
}
