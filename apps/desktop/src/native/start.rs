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
    Busy(String),
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
/// to its `progress`.
type OpenWork = Box<dyn FnOnce(&dyn Fn(String)) -> eyre::Result<Setlist> + Send>;

#[component]
fn Start(opened: Signal<Option<Setlist>>) -> Element {
    let mut opening = use_signal(|| Opening::Idle);
    let local = use_hook(documents);
    let busy = matches!(opening(), Opening::Busy(_));

    // Open a set on a thread of its own; it becomes the window's when it
    // is ready. What it is doing shows as it goes.
    let mut open_set = move |what: String, work: OpenWork| {
        opening.set(Opening::Busy(what));
        let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
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
                    opening.set(Opening::Failed(session_daw::task_set::brief(&format!(
                        "{e:#}"
                    ))));
                }
                None => opening.set(Opening::Failed("opening stopped".to_owned())),
            }
        });
    };
    let mut open_path = move |path: PathBuf| {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        open_set(
            format!("Opening {name}…"),
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
                Some(Err(e)) => orgs.set(Load::Failed(session_daw::task_set::brief(&format!(
                    "{e:#}"
                )))),
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
                Some(Err(e)) => setlists.set(Load::Failed(session_daw::task_set::brief(&format!(
                    "{e:#}"
                )))),
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
                    sign_in.set(SignIn::Failed(session_daw::task_set::brief(&format!(
                        "{e:#}"
                    ))));
                    return;
                }
                None => return,
            };
            super::open_url(&started.link());
            sign_in.set(SignIn::Code(started.clone()));
            match off_thread(move || task_account::finish(&server, &started)).await {
                Some(Ok(account)) => sign_in.set(SignIn::In(account)),
                Some(Err(e)) => sign_in.set(SignIn::Failed(session_daw::task_set::brief(
                    &format!("{e:#}"),
                ))),
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
                "Joining the set…".to_owned(),
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

    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; box-sizing:border-box; \
                    overflow-y:auto; background:{BG}; color:{TEXT}; font-family:system-ui, sans-serif; \
                    display:flex; flex-direction:column; align-items:center;",
            div {
                style: "width:100%; max-width:520px; box-sizing:border-box; padding:{TOP}px 18px 40px; \
                        display:flex; flex-direction:column; gap:22px;",
                div {
                    style: "display:flex; flex-direction:column; gap:4px;",
                    span { style: "font-size:26px; font-weight:700;", "Session" }
                    span { style: "font-size:14px; color:{DIM};", "Your setlist, live — join a set, or open one." }
                }
                match opening() {
                    Opening::Busy(step) => rsx! { Note { color: ACCENT, "{step}" } },
                    Opening::Failed(why) => rsx! { Note { color: WARN, "Could not open it: {why}" } },
                    Opening::Idle => rsx! {},
                }
                Section { title: "Join a set",
                    Row {
                        title: "The Session demo",
                        detail: "The live set everyone who visits plays together",
                        enabled: !busy,
                        on_press: move |()| join_demo(DEMO_LINK.to_owned()),
                    }
                    div {
                        style: "display:flex; gap:8px; padding:10px 14px; border-bottom:1px solid {RULE};",
                        input {
                            style: "flex:1; min-width:0; height:36px; box-sizing:border-box; padding:0 10px; \
                                    border-radius:8px; border:1px solid {RULE}; background:{BG}; color:{TEXT}; \
                                    font-family:inherit; font-size:14px;",
                            r#type: "text",
                            placeholder: "A live link someone shared",
                            value: "{link}",
                            oninput: move |e| link.set(e.value()),
                        }
                        if cfg!(target_os = "ios") {
                            Button {
                                label: "Paste",
                                enabled: !busy,
                                on_press: move |()| {
                                    if let Some(text) = super::pasted() {
                                        link.set(text.clone());
                                        join_pasted(text);
                                    }
                                },
                            }
                        }
                        Button {
                            label: "Join",
                            enabled: !busy && !link().trim().is_empty(),
                            on_press: move |()| join_link(link()),
                        }
                    }
                }
                Section { title: "Task",
                    match sign_in() {
                        SignIn::Out => rsx! {
                            Row {
                                title: "Sign in",
                                detail: "From a browser where you are signed in to Task",
                                enabled: !busy,
                                on_press: start_sign_in,
                            }
                        },
                        SignIn::Starting => rsx! { Line { color: DIM, "Asking Task for a code…" } },
                        SignIn::Code(started) => rsx! {
                            div {
                                style: "display:flex; flex-direction:column; gap:8px; padding:14px;",
                                span { style: "font-size:13px; color:{DIM};", "Approve this code in the browser:" }
                                span { style: "font-size:28px; font-weight:700; letter-spacing:0.12em;", "{started.code.user_code}" }
                                span { style: "font-size:12px; color:{DIM}; word-break:break-all;", "{started.link()}" }
                                div {
                                    style: "display:flex;",
                                    Button {
                                        label: "Open the page again",
                                        enabled: true,
                                        on_press: move |()| super::open_url(&started.link()),
                                    }
                                }
                            }
                        },
                        SignIn::Failed(why) => rsx! {
                            Line { color: WARN, "Not signed in: {why}" }
                            Row {
                                title: "Try again",
                                detail: "",
                                enabled: !busy,
                                on_press: start_sign_in,
                            }
                        },
                        SignIn::In(account) => rsx! {
                            TaskLibrary {
                                account: account.clone(),
                                orgs: orgs(),
                                org,
                                setlists: setlists(),
                                enabled: !busy,
                                on_open: move |setlist: LibrarySetlist| {
                                    let Some(org) = org() else { return };
                                    let library = account.library(&org);
                                    let name = account.name();
                                    open_set(
                                        format!("Opening {}…", setlist.title),
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
                Section { title: "{LOCAL_TITLE}",
                    if local.is_empty() {
                        Line { color: DIM, "{LOCAL_EMPTY}" }
                    }
                    for path in local.clone() {
                        Row {
                            key: "{path.display()}",
                            title: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            detail: kind_of(&path),
                            enabled: !busy,
                            on_press: move |()| open_path(path.clone()),
                        }
                    }
                    if cfg!(not(target_os = "ios")) {
                        Row {
                            title: "Open a song or setlist…",
                            detail: "A REAPER project, a .session, or a .setlist",
                            enabled: !busy,
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

/// The signed-in person's library: which org, its setlists, signing out.
#[component]
fn TaskLibrary(
    account: Account,
    orgs: Load<Vec<String>>,
    org: Signal<Option<String>>,
    setlists: Load<Vec<LibrarySetlist>>,
    enabled: bool,
    on_open: EventHandler<LibrarySetlist>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let who = account
        .session
        .email
        .clone()
        .unwrap_or_else(|| account.name());
    rsx! {
        div {
            style: "display:flex; align-items:center; gap:8px; padding:10px 14px; border-bottom:1px solid {RULE};",
            span { style: "flex:1; min-width:0; font-size:13px; color:{DIM}; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;", "Signed in as {who}" }
            Button { label: "Sign out", enabled: true, on_press: move |()| on_sign_out.call(()) }
        }
        match orgs {
            Load::Waiting => rsx! { Line { color: DIM, "Finding your orgs…" } },
            Load::Failed(why) => rsx! { Line { color: WARN, "Could not reach Task: {why}" } },
            Load::Ready(list) if list.len() > 1 => rsx! {
                div {
                    style: "display:flex; flex-wrap:wrap; gap:6px; padding:10px 14px; border-bottom:1px solid {RULE};",
                    for slug in list {
                        Chip {
                            key: "{slug}",
                            label: slug.clone(),
                            on: org().as_deref() == Some(slug.as_str()),
                            on_press: move |()| org.set(Some(slug.clone())),
                        }
                    }
                }
            },
            Load::Ready(_) => rsx! {},
        }
        match setlists {
            Load::Waiting => rsx! {
                if org().is_some() {
                    Line { color: DIM, "Finding its setlists…" }
                }
            },
            Load::Failed(why) => rsx! { Line { color: WARN, "Could not read the library: {why}" } },
            Load::Ready(list) if list.is_empty() => rsx! { Line { color: DIM, "No setlists in this library yet." } },
            Load::Ready(list) => rsx! {
                for setlist in list {
                    Row {
                        key: "{setlist.id}",
                        title: setlist.title.clone(),
                        detail: songs_line(&setlist),
                        enabled,
                        on_press: move |()| on_open.call(setlist.clone()),
                    }
                }
            },
        }
    }
}

/// A setlist's songs, in a line.
fn songs_line(setlist: &LibrarySetlist) -> String {
    let titles: Vec<&str> = setlist.songs.iter().map(|s| s.title.as_str()).collect();
    match titles.len() {
        0 => "No songs".to_owned(),
        n => format!("{n} songs · {}", titles.join(", ")),
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

#[component]
fn Section(title: String, children: Element) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            span { style: "font-size:11px; font-weight:700; letter-spacing:0.08em; color:{DIM}; padding-left:4px;", "{title.to_uppercase()}" }
            div {
                style: "display:flex; flex-direction:column; background:{BAR}; border:1px solid {RULE}; \
                        border-radius:12px; overflow:hidden;",
                {children}
            }
        }
    }
}

#[component]
fn Row(title: String, detail: String, enabled: bool, on_press: EventHandler<()>) -> Element {
    let fg = if enabled { TEXT } else { DIM };
    rsx! {
        button {
            style: "display:flex; flex-direction:column; align-items:flex-start; gap:2px; min-height:52px; \
                    padding:10px 14px; border:none; border-bottom:1px solid {RULE}; background:transparent; \
                    color:{fg}; font-family:inherit; text-align:left; cursor:pointer; width:100%;",
            onclick: move |_| {
                if enabled {
                    on_press.call(());
                }
            },
            span { style: "font-size:15px; font-weight:600;", "{title}" }
            if !detail.is_empty() {
                span { style: "font-size:12px; color:{DIM};", "{detail}" }
            }
        }
    }
}

#[component]
fn Button(label: String, enabled: bool, on_press: EventHandler<()>) -> Element {
    let (fg, bg) = if enabled {
        (TEXT, "#23262c")
    } else {
        (DIM, "transparent")
    };
    rsx! {
        button {
            style: "flex:none; height:36px; padding:0 14px; border-radius:8px; border:1px solid {RULE}; \
                    background:{bg}; color:{fg}; font-family:inherit; font-size:14px; font-weight:600; cursor:pointer;",
            onclick: move |_| {
                if enabled {
                    on_press.call(());
                }
            },
            "{label}"
        }
    }
}

#[component]
fn Chip(label: String, on: bool, on_press: EventHandler<()>) -> Element {
    let (fg, bg) = if on {
        (ACCENT, "#1f2a3a")
    } else {
        (DIM, "transparent")
    };
    rsx! {
        button {
            style: "height:30px; padding:0 12px; border-radius:15px; border:1px solid {RULE}; \
                    background:{bg}; color:{fg}; font-family:inherit; font-size:13px; cursor:pointer;",
            onclick: move |_| on_press.call(()),
            "{label}"
        }
    }
}

/// A line of text in a section.
#[component]
fn Line(color: String, children: Element) -> Element {
    rsx! {
        div {
            style: "padding:12px 14px; font-size:13px; line-height:1.5; color:{color}; border-bottom:1px solid {RULE};",
            {children}
        }
    }
}

#[component]
fn Note(color: String, children: Element) -> Element {
    rsx! {
        div {
            style: "padding:10px 14px; border-radius:10px; border:1px solid {RULE}; background:{BAR}; \
                    font-size:13px; line-height:1.5; color:{color};",
            {children}
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
