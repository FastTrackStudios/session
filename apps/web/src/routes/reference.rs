//! `/reference` — a recording playing in step with the session.
//!
//! The browser half of [`session::reference`]. That module says where
//! the player should be and what it should be doing; this is the thing
//! that does it, and the only part that has to know YouTube exists.
//!
//! **Why the browser is the right home for this.** A YouTube video can
//! only be played by YouTube's own player, in an iframe, on a page —
//! their terms require it and their API assumes it. A native window
//! would have to embed a browser to do the same job, so the web build
//! is not a lesser surface here. It is the one that can.

use dioxus::prelude::*;
use session::reference::{Command, Playing, Reference, Source, follow};

/// The IFrame Player API, loaded once.
///
/// From YouTube's own origin because it must be: the API script and the
/// player it creates talk to each other across the frame boundary, and
/// a copy served from anywhere else is a different origin and a silent
/// no-op.
const API: &str = "https://www.youtube.com/iframe_api";

#[component]
pub fn ReferencePage() -> Element {
    let mut pasted = use_signal(String::new);
    let mut reference = use_signal(|| None::<Reference>);
    let mut refused = use_signal(|| false);

    rsx! {
        document::Script { src: API }
        div { class: "reference",
            h1 { "Play against a reference" }
            p { class: "lede",
                "Paste a video and line it up with the session. The bar \
                 lines follow what you hear, not the other way round."
            }

            div { class: "paste",
                input {
                    r#type: "text",
                    placeholder: "YouTube link, or just the id",
                    value: "{pasted}",
                    oninput: move |event| {
                        pasted.set(event.value());
                        refused.set(false);
                    },
                }
                button {
                    onclick: move |_| {
                        match Source::youtube(&pasted()) {
                            // The id is what is kept. Whatever else was
                            // on the end of the link identified a
                            // playlist, a start time or whoever shared
                            // it — none of it identifies the video.
                            Some(source) => {
                                refused.set(false);
                                reference.set(Some(Reference::new(source)));
                            }
                            None => refused.set(true),
                        }
                    },
                    "Use it"
                }
            }

            if refused() {
                p { class: "refused",
                    "That is not a video I can find an id in. A watch \
                     link, a short link, or the eleven characters on \
                     their own."
                }
            }

            if let Some(reference) = reference() {
                Player { reference }
            }
        }
    }
}

/// The player, and the loop that keeps it in step.
#[component]
fn Player(reference: Reference) -> Element {
    // The id is copied out rather than borrowed: the sync loop below
    // owns the reference for as long as the component lives, and a
    // borrow held across that would outlive the frame it was taken in.
    let Source::Youtube(id) = reference.source.clone() else {
        return rsx! {
            p { "Only YouTube is wired up so far." }
        };
    };

    // Where the session is. A stand-in until this page is wired to a
    // transport: the sync loop below is the part worth building, and it
    // does not care where the number comes from.
    let session_at = use_signal(|| 0.0_f64);
    let rolling = use_signal(|| false);

    // Hand the player to the API once the frame exists. The API calls
    // back a global when it has loaded, which may be before or after
    // this component mounts — so both orders have to work, which is
    // what the `if` in the script is for.
    use_effect(move || {
        let _ = dioxus::document::eval(SETUP);
    });

    // The loop. Ten times a second, not every frame: the player is
    // asked across a frame boundary and the tolerance is eighty
    // milliseconds, so a faster loop would ask more often than the
    // answer can change.
    use_future(move || {
        let reference = reference.clone();
        async move {
            loop {
                // The wasm-cfg-split seam: browser timers here, tokio
                // on native, so this loop is the same code either way.
                architect::platform::sleep(std::time::Duration::from_millis(100)).await;
                let Ok(state) = dioxus::document::eval(READ).await else {
                    continue;
                };
                let (at, playing) = match state.as_str() {
                    Some(text) => match text.split_once('|') {
                        Some((at, playing)) => (at.parse::<f64>().unwrap_or(0.0), playing == "1"),
                        None => continue,
                    },
                    None => continue,
                };
                let command = next_command(&reference, session_at(), rolling(), at, playing);
                let js = match command {
                    Command::Seek(to) => format!("window.ftsReference?.seekTo({to}, true);"),
                    Command::Play => "window.ftsReference?.playVideo();".to_owned(),
                    Command::Pause => "window.ftsReference?.pauseVideo();".to_owned(),
                    // The common case, and it must cost nothing: a
                    // player nudged every tick stutters.
                    Command::Nothing => continue,
                };
                let _ = dioxus::document::eval(&js);
            }
        }
    });

    rsx! {
        div { class: "player",
            iframe {
                id: "fts-reference",
                width: "640",
                height: "360",
                // `enablejsapi` is what makes the player answerable at
                // all; without it the frame plays and ignores everyone.
                src: "https://www.youtube-nocookie.com/embed/{id}?enablejsapi=1&rel=0",
                // nocookie above, and no autoplay: a reference that
                // started playing when the page loaded would be a
                // reference talking over whatever you were listening to.
                allow: "accelerometer; encrypted-media; picture-in-picture",
                title: "Reference recording",
            }
        }
    }
}

/// Hand the iframe to the IFrame API.
///
/// Written to work in either order: the API script may load before this
/// runs or after, and a page that only handled one of those would work
/// on a warm cache and not on a cold one — which is the worst kind of
/// intermittent.
const SETUP: &str = r"
    window.ftsMakeReference = function () {
        if (window.ftsReference) { return; }
        const frame = document.getElementById('fts-reference');
        if (!frame || !window.YT || !window.YT.Player) { return; }
        window.ftsReference = new window.YT.Player('fts-reference', {});
    };
    window.onYouTubeIframeAPIReady = window.ftsMakeReference;
    window.ftsMakeReference();
";

/// What the player says it is doing: `seconds|1` or `seconds|0`.
///
/// One string rather than two calls, because the two would be read a
/// frame apart and a position from before a state change is how a loop
/// decides to correct something that already corrected itself.
const READ: &str = r"
    (() => {
        const p = window.ftsReference;
        if (!p || !p.getCurrentTime) { return '0|0'; }
        const playing = p.getPlayerState && p.getPlayerState() === 1;
        return p.getCurrentTime() + '|' + (playing ? '1' : '0');
    })()
";

/// What to do about the player this frame, as a plain question.
///
/// Separated from the component so the decision can be tested without a
/// browser: everything above is an iframe and an input box, and
/// everything that could be wrong is here or in `session::reference`.
#[must_use]
pub fn next_command(
    reference: &Reference,
    session_at: f64,
    rolling: bool,
    player_at: f64,
    player_playing: bool,
) -> Command {
    follow(
        reference,
        session_at,
        rolling,
        Playing {
            at: player_at,
            playing: player_playing,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::next_command;
    use session::reference::{Command, Reference, Source};

    fn video() -> Reference {
        Reference::new(Source::Youtube("dQw4w9WgXcQ".into())).aligned(4.0, 30.0)
    }

    /// The page asks the same question the domain answers.
    ///
    /// Thin on purpose: the moment this function starts deciding
    /// anything itself, the decision is in a component and can only be
    /// tested by driving a browser.
    #[test]
    fn the_page_defers_to_the_domain() {
        let r = video();
        // Session at the anchor, player at the start: go to 30.
        assert_eq!(next_command(&r, 4.0, true, 0.0, false), Command::Seek(30.0));
        // In the right place and stopped: start it.
        assert_eq!(next_command(&r, 4.0, true, 30.0, false), Command::Play);
        // In step: leave it be.
        assert_eq!(next_command(&r, 4.0, true, 30.0, true), Command::Nothing);
        // Session stopped: stop first, whatever else is true.
        assert_eq!(next_command(&r, 4.0, false, 999.0, true), Command::Pause);
    }
}
