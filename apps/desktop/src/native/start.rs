//! The start screen: what the window shows when nothing is open yet — on a
//! phone, which has no Open dialog and no song picked before launch; on a
//! desktop, when nothing was chosen or it did not open.
//!
//! Three ways in:
//!
//! - **Join a set** — one Task keeps, by its live link: the public demo, or
//!   one someone shared. Its songs stream in and play here, with everyone
//!   else in the set (`session_daw::stream_set`).
//! - **Task** — sign in from another device (`session_daw::task_account`)
//!   and pick a setlist from an org's library; it streams in the same way.
//! - **On this device** — a song or setlist in the app's documents folder
//!   (on a phone, what the Files app shows under Session), or, on a
//!   desktop, from the Open dialog.
//!
//! Opening runs on a thread of its own (it takes seconds), and the set it
//! opened becomes the window's ([`App`]).

use std::path::PathBuf;

use dioxus::prelude::*;
use lucide_dioxus::{
    ChevronRight, CircleAlert, FileMusic, FolderOpen, LibraryBig, Link, ListMusic, Radio,
};
use session_daw::loading::{Loading, Progress, mark_src};
use session_daw::setlist::Setlist;
use session_daw::stream_set::{LibrarySetlist, Remote};
use session_daw::task_account::{self, Account, Started};

const BG: &str = "#0f1012";
const BAR: &str = "#17181b";
const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";
const DIM: &str = "#8b9099";
const ACCENT: &str = "#3aa0ff";
const WARN: &str = "#e3b341";
/// Room at the top: on macOS the window's traffic lights sit over the
/// page (its title bar is transparent, as the shell's).
const TOP: u32 = if cfg!(target_os = "macos") { 44 } else { 28 };

/// The public demo's live link (`SESSION_DEMO_LINK` at build time, as the
/// site's /demo page takes it).
const DEMO_LINK: &str = match option_env!("SESSION_DEMO_LINK") {
    Some(link) => link,
    None => {
        "https://task.fasttrackstudio.app/org/days-to-praise/share/1e75d7fa11c540ac8dbbd461048e59fc"
    }
};

/// The window: the set once one is open, the start screen until then.
#[component]
pub fn App() -> Element {
    // iOS: winit's window is handed to the app's scene (see `ios_scene`) —
    // here, at the root, whichever the window opens on.
    #[cfg(target_os = "ios")]
    {
        let window = dioxus_native::use_window();
        use_hook(move || {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::UiKit(uikit) = handle.as_raw()
            {
                super::ios_scene::window_created(uikit.ui_view);
            }
        });
    }
    let initial: Option<Setlist> = use_context();
    let opened = use_signal(|| initial);
    let page = match opened() {
        Some(setlist) => rsx! {
            WithSetlist { setlist, super::shell::Shell {} }
        },
        None => rsx! { Start { opened } },
    };
    rsx! {
        document::Style { {ROOT_CSS} }
        {page}
    }
}

/// The page itself, edge to edge. Blitz's default stylesheet gives `body`
/// an 8px margin, and Blitz places an absolutely positioned box against its
/// parent — the body — so without this every view sat 8px right and down
/// and ran off the right edge. And the canvas takes the page's background:
/// the app's own dark, under a phone's status bar and home indicator too,
/// rather than the renderer's clear colour.
const ROOT_CSS: &str = "html, body { margin: 0; padding: 0; background: #0f1012; }";

/// What the launch chose to open as the window opens (see
/// `super::choose`), if anything.
#[derive(Clone)]
pub struct OpenFirst(pub Option<PathBuf>);

/// The set, as the shell reads it.
#[component]
fn WithSetlist(setlist: Setlist, children: Element) -> Element {
    use_context_provider(|| setlist);
    // `FTS_SESSION_AUTOPLAY=1` starts playing as the set opens — for
    // measuring a session while it plays without a hand on the mouse.
    use_hook(|| {
        if std::env::var("FTS_SESSION_AUTOPLAY").is_ok_and(|v| v != "0") {
            session_daw::engine::transport(session_daw::engine::Move::PlayStop, 0.0);
        }
    });
    children
}

/// Where opening has got to.
#[derive(Clone, PartialEq)]
enum Opening {
    Idle,
    /// Opening: the loading screen shows where it has got to.
    Busy(Progress),
    Failed(String),
}

/// Where signing in to Task has got to.
#[derive(Clone, PartialEq)]
enum SignIn {
    Out,
    Starting,
    /// Waiting for the code to be approved.
    Code(Started),
    In(Account),
    Failed(String),
}

/// Something loaded in the background.
#[derive(Clone, PartialEq)]
enum Load<T> {
    Waiting,
    Ready(T),
    Failed(String),
}

/// `work` on a thread of its own; its answer, awaited.
async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (done, answer) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    answer.await.ok()
}

/// A way of opening a set, run on a thread of its own: it says each step
/// to its `progress` — the loading screen's.
type OpenWork = Box<dyn FnOnce(&dyn Fn(Progress)) -> eyre::Result<Setlist> + Send>;

/// An error for a person: the whole chain, in a few words.
fn brief(e: &eyre::Report) -> String {
    session_daw::task_set::brief(&format!("{e:#}"))
}

#[component]
fn Start(opened: Signal<Option<Setlist>>) -> Element {
    let mut opening = use_signal(|| Opening::Idle);
    let local = use_hook(documents);

    // Open a set on a thread of its own; it becomes the window's when it
    // is ready. Meanwhile the loading screen says where it has got to.
    let mut open_set = move |first: Progress, work: OpenWork| {
        opening.set(Opening::Busy(first));
        let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<Progress>();
        spawn(async move {
            while let Some(step) = heard.recv().await {
                if matches!(*opening.peek(), Opening::Busy(_)) {
                    opening.set(Opening::Busy(step));
                }
            }
        });
        spawn(async move {
            let outcome = off_thread(move || work(&|step| drop(tell.send(step)))).await;
            match outcome {
                Some(Ok(setlist)) => opened.set(Some(setlist)),
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "start: the set did not open");
                    opening.set(Opening::Failed(brief(&e)));
                }
                None => opening.set(Opening::Failed("opening stopped".to_owned())),
            }
        });
    };
    let mut open_path = move |path: PathBuf| {
        let title = path.file_stem().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        open_set(
            Progress::Opening { title },
            Box::new(move |_| {
                let songs = super::songs_of(&path)
                    .ok_or_else(|| eyre::eyre!("{} is not a song or a setlist", path.display()))?;
                let setlist = Setlist::open(&songs)?;
                super::remember(&path);
                Ok(setlist)
            }),
        );
    };

    // What the launch chose, opened as the window opens.
    let OpenFirst(first) = use_context();
    use_hook(move || {
        if let Some(path) = first {
            open_path(path);
        }
    });

    // Task: signed in already, or not yet.
    let mut sign_in = use_signal(|| task_account::signed_in().map_or(SignIn::Out, SignIn::In));
    let mut orgs = use_signal(|| Load::<Vec<String>>::Waiting);
    let mut org = use_signal(|| None::<String>);
    let mut setlists = use_signal(|| Load::<Vec<LibrarySetlist>>::Waiting);
    // The signed-in person's orgs, and the chosen org's setlists, as they
    // change.
    let account = match sign_in() {
        SignIn::In(account) => Some(account),
        _ => None,
    };
    let for_orgs = account.clone();
    use_effect(use_reactive!(|for_orgs| {
        let Some(account) = for_orgs else { return };
        orgs.set(Load::Waiting);
        spawn(async move {
            match off_thread(move || task_account::orgs(&account)).await {
                Some(Ok(list)) => {
                    if org.peek().is_none() {
                        org.set(list.first().cloned());
                    }
                    orgs.set(Load::Ready(list));
                }
                Some(Err(e)) => orgs.set(Load::Failed(brief(&e))),
                None => {}
            }
        });
    }));
    let for_setlists = account.clone().zip(org());
    use_effect(use_reactive!(|for_setlists| {
        let Some((account, org)) = for_setlists else {
            return;
        };
        setlists.set(Load::Waiting);
        spawn(async move {
            let library = account.library(&org);
            match off_thread(move || session_daw::stream_set::setlists(&library)).await {
                Some(Ok(list)) => setlists.set(Load::Ready(list)),
                Some(Err(e)) => setlists.set(Load::Failed(brief(&e))),
                None => {}
            }
        });
    }));
    let start_sign_in = move |()| {
        sign_in.set(SignIn::Starting);
        spawn(async move {
            let server = task_account::server();
            let at = server.clone();
            let started = match off_thread(move || task_account::start(&at)).await {
                Some(Ok(started)) => started,
                Some(Err(e)) => {
                    sign_in.set(SignIn::Failed(brief(&e)));
                    return;
                }
                None => return,
            };
            super::open_url(&started.link());
            sign_in.set(SignIn::Code(started.clone()));
            match off_thread(move || task_account::finish(&server, &started)).await {
                Some(Ok(account)) => sign_in.set(SignIn::In(account)),
                Some(Err(e)) => sign_in.set(SignIn::Failed(brief(&e))),
                None => {}
            }
        });
    };

    let mut link = use_signal(String::new);
    let who = account.as_ref().map_or_else(guest_name, Account::name);
    let join = {
        let who = who.clone();
        move |text: String| {
            let Some(live) = live_link(&text) else {
                opening.set(Opening::Failed(
                    "that is not a Session or Task live link".to_owned(),
                ));
                return;
            };
            let name = who.clone();
            open_set(
                Progress::Joining { retry: None },
                Box::new(move |progress| {
                    session_daw::stream_set::open(Remote::Live { link: live, name }, progress)
                }),
            );
        }
    };
    // `FTS_SESSION_LIVE=<link>` joins a live set as the window opens, as
    // `FTS_SESSION_PROJECT` opens a song (`demo` is the public demo).
    use_hook({
        let mut join = join.clone();
        move || {
            if let Some(link) = std::env::var("FTS_SESSION_LIVE")
                .ok()
                .filter(|l| !l.trim().is_empty())
            {
                join(if link.trim() == "demo" {
                    DEMO_LINK.to_owned()
                } else {
                    link
                });
            }
        }
    });
    let mut join_demo = join.clone();
    let mut join_link = join.clone();
    let mut join_pasted = join;

    // Opening: the loading screen, whole.
    if let Opening::Busy(progress) = opening() {
        return rsx! { Loading { progress } };
    }

    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; box-sizing:border-box; \
                    overflow-y:auto; background:{BG}; color:{TEXT}; font-family:system-ui, sans-serif; \
                    display:flex; flex-direction:column; align-items:center;",
            div {
                style: "width:100%; max-width:560px; box-sizing:border-box; padding:{TOP}px 16px 48px; \
                        display:flex; flex-direction:column; gap:26px;",
                // The mark and the name.
                div {
                    style: "display:flex; align-items:center; gap:14px; padding:4px 2px 0;",
                    img { src: mark_src(), width: "52", height: "52", style: "border-radius:12px; flex:none;" }
                    div {
                        style: "display:flex; flex-direction:column; gap:2px; min-width:0;",
                        span { style: "font-size:28px; font-weight:750; letter-spacing:-0.02em;", "Session" }
                        span { style: "font-size:14px; color:{DIM};", "Your setlist, live." }
                    }
                }
                if let Opening::Failed(why) = opening() {
                    div {
                        style: "display:flex; gap:10px; align-items:flex-start; padding:12px 14px; border-radius:12px; \
                                background:#2a1f12; border:1px solid #5b4219; color:{WARN}; font-size:13px; line-height:1.45;",
                        CircleAlert { size: 18, color: "currentColor" }
                        span { style: "flex:1; min-width:0;", "Could not open it — {why}" }
                    }
                }
                // Live: the demo first, the way most people arrive.
                div {
                    style: "display:flex; flex-direction:column; gap:10px;",
                    Heading { label: "Live now" }
                    button {
                        style: "display:flex; align-items:center; gap:14px; width:100%; box-sizing:border-box; padding:18px 16px; \
                                border-radius:16px; border:1px solid #2c4a6b; cursor:pointer; text-align:left; \
                                background:linear-gradient(135deg, #16283d 0%, #121a24 60%, #111316 100%); \
                                color:{TEXT}; font-family:inherit;",
                        onclick: move |_| join_demo(DEMO_LINK.to_owned()),
                        div {
                            style: "flex:none; width:44px; height:44px; border-radius:22px; display:flex; \
                                    align-items:center; justify-content:center; background:{ACCENT}; color:#0b0c0e;",
                            Radio { size: 22, color: "currentColor" }
                        }
                        div {
                            style: "flex:1; min-width:0; display:flex; flex-direction:column; gap:3px;",
                            div {
                                style: "display:flex; align-items:center; gap:8px;",
                                span { style: "font-size:17px; font-weight:650;", "The Session demo" }
                                span {
                                    style: "font-size:10px; font-weight:750; letter-spacing:0.08em; padding:2px 6px; \
                                            border-radius:5px; background:#3aa0ff26; color:{ACCENT};",
                                    "LIVE"
                                }
                            }
                            span { style: "font-size:13px; color:#aab4c0; line-height:1.4;", "Play along with everyone in the demo set — chart, lyrics and click." }
                        }
                        ChevronRight { size: 20, color: "#6b7a8c" }
                    }
                    // A link someone shared.
                    div {
                        style: "display:flex; gap:8px; align-items:center; padding:8px; border-radius:14px; \
                                background:{BAR}; border:1px solid {RULE};",
                        div {
                            style: "flex:none; padding-left:6px; color:{DIM}; display:flex;",
                            Link { size: 18, color: "currentColor" }
                        }
                        // The hint under the field while it is empty: Blitz
                        // draws no `placeholder`.
                        div {
                            style: "position:relative; flex:1; min-width:0; height:38px;",
                            if link().is_empty() {
                                span {
                                    style: "position:absolute; top:0; left:6px; height:38px; display:flex; \
                                            align-items:center; font-size:15px; color:#6b7280; pointer-events:none;",
                                    "Have a link? Paste it here"
                                }
                            }
                            input {
                                style: "position:absolute; top:0; left:0; width:100%; height:38px; box-sizing:border-box; \
                                        padding:0 6px; border:none; background:transparent; color:{TEXT}; \
                                        font-family:inherit; font-size:15px;",
                                r#type: "text",
                                value: "{link}",
                                oninput: move |e| link.set(e.value()),
                            }
                        }
                        if cfg!(target_os = "ios") && link().trim().is_empty() {
                            Pill {
                                label: "Paste",
                                primary: false,
                                on_press: move |()| {
                                    if let Some(text) = super::pasted() {
                                        link.set(text.clone());
                                        join_pasted(text);
                                    }
                                },
                            }
                        } else {
                            Pill {
                                label: "Join",
                                primary: !link().trim().is_empty(),
                                on_press: move |()| {
                                    if !link().trim().is_empty() {
                                        join_link(link());
                                    }
                                },
                            }
                        }
                    }
                }
                // Task: the library.
                div {
                    style: "display:flex; flex-direction:column; gap:10px;",
                    Heading { label: "Your library" }
                    match sign_in() {
                        SignIn::Out | SignIn::Failed(_) => rsx! {
                            Card {
                                div {
                                    style: "display:flex; gap:14px; align-items:flex-start;",
                                    div {
                                        style: "flex:none; width:40px; height:40px; border-radius:10px; display:flex; \
                                                align-items:center; justify-content:center; background:#1f232a; color:{ACCENT};",
                                        LibraryBig { size: 20, color: "currentColor" }
                                    }
                                    div {
                                        style: "flex:1; min-width:0; display:flex; flex-direction:column; gap:4px;",
                                        span { style: "font-size:16px; font-weight:650;", "Your setlists, from Task" }
                                        span { style: "font-size:13px; color:{DIM}; line-height:1.45;", "Sign in with your FastTrackStudio account to open your org's sets and stream them here." }
                                        if let SignIn::Failed(why) = sign_in() {
                                            span { style: "font-size:12px; color:{WARN}; line-height:1.4;", "Not signed in — {why}" }
                                        }
                                    }
                                }
                                div {
                                    style: "display:flex; justify-content:flex-end; margin-top:14px;",
                                    Pill { label: "Sign in", primary: true, on_press: start_sign_in }
                                }
                            }
                        },
                        SignIn::Starting => rsx! {
                            Card { span { style: "font-size:14px; color:{DIM};", "Asking Task for a code…" } }
                        },
                        SignIn::Code(started) => rsx! {
                            Card {
                                div {
                                    style: "display:flex; flex-direction:column; align-items:center; gap:12px; padding:6px 0;",
                                    span { style: "font-size:14px; color:{DIM};", "Approve this code in your browser" }
                                    CodeBoxes { code: started.code.user_code.clone() }
                                    span { style: "font-size:12px; color:#6b7280;", "Waiting for approval…" }
                                    Pill {
                                        label: "Open the page again",
                                        primary: false,
                                        on_press: move |()| super::open_url(&started.link()),
                                    }
                                }
                            }
                        },
                        SignIn::In(account) => rsx! {
                            TaskLibrary {
                                account: account.clone(),
                                orgs: orgs(),
                                org,
                                setlists: setlists(),
                                on_open: move |setlist: LibrarySetlist| {
                                    let Some(org) = org() else { return };
                                    let library = account.library(&org);
                                    let name = account.name();
                                    let first = setlist.songs.first().map(|s| s.title.clone()).unwrap_or_default();
                                    open_set(
                                        Progress::Fetching { title: first, retry: None },
                                        Box::new(move |progress| {
                                            session_daw::stream_set::open(
                                                Remote::Library { library, setlist, name },
                                                progress,
                                            )
                                        }),
                                    );
                                },
                                on_sign_out: move |()| {
                                    task_account::sign_out();
                                    sign_in.set(SignIn::Out);
                                },
                            }
                        },
                    }
                }
                // Songs and sets on this device.
                div {
                    style: "display:flex; flex-direction:column; gap:10px;",
                    Heading { label: LOCAL_TITLE }
                    div {
                        style: "display:flex; flex-direction:column; border-radius:14px; overflow:hidden; \
                                background:{BAR}; border:1px solid {RULE};",
                        if local.is_empty() {
                            div {
                                style: "display:flex; gap:12px; align-items:center; padding:16px; font-size:13px; \
                                        line-height:1.45; color:{DIM};",
                                FolderOpen { size: 20, color: "currentColor" }
                                span { style: "flex:1; min-width:0;", "{LOCAL_EMPTY}" }
                            }
                        }
                        for (i, path) in local.clone().into_iter().enumerate() {
                            ListRow {
                                key: "{path.display()}",
                                first: i == 0,
                                title: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                detail: kind_of(&path),
                                on_press: move |()| open_path(path.clone()),
                            }
                        }
                        if cfg!(not(target_os = "ios")) {
                            ListRow {
                                first: local.is_empty(),
                                title: "Open a song or setlist…",
                                detail: "A REAPER project, a .session, or a .setlist",
                                on_press: move |()| {
                                    if let Some(path) = super::pick() {
                                        open_path(path);
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The signed-in person's library: which org, its setlists, signing out.
#[component]
fn TaskLibrary(
    account: Account,
    orgs: Load<Vec<String>>,
    org: Signal<Option<String>>,
    setlists: Load<Vec<LibrarySetlist>>,
    on_open: EventHandler<LibrarySetlist>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let who = account
        .session
        .email
        .clone()
        .unwrap_or_else(|| account.name());
    rsx! {
        if let Load::Ready(list) = &orgs {
            if list.len() > 1 {
                div {
                    style: "display:flex; flex-wrap:wrap; gap:6px;",
                    for slug in list.clone() {
                        Chip {
                            key: "{slug}",
                            label: slug.clone(),
                            on: org().as_deref() == Some(slug.as_str()),
                            on_press: move |()| org.set(Some(slug.clone())),
                        }
                    }
                }
            }
        }
        match (&orgs, &setlists) {
            (Load::Failed(why), _) | (_, Load::Failed(why)) => rsx! {
                Card { span { style: "font-size:13px; color:{WARN}; line-height:1.45;", "Could not read the library — {why}" } }
            },
            (Load::Waiting, _) | (_, Load::Waiting) => rsx! {
                Card { span { style: "font-size:14px; color:{DIM};", "Finding your setlists…" } }
            },
            (_, Load::Ready(list)) if list.is_empty() => rsx! {
                Card { span { style: "font-size:14px; color:{DIM};", "No setlists in this library yet." } }
            },
            (_, Load::Ready(list)) => rsx! {
                for setlist in list.clone() {
                    SetlistCard {
                        key: "{setlist.id}",
                        setlist: setlist.clone(),
                        on_press: move |()| on_open.call(setlist.clone()),
                    }
                }
            },
        }
        div {
            style: "display:flex; align-items:center; gap:8px; padding:0 4px; font-size:12px; color:#6b7280;",
            span { style: "flex:1; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", "Signed in as {who}" }
            button {
                style: "border:none; background:transparent; color:{DIM}; font-family:inherit; font-size:12px; \
                        text-decoration:underline; cursor:pointer; padding:4px;",
                onclick: move |_| on_sign_out.call(()),
                "Sign out"
            }
        }
    }
}

/// A setlist as a card: its name, how many songs, and the first few.
#[component]
fn SetlistCard(setlist: LibrarySetlist, on_press: EventHandler<()>) -> Element {
    let count = setlist.songs.len();
    let preview = songs_line(&setlist);
    rsx! {
        button {
            style: "display:flex; align-items:center; gap:14px; width:100%; box-sizing:border-box; padding:14px 16px; border-radius:14px; \
                    background:{BAR}; border:1px solid {RULE}; color:{TEXT}; font-family:inherit; \
                    text-align:left; cursor:pointer;",
            onclick: move |_| on_press.call(()),
            div {
                style: "flex:none; width:40px; height:40px; border-radius:10px; display:flex; align-items:center; \
                        justify-content:center; background:#1f232a; color:{ACCENT};",
                ListMusic { size: 20, color: "currentColor" }
            }
            div {
                style: "flex:1; min-width:0; display:flex; flex-direction:column; gap:3px;",
                div {
                    style: "display:flex; align-items:center; gap:8px;",
                    span { style: "font-size:16px; font-weight:650; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", "{setlist.title}" }
                    span { style: "flex:none; font-size:11px; color:{DIM};", "{count} songs" }
                }
                span { style: "font-size:12px; color:#6b7280; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", "{preview}" }
            }
            ChevronRight { size: 18, color: "#6b7280" }
        }
    }
}

/// A setlist's songs, in a line.
fn songs_line(setlist: &LibrarySetlist) -> String {
    let titles: Vec<&str> = setlist.songs.iter().map(|s| s.title.as_str()).collect();
    if titles.is_empty() {
        "No songs".to_owned()
    } else {
        titles.join(" · ")
    }
}

/// The live link in `text`: a Task live share link as it is, the one a
/// Session app link carries (`…/app/?live=<link>`), or — for the site's
/// demo page (`…/demo`) — the demo's. Typed as people type them: with or
/// without `https://`.
fn live_link(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches('/');
    let bare = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    // A host and a path, at least: `name.tld/…`.
    let (host, path) = bare.split_once('/')?;
    if !host.contains('.') || host.contains(' ') {
        return None;
    }
    let url = if bare.len() == text.len() {
        format!("https://{text}")
    } else {
        text.to_owned()
    };
    if let Some(carried) = url
        .split(['?', '&'])
        .find_map(|pair| pair.strip_prefix("live="))
    {
        return Some(percent_decode(carried));
    }
    if path.split(['?', '#']).next() == Some("demo") {
        return Some(DEMO_LINK.to_owned());
    }
    Some(url)
}

/// `%XX` escapes undone (a link riding in another's query).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&h), Some(&l)) if hex(h).is_some() && hex(l).is_some() => {
                out.push(u8::try_from(hex(h).unwrap_or(0) * 16 + hex(l).unwrap_or(0)).unwrap_or(0));
                i += 3;
            }
            (b'+', ..) => {
                out.push(b' ');
                i += 1;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A visitor's name in a set, when not signed in: Guest and four letters,
/// so two guests are told apart (as a page names one).
fn guest_name() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())
        % 65_536;
    format!("Guest {n:04x}")
}

#[cfg(target_os = "ios")]
const LOCAL_TITLE: &str = "On this iPhone";
#[cfg(not(target_os = "ios"))]
const LOCAL_TITLE: &str = "On this computer";

#[cfg(target_os = "ios")]
const LOCAL_EMPTY: &str = "Nothing here yet. Put a song folder or a setlist in Files → On My iPhone → Session, and it shows here.";
#[cfg(not(target_os = "ios"))]
const LOCAL_EMPTY: &str = "No songs in Documents/Session yet.";

/// The songs and setlists in the app's documents folder: each entry that is
/// a song folder, a setlist, or a project — newest first.
fn documents() -> Vec<PathBuf> {
    let Some(dir) = documents_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            p.is_dir() || matches!(ext.as_str(), "setlist" | "rpp")
        })
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .map(|p| {
            let at = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (at, p)
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Where the app keeps its songs: on a phone, its own Documents (what the
/// Files app shows as On My iPhone → Session); on a desktop, a Session
/// folder in the user's Documents.
fn documents_dir() -> Option<PathBuf> {
    let docs = dirs::document_dir()?;
    if cfg!(target_os = "ios") {
        Some(docs)
    } else {
        Some(docs.join("Session"))
    }
}

/// What an entry is, in a word or two.
fn kind_of(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "setlist" => "Setlist".to_owned(),
        "rpp" => "REAPER project".to_owned(),
        "session" => "Song".to_owned(),
        _ if session_daw::setlist::song_in(path).is_some() => "Song".to_owned(),
        _ => "Setlist".to_owned(),
    }
}

/// A section's heading.
#[component]
fn Heading(label: String) -> Element {
    rsx! {
        span {
            style: "font-size:12px; font-weight:700; letter-spacing:0.08em; color:{DIM}; padding-left:4px;",
            "{label.to_uppercase()}"
        }
    }
}

/// A panel holding one thing.
#[component]
fn Card(children: Element) -> Element {
    rsx! {
        div {
            style: "padding:16px; border-radius:14px; background:{BAR}; border:1px solid {RULE};",
            {children}
        }
    }
}

/// A row of a list: a file, or the Open dialog.
#[component]
fn ListRow(first: bool, title: String, detail: String, on_press: EventHandler<()>) -> Element {
    let rule = if first {
        "none".to_owned()
    } else {
        format!("1px solid {RULE}")
    };
    rsx! {
        button {
            style: "display:flex; align-items:center; gap:12px; width:100%; box-sizing:border-box; min-height:56px; padding:10px 14px; \
                    border:none; border-top:{rule}; background:transparent; color:{TEXT}; \
                    font-family:inherit; text-align:left; cursor:pointer;",
            onclick: move |_| on_press.call(()),
            div {
                style: "flex:none; color:{DIM}; display:flex;",
                FileMusic { size: 20, color: "currentColor" }
            }
            div {
                style: "flex:1; min-width:0; display:flex; flex-direction:column; gap:2px;",
                span { style: "font-size:15px; font-weight:600; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", "{title}" }
                span { style: "font-size:12px; color:{DIM};", "{detail}" }
            }
            ChevronRight { size: 18, color: "#6b7280" }
        }
    }
}

/// A rounded button: the primary one filled.
#[component]
fn Pill(label: String, primary: bool, on_press: EventHandler<()>) -> Element {
    let (fg, bg, border) = if primary {
        ("#0b0c0e", ACCENT, ACCENT)
    } else {
        (TEXT, "#23262c", RULE)
    };
    rsx! {
        button {
            style: "flex:none; height:38px; padding:0 16px; border-radius:19px; border:1px solid {border}; \
                    background:{bg}; color:{fg}; font-family:inherit; font-size:14px; font-weight:650; cursor:pointer;",
            onclick: move |_| on_press.call(()),
            "{label}"
        }
    }
}

#[component]
fn Chip(label: String, on: bool, on_press: EventHandler<()>) -> Element {
    let (fg, bg, border) = if on {
        (ACCENT, "#3aa0ff1f", "#3aa0ff66")
    } else {
        (DIM, "transparent", RULE)
    };
    rsx! {
        button {
            style: "height:32px; padding:0 14px; border-radius:16px; border:1px solid {border}; \
                    background:{bg}; color:{fg}; font-family:inherit; font-size:13px; cursor:pointer;",
            onclick: move |_| on_press.call(()),
            "{label}"
        }
    }
}

/// A sign-in code, one box per character, as the approval page shows it.
#[component]
fn CodeBoxes(code: String) -> Element {
    rsx! {
        div {
            style: "display:flex; gap:6px; justify-content:center;",
            for (i, c) in code.chars().enumerate() {
                span {
                    key: "{i}",
                    style: "width:30px; height:40px; border-radius:8px; display:flex; align-items:center; \
                            justify-content:center; background:#0f1012; border:1px solid {RULE}; \
                            font-size:20px; font-weight:700; font-family:ui-monospace, monospace;",
                    "{c}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::live_link;

    #[test]
    fn a_live_link_is_taken_as_it_is_or_out_of_the_app_link_carrying_it() {
        let task = "https://task.example/org/band/share/abc";
        assert_eq!(live_link(task).as_deref(), Some(task));
        assert_eq!(
            live_link(" https://session.example/app/?live=https%3A%2F%2Ftask.example%2Forg%2Fband%2Fshare%2Fabc&name=Me ")
                .as_deref(),
            Some(task)
        );
        assert_eq!(live_link("not a link"), None);
        assert_eq!(live_link("example.com"), None);
    }

    #[test]
    fn a_link_typed_without_its_scheme_or_the_demo_page_is_understood() {
        assert_eq!(
            live_link("task.example/org/band/share/abc").as_deref(),
            Some("https://task.example/org/band/share/abc")
        );
        for demo in [
            "session.fasttrackstudio.app/demo",
            "https://session.fasttrackstudio.app/demo/",
        ] {
            assert_eq!(live_link(demo).as_deref(), Some(super::DEMO_LINK), "{demo}");
        }
    }
}
