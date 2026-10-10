//! A song as one project file: `Song.session`, a Loro doc in a Turso
//! file (`dawfile_store`).
//!
//! The song open in the engine is kept in step with its doc by a bridge
//! (`session::sync::Bridge`) that lives as long as the song is open: it
//! knows which engine FX and item each doc entry is, so a save records
//! only what changed. A save appends that as one update to the song's
//! branch; an open rebuilds the song in the engine from the branch's doc.
//! An alternative or checkpoint is another branch of the same file,
//! forked from the song.
//!
//! A `Song.session` folder (the format before this) is read as it always
//! was; the first save writes the file and keeps the folder beside it as
//! `Song.session.legacy`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use daw::standalone::Standalone;
use dawfile_store::{MAIN, Store};
use session::sync::engine::{Blobs, Direct, MediaRoot};
use session::sync::{Bridge, SessionDoc};

/// A project file's blobs, as the bridge keeps FX state.
struct StoreBlobs(Arc<Mutex<Store>>);

impl Blobs for StoreBlobs {
    fn put(&self, bytes: &[u8]) -> String {
        with_store(&self.0, |store| {
            store.put_blob("fx", bytes).map_err(|e| e.to_string())
        })
        .unwrap_or_else(|_| dawfile_store::hash_of(bytes))
    }
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        with_store(&self.0, |store| store.blob(name).map_err(|e| e.to_string())).ok()
    }
}

/// The project file at `path`, shared, and `branch`'s doc from it.
fn open_branch(
    path: &Path,
    branch: &str,
) -> Result<(Arc<Mutex<Store>>, session::sync::loro::LoroDoc), String> {
    let store = Arc::new(Mutex::new(
        Store::open(path).map_err(|e| format!("{}: {e}", path.display()))?,
    ));
    let doc = with_store(&store, |s| s.load(branch).map_err(|e| e.to_string()))?;
    Ok((store, doc))
}

/// `work` on the store, its lock held for that call only.
fn with_store<T>(
    store: &Mutex<Store>,
    work: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, String> {
    let held = store.lock().map_err(|_| "the store's lock")?;
    work(&held)
}

/// A song open from (or saved to) its project file.
struct Open {
    path: PathBuf,
    branch: String,
    store: Arc<Mutex<Store>>,
    bridge: Bridge,
    /// The doc's version at the last save: what the next one appends
    /// from.
    saved: session::sync::loro::VersionVector,
}

/// The open songs, by the engine's project guid.
fn open_songs() -> &'static tokio::sync::Mutex<HashMap<String, Open>> {
    static OPEN: std::sync::LazyLock<tokio::sync::Mutex<HashMap<String, Open>>> =
        std::sync::LazyLock::new(Default::default);
    &OPEN
}

/// Whether `path` is a project file — a `Song.session` that is a file,
/// not the folder of the format before.
#[must_use]
pub fn is_project_file(path: &Path) -> bool {
    crate::open_core::is_session(path) && path.is_file()
}

/// The project file and branch a song path names: `Song.session` (a
/// file) is the song, `Song.session/Organized` its alternative; `None`
/// for anything else (an `.RPP`, a `.session` folder).
#[must_use]
pub fn file_and_branch(path: &Path) -> Option<(PathBuf, String)> {
    if is_project_file(path) {
        return Some((path.to_path_buf(), MAIN.to_owned()));
    }
    let file = path.parent()?;
    let branch = path.file_name()?.to_string_lossy().into_owned();
    is_project_file(file).then(|| (file.to_path_buf(), branch))
}

/// Where a song saves: its project file and branch — an alternative's own
/// branch, else the song's file beside whatever it was opened from.
#[must_use]
pub fn save_target(opened_from: &Path) -> (PathBuf, String) {
    file_and_branch(opened_from).unwrap_or_else(|| {
        let file = if crate::open_core::is_session(opened_from) {
            opened_from.to_path_buf()
        } else {
            opened_from.with_extension("session")
        };
        (file, MAIN.to_owned())
    })
}

/// The project file's own runtime: a song opens before the app's engine
/// runtime and facade are up (at launch), so its work never waits on them.
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: std::sync::LazyLock<Option<tokio::runtime::Runtime>> =
        std::sync::LazyLock::new(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("project-file")
                .thread_stack_size(16 * 1024 * 1024)
                .enable_all()
                .build()
                .ok()
        });
    RUNTIME
        .as_ref()
        .ok_or_else(|| "no runtime for the project file".to_owned())
}

/// Run `work` to completion on the project file's runtime, from any
/// thread — an action's, the window's, or a runtime's own.
fn run<T: Send>(work: impl std::future::Future<Output = T> + Send) -> Result<T, String> {
    let runtime = runtime()?;
    std::thread::scope(|scope| {
        scope
            .spawn(|| runtime.block_on(work))
            .join()
            .map_err(|_| "the project file's work panicked".to_owned())
    })
}

/// A facade on `daw` of the project file's own — served in process, on
/// its runtime — kept while it is the same engine.
async fn client(daw: &Standalone) -> Result<daw_control::Daw, String> {
    // The whole bundle is kept: it owns the task serving the link.
    static CLIENT: std::sync::LazyLock<
        tokio::sync::Mutex<Option<daw::standalone::bootstrap::InProcessDaw>>,
    > = std::sync::LazyLock::new(Default::default);
    let mut held = CLIENT.lock().await;
    let kept = held
        .as_ref()
        .filter(|bundle| bundle.standalone.same_engine(daw))
        .map(|bundle| bundle.daw.clone());
    if let Some(client) = kept {
        drop(held);
        return Ok(client);
    }
    let built = daw::standalone::bootstrap::build_in_process_daw(daw.clone())
        .await
        .map_err(|e| e.to_string())?;
    let client = built.daw.clone();
    *held = Some(built);
    drop(held);
    Ok(client)
}

/// `daw`'s project `guid`, through the project file's facade.
async fn project(daw: &Standalone, guid: &str) -> Result<daw_control::Project, String> {
    client(daw)
        .await?
        .project(guid.to_owned())
        .await
        .map_err(|e| e.to_string())
}

fn direct(daw: &Standalone) -> Arc<dyn Direct> {
    Arc::new(daw.clone())
}

fn media_of(path: &Path) -> MediaRoot {
    MediaRoot(path.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// Save the song (the engine's `project_guid`) into the project file at
/// `path` on `branch`. Says where.
///
/// What changed since its last save goes in as one update named `label`.
/// A `Song.session` folder there is kept beside it as
/// `Song.session.legacy`.
///
/// # Errors
///
/// The file would not open or take the write, or the engine would not be
/// read.
pub fn save(
    daw: &Standalone,
    project_guid: &str,
    path: &Path,
    branch: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let _job = crate::jobs::start("Saving", 0);
    if path.is_dir() {
        let legacy = PathBuf::from(format!("{}.legacy", path.display()));
        std::fs::rename(path, &legacy)
            .map_err(|e| format!("keeping {} as {}: {e}", path.display(), legacy.display()))?;
    }
    let daw = daw.clone();
    let guid = project_guid.to_owned();
    let path = path.to_path_buf();
    let branch = branch.to_owned();
    let label = label.to_owned();
    run(async move {
        let mut songs = open_songs().lock().await;
        let same = songs
            .get(&guid)
            .is_some_and(|open| open.path == path && open.branch == branch);
        if !same {
            // Into this file for the first time (or another file): its
            // history so far, the song written over it.
            // A copy saved elsewhere leaves the song's own bridge — and
            // what it knows of the song's ids — where it is.
            if songs.contains_key(&guid) {
                drop(songs);
                let mut copy = begin(&daw, &guid, &path, &branch).await?;
                return save_open(&mut copy, &label).await.map(|()| path);
            }
            songs.insert(guid.clone(), begin(&daw, &guid, &path, &branch).await?);
        }
        let saved = match songs.get_mut(&guid) {
            Some(open) => save_open(open, &label).await,
            None => Err("the song is not open".to_owned()),
        };
        drop(songs);
        saved.map(|()| path)
    })?
}

/// The song `guid` into the project file at `path` on `branch`: the
/// branch's history so far, a bridge reading the song over it.
async fn begin(daw: &Standalone, guid: &str, path: &Path, branch: &str) -> Result<Open, String> {
    let (store, loaded) = open_branch(path, branch)?;
    let doc = SessionDoc::from_loro(loaded);
    let saved = doc.version();
    let chart = doc.read().chart;
    let bridge = Bridge::host(
        project(daw, guid).await?,
        media_of(path),
        doc,
        chart,
        Arc::new(StoreBlobs(store.clone())),
        Some(direct(daw)),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(Open {
        path: path.to_path_buf(),
        branch: branch.to_owned(),
        store,
        bridge,
        saved,
    })
}

/// The song as it is now into its file: read, and what changed appended.
async fn save_open(open: &mut Open, label: &str) -> Result<(), String> {
    open.bridge
        .local_changed()
        .await
        .map_err(|e| e.to_string())?;
    append(open, label)
}

/// Append what the doc holds past its last save to the song's branch.
fn append(open: &mut Open, label: &str) -> Result<(), String> {
    let doc = open.bridge.doc();
    let update = doc.updates_since(&open.saved).map_err(|e| e.to_string())?;
    if doc.version() != open.saved {
        with_store(&open.store, |s| {
            s.append_update(&open.branch, label, &update)
                .map_err(|e| e.to_string())
        })?;
        open.saved = doc.version();
    }
    // One file again: what was written folded out of the log beside it.
    with_store(&open.store, |s| s.checkpoint().map_err(|e| e.to_string()))
}

/// Open `branch` of the project file at `path` as a new song in `daw`:
/// its doc rebuilt in the engine. Its media resolves against the file's
/// folder. Returns the engine's project guid.
///
/// # Errors
///
/// The file would not open or load, or the engine refused to rebuild it.
pub fn load(daw: &Standalone, name: &str, path: &Path, branch: &str) -> Result<String, String> {
    let _job = crate::jobs::start("Opening", 0);
    let (store, loaded) = open_branch(path, branch)?;
    let doc = SessionDoc::from_loro(loaded);
    let saved = doc.version();
    let guid = uuid::Uuid::new_v4().to_string();
    daw.seed_project(daw_proto::ProjectInfo {
        guid: guid.clone(),
        name: name.to_owned(),
        path: path.to_string_lossy().into_owned(),
    });
    let daw = daw.clone();
    let path = path.to_path_buf();
    let branch = branch.to_owned();
    let made = guid.clone();
    run(async move {
        let bridge = Bridge::join(
            project(&daw, &made).await?,
            media_of(&path),
            doc,
            Arc::new(StoreBlobs(store.clone())),
            Some(direct(&daw)),
        )
        .await
        .map_err(|e| e.to_string())?;
        open_songs().lock().await.insert(
            made,
            Open {
                path,
                branch,
                store,
                bridge,
                saved,
            },
        );
        Ok::<(), String>(())
    })??;
    Ok(guid)
}

/// Fork `name` from the song open as `project_guid`, as it is now.
///
/// `path` is what it was opened from: the song or one of its
/// alternatives. The song is saved first, then the branch made at that
/// point, from its own. The song stays what is open. Says the project
/// file.
///
/// # Errors
///
/// A name with a path separator in it or none at all, or as [`save`].
pub fn fork(
    daw: &Standalone,
    project_guid: &str,
    path: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) || name == dawfile_store::MAIN {
        return Err(format!("{name:?} is not a name an alternative can have"));
    }
    let (file, branch) = save_target(path);
    save(daw, project_guid, &file, &branch, &format!("Before {name}"))?;
    let guid = project_guid.to_owned();
    let name = name.to_owned();
    run(async move {
        let songs = open_songs().lock().await;
        let open = songs.get(&guid).ok_or("the song is not open")?;
        let forked = with_store(&open.store, |s| {
            s.fork(&name, &open.branch, open.bridge.doc().loro())
                .and_then(|()| s.checkpoint())
                .map_err(|e| e.to_string())
        });
        drop(songs);
        forked
    })??;
    Ok(file)
}

/// Every branch of the project file at `path`: the song, then its
/// alternatives and checkpoints.
///
/// # Errors
///
/// The file would not open.
pub fn branches(path: &Path) -> Result<Vec<dawfile_store::Branch>, String> {
    Store::open(path)
        .and_then(|store| store.branches())
        .map_err(|e| e.to_string())
}

/// The doc `path` (a project file, or an alternative in one) holds, as
/// saved; `None` for anything else.
#[must_use]
pub fn doc_of(path: &Path) -> Option<session::sync::loro::LoroDoc> {
    let (file, branch) = file_and_branch(path)?;
    Store::open(&file).ok()?.load(&branch).ok()
}

/// Whether the song open as `project_guid` is what its project file
/// (`path`) reopens as.
///
/// The file is rebuilt in a second project and both are read as the doc
/// keeps them (its own ids, not the engine's), compared part by part.
/// Empty when the same; else what differs. The second project is closed
/// again.
///
/// # Errors
///
/// The file would not open, or either song would not be read.
pub fn check(daw: &Standalone, project_guid: &str, path: &Path) -> Result<Vec<String>, String> {
    let (file, branch) = save_target(path);
    let reopened = load(daw, "check", &file, &branch)?;
    let live_guid = project_guid.to_owned();
    let daw_for = daw.clone();
    let apart = run(async move {
        let saved = open_songs()
            .lock()
            .await
            .remove(&reopened)
            .ok_or("the saved song did not open")?
            .bridge
            .shown()
            .clone();
        // The live song as the doc reads it: through its own bridge when
        // it has one (its ids), else read afresh.
        let own = live_read(&live_guid).await?;
        let live = match own {
            Some(live) => live,
            None => Bridge::host(
                project(&daw_for, &live_guid).await?,
                media_of(&file),
                SessionDoc::new(),
                saved.chart.clone(),
                Arc::new(session::sync::engine::MemoryBlobs::default()),
                Some(direct(&daw_for)),
            )
            .await
            .map_err(|e| e.to_string())?
            .shown()
            .clone(),
        };
        Ok::<_, String>((reopened, differences(&live, &saved)))
    })??;
    let _ = daw::service::Projects::close(daw, &apart.0);
    Ok(apart.1)
}

/// The song `guid` as its own bridge reads it now, if it has one.
async fn live_read(guid: &str) -> Result<Option<session::sync::SessionModel>, String> {
    let mut songs = open_songs().lock().await;
    let Some(open) = songs.get_mut(guid) else {
        return Ok(None);
    };
    let read = open.bridge.local_changed().await.map_err(|e| e.to_string());
    let shown = open.bridge.shown().clone();
    drop(songs);
    read.map(|_| Some(shown))
}

/// What differs between two songs as the doc reads them, part by part:
/// each track and item that is not the same, by its guid, first few.
fn differences(
    live: &session::sync::SessionModel,
    saved: &session::sync::SessionModel,
) -> Vec<String> {
    const SHOWN: usize = 4;
    let mut out = Vec::new();
    let tracks: Vec<String> = live
        .tracks
        .iter()
        .filter(|t| saved.track(&t.guid) != Some(*t))
        .map(|t| {
            let parts = saved.track(&t.guid).map_or_else(String::new, |s| {
                [
                    ("fx", t.fx != s.fx),
                    ("sends", t.sends != s.sends),
                    ("envelopes", t.envelopes != s.envelopes),
                    ("lanes", t.lanes != s.lanes),
                    ("groups", t.grouping != s.grouping),
                ]
                .into_iter()
                .filter(|(_, differs)| *differs)
                .map(|(part, _)| part)
                .collect::<Vec<_>>()
                .join(", ")
            });
            // What each side's chain is, when that is what differs.
            let chains =
                saved
                    .track(&t.guid)
                    .filter(|s| s.fx != t.fx)
                    .map_or_else(String::new, |s| {
                        let chain = |fx: &[session::sync::FxState]| {
                            fx.iter()
                                .map(|f| {
                                    format!(
                                        "{}{}",
                                        f.name,
                                        f.state.as_deref().map_or_else(String::new, |h| {
                                            format!("@{}", h.get(..8).unwrap_or(h))
                                        })
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        };
                        format!(" (live [{}], saved [{}])", chain(&t.fx), chain(&s.fx))
                    });
            let lanes = saved
                .track(&t.guid)
                .filter(|s| s.lanes != t.lanes)
                .map_or_else(String::new, |s| {
                    let lane = |l: &session::sync::LaneState| {
                        format!(
                            "{} lanes, playing {:b}, named {:?}",
                            l.count, l.play_mask, l.names
                        )
                    };
                    format!(" (live {}; saved {})", lane(&t.lanes), lane(&s.lanes))
                });
            format!("track {} {:?}: {parts}{chains}{lanes}", t.guid, t.name)
        })
        .chain(
            saved
                .tracks
                .iter()
                .filter(|t| live.track(&t.guid).is_none())
                .map(|t| format!("track {} {:?} (only saved)", t.guid, t.name)),
        )
        .collect();
    if !tracks.is_empty() {
        out.push(format!("tracks: {} differ", tracks.len()));
        out.extend(tracks.into_iter().take(SHOWN).map(|l| format!("  {l}")));
    }
    let items: Vec<String> = live
        .items
        .iter()
        .filter(|(g, i)| saved.items.get(*g) != Some(*i))
        .map(|(g, _)| format!("item {g}"))
        .chain(
            saved
                .items
                .keys()
                .filter(|g| !live.items.contains_key(*g))
                .map(|g| format!("item {g} (only saved)")),
        )
        .collect();
    if !items.is_empty() {
        out.push(format!("items: {} differ", items.len()));
        out.extend(items.into_iter().take(SHOWN).map(|l| format!("  {l}")));
    }
    for (what, same) in [
        ("markers", live.markers == saved.markers),
        ("regions", live.regions == saved.regions),
        ("tempo", live.tempo == saved.tempo),
    ] {
        if !same {
            out.push(format!("{what} differ"));
        }
    }
    out
}
