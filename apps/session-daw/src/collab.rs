//! Collaboration: share the open setlist with others, or join someone's.
//!
//! A session is the whole SET: every song has its own doc (and its own
//! bridge to its engine project), and one presence channel spans them, so
//! people can be on different songs and still see where everyone is —
//! the song tabs show who is on which, and the arrangement draws the
//! people on the song it shows. A lone song is a set of one.
//!
//! One task per session owns everything that must not race: the bridges,
//! this peer's presence, and the shared transport's follower. It wakes on
//! three things —
//!
//! - a song's engine project changed (its event bus): read it back, write
//!   that song's doc;
//! - a song's doc changed remotely (a Loro import): make that project
//!   match, and have the arrangement read it back if it is showing it;
//! - a ~30 Hz tick: publish what this peer is doing (throttled, only what
//!   changed), fold everyone's presence into the roster, and keep the
//!   engine on the shared transport.
//!
//! Hosting serves the set over iroh; joining dials it. The ticket
//! (`fts-session:<endpoint id>/<set uuid>`) is what you hand someone. A
//! joiner opens the same setlist first — the media is read from its own
//! copy — and each of its songs is then made to match the host's.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use session::sync::engine::MediaRoot;
use session::sync::net::{PresenceSink, SetHost, SetPeer, session_id, song_id};
use session::sync::presence::{self, PeerState, PlayState, Pointer, Roster, Throttle};
use session::sync::clock::SharedClock;
use session::sync::transport::{
    self, Command, LocalTransport, SharedTransport, SyncPosition, TransportMode, TransportSync,
};
use session::sync::{Bridge, Change, ORIGIN_LOCAL, SessionDoc};
use uuid::Uuid;

/// What the collaboration is doing, for the toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub hosting: bool,
    /// What to hand someone so they can join.
    pub ticket: String,
    /// How many others are here.
    pub peers: usize,
    pub shared_transport: bool,
    /// The set being shared (`Worship Set`, or a lone song's name).
    pub set: String,
    /// This person, as the others see them: name and colour.
    pub name: String,
    pub color: u32,
}

/// One song of the set, as the session keeps it.
#[derive(Clone)]
struct Song {
    /// Its engine project (minted per open — this machine's only).
    project: String,
    /// The name every machine knows it by: its file's stem (`Washed`).
    key: String,
    doc: SessionDoc,
}

struct Live {
    status: Status,
    stop: tokio::sync::watch::Sender<bool>,
    /// This peer's end of the shared transport.
    sync: Arc<Mutex<TransportSync>>,
    /// Charts the others changed, by project, since the editor looked.
    remote_chart: Arc<Mutex<HashMap<String, String>>>,
    songs: Vec<Song>,
    presence: Arc<dyn PresenceSink>,
    me: String,
    /// Shared with anyone (hosting or joined), or only this machine's
    /// record of the set's edits.
    shared: bool,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

/// Whether an environment switch is on: set, and not to nothing.
#[must_use]
pub fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

/// Why the last share or join failed, for the bar.
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Why the last share or join failed, if it did.
#[must_use]
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().ok()?.clone()
}

fn record<T>(result: eyre::Result<T>) -> eyre::Result<T> {
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = result.as_ref().err().map(ToString::to_string);
    }
    result
}

/// [`host`] off the calling thread (a click must not wait on iroh).
pub fn host_in_background(name: String) {
    std::thread::spawn(move || {
        let _ = host(name);
    });
}

/// [`join`] off the calling thread.
pub fn join_in_background(ticket: String, name: String) {
    std::thread::spawn(move || {
        let _ = join(&ticket, name);
    });
}

/// The session's status, if one is live.
#[must_use]
pub fn status() -> Option<Status> {
    LIVE.lock().ok()?.as_ref().filter(|l| l.shared).map(|l| l.status.clone())
}

fn song_where(pick: impl Fn(&Song) -> bool) -> Option<(Song, Arc<dyn PresenceSink>, String)> {
    let live = LIVE.lock().ok()?;
    let l = live.as_ref()?;
    let song = l.songs.iter().find(|s| pick(s))?.clone();
    Some((song, Arc::clone(&l.presence), l.me.clone()))
}

fn current() -> Option<(Song, Arc<dyn PresenceSink>, String)> {
    let project = crate::open::current_song()?;
    song_where(|s| s.project == project)
}

/// The name a song is known by everywhere (`Washed`), from its project
/// here.
#[must_use]
pub fn key_of(project: &str) -> Option<String> {
    song_where(|s| s.project == project).map(|(s, _, _)| s.key)
}

/// A song's full edit history, to save with it — `None` when no session
/// doc is keeping one for `project` (yet).
#[must_use]
pub fn history(project: &str) -> Option<session::sync::loro::LoroDoc> {
    song_where(|s| s.project == project).map(|(s, _, _)| s.doc.loro().clone())
}

/// Presence for a session nobody else is in.
struct Alone;

impl PresenceSink for Alone {
    fn set(&self, _: &str, _: session::sync::loro::LoroValue) {}
    fn delete(&self, _: &str) {}
    fn states(&self) -> std::collections::HashMap<String, session::sync::loro::LoroValue> {
        std::collections::HashMap::new()
    }
}

/// Every song open in the engine: its project, where its media is, the
/// name every machine knows it by, and where it was opened from.
///
/// The name, not the project guid: the engine mints a guid per open, so
/// two copies of one song never share it. The file's stem is the same on
/// every machine (`Washed`); once songs live in the Task library this is
/// the library's song id.
async fn open_songs() -> eyre::Result<Vec<(daw_control::Project, MediaRoot, String, std::path::PathBuf)>> {
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("the daw facade is not up"))?;
    let mut out = Vec::new();
    for project in daw.projects().await? {
        let path = std::path::PathBuf::from(project.info().await?.path);
        let folder = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let key = path
            .file_stem()
            .map_or_else(|| project.guid().to_owned(), |s| s.to_string_lossy().into_owned());
        out.push((project, MediaRoot(folder), key, path));
    }
    Ok(out)
}

/// The name the set is known by: the setlist file's stem, or — a lone
/// song — that song's.
fn set_key(songs: &[(daw_control::Project, MediaRoot, String, std::path::PathBuf)]) -> String {
    std::env::var_os("FTS_SESSION_SETLIST")
        .and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()))
        .or_else(|| songs.first().map(|s| s.2.clone()))
        .unwrap_or_else(|| "session".into())
}

/// The song's chart: the one beside it, or what its doc already holds.
fn chart_for(path: &std::path::Path, doc: &SessionDoc) -> String {
    crate::prepare::chart_beside(path)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| doc.read().chart)
}

/// Keep a session doc for every open song, shared with nobody: every edit
/// is recorded (and saved with the song's `.session`), and sharing later
/// puts these same docs — history and all — on the network. Each starts
/// from the history its `.session` was saved with, when it still matches.
///
/// # Errors
/// When the engine is not up or cannot be read.
pub fn open_local() -> eyre::Result<()> {
    record(open_local_with(HashMap::new()))
}

fn open_local_with(mut docs: HashMap<String, SessionDoc>) -> eyre::Result<()> {
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let live = runtime.block_on(async move {
        let open = open_songs().await?;
        // Already keeping exactly these songs: nothing to do.
        let keeping = |l: &Live| {
            l.songs.len() == open.len() && open.iter().all(|(p, ..)| l.songs.iter().any(|s| s.project == p.guid()))
        };
        // …or sharing: a shared set is changed by leaving it.
        if docs.is_empty() && LIVE.lock().ok().is_some_and(|l| l.as_ref().is_some_and(|l| l.shared || keeping(l))) {
            return Ok(None);
        }
        if docs.is_empty() {
            docs = LIVE.lock().ok().map(|mut l| stop(l.take())).unwrap_or_default();
        }
        let mut bridges = Vec::new();
        for (project, media, key, path) in open {
            let doc = docs.remove(&key).unwrap_or_else(|| {
                crate::open::is_session(&path)
                    .then(|| crate::session_file::load_session_history(&path))
                    .flatten()
                    .map_or_else(SessionDoc::new, SessionDoc::from_loro)
            });
            let chart = chart_for(&path, &doc);
            let guid = project.guid().to_owned();
            let bridge = Bridge::host(project, media, doc, chart).await?;
            bridges.push((Song { project: guid, key, doc: bridge.doc().clone() }, bridge));
        }
        let presence: Arc<dyn PresenceSink> = Arc::new(Alone);
        let started = start(bridges, Arc::clone(&presence), SharedClock::owned(), String::new(), "local".into(), String::new(), None, None);
        let mut live = started.into_live(String::new(), false, presence);
        live.shared = false;
        Ok::<_, eyre::Report>(Some(live))
    })?;
    if let Some(live) = live
        && let Ok(mut slot) = LIVE.lock()
    {
        stop(slot.take());
        *slot = Some(live);
    }
    Ok(())
}

/// Stop whatever is keeping the set, handing back its songs' docs.
fn stop(live: Option<Live>) -> HashMap<String, SessionDoc> {
    let Some(live) = live else { return HashMap::new() };
    let _ = live.stop.send(true);
    live.songs.into_iter().map(|s| (s.key, s.doc)).collect()
}

/// A chart the others changed since the editor last looked, for the song
/// on screen.
#[must_use]
pub fn take_remote_chart() -> Option<String> {
    let project = crate::open::current_song()?;
    let live = LIVE.lock().ok()?;
    live.as_ref()?.remote_chart.lock().ok()?.remove(&project)
}

/// The song on screen's doc and the presence, for the chart editor's
/// carets.
#[must_use]
pub fn chart_context() -> Option<(SessionDoc, Arc<dyn PresenceSink>, String)> {
    current().map(|(s, p, me)| (s.doc, p, me))
}

/// The chart was edited here (the song on screen).
pub fn local_chart(text: &str) {
    let Some((song, _, _)) = current() else { return };
    // The bridge owns doc writes; a chart edit is recorded straight into
    // the text, which the next local reconcile leaves alone.
    let mut model = song.doc.read();
    if model.chart != text {
        model.chart = text.to_string();
        if let Err(e) = song.doc.write(&model, ORIGIN_LOCAL) {
            tracing::warn!(collab.chart_error = %e, "collab: the chart edit was not recorded");
        }
    }
}

/// Switch between everyone playing on their own and one transport — for
/// everyone, from where this engine is.
pub fn set_shared_transport(shared: bool) {
    let Ok(mut live) = LIVE.lock() else { return };
    let Some(l) = live.as_mut() else { return };
    let mode = if shared { TransportMode::Shared } else { TransportMode::Independent };
    let local = local_transport(&l.songs);
    let Ok(mut sync) = l.sync.lock() else { return };
    let entry = sync.set_mode(mode, &local, crate::ghosts::now_ms());
    l.status.shared_transport = shared;
    l.presence.set(transport::KEY, entry.encode());
}

/// Someone pressed something on this transport — play, stop, a seek, home,
/// end, another song. Playing together, it becomes everyone's once it has
/// landed.
pub fn transport_pressed() {
    let Ok(live) = LIVE.lock() else { return };
    if let Some(l) = live.as_ref()
        && let Ok(mut sync) = l.sync.lock()
    {
        sync.pressed(crate::ghosts::now_ms());
    }
}

/// Whether this window is following someone else's transport (playing
/// together, and not the one who pressed last). A follower does nothing
/// to its transport on its own — not even rolling into the next song at
/// the end of one: the leader does, and the follower follows.
#[must_use]
pub fn following() -> bool {
    let Ok(live) = LIVE.lock() else { return false };
    let Some(l) = live.as_ref() else { return false };
    let Ok(sync) = l.sync.lock() else { return false };
    sync.mode() == TransportMode::Shared && sync.current().is_some_and(|c| c.by != l.me)
}

/// The song the shared transport wants this window on (its project here),
/// once — for the setlist to open.
static SONG_REQUEST: Mutex<Option<String>> = Mutex::new(None);

/// The song the shared transport asked for, if it asked since last time.
#[must_use]
pub fn take_song_request() -> Option<String> {
    SONG_REQUEST.lock().ok()?.take()
}

/// What this engine is doing: playing, where, on which song (by name).
fn local_transport(songs: &[Song]) -> LocalTransport {
    let (position, playing) = crate::engine::Transport::shared().map_or((0.0, false), |t| t.read());
    let song = crate::open::current_song()
        .and_then(|p| songs.iter().find(|s| s.project == p).map(|s| s.key.clone()));
    LocalTransport { playing, position, song }
}

/// Leave the session (or stop hosting it). The songs keep their docs —
/// everything edited together stays in their history — shared with nobody.
pub fn leave() {
    let docs = LIVE.lock().ok().map(|mut live| {
        let shared = live.as_ref().is_some_and(|l| l.shared);
        if shared { stop(live.take()) } else { HashMap::new() }
    });
    crate::ghosts::publish(None, 0.0, false);
    crate::ghosts::publish_everyone(Vec::new());
    if let Some(docs) = docs
        && !docs.is_empty()
    {
        std::thread::spawn(move || {
            if let Err(e) = open_local_with(docs) {
                tracing::warn!(collab.error = %e, "collab: the songs' history could not be kept");
            }
        });
    }
}

fn next_seq() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let base = u64::try_from(web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()))
    .unwrap_or(0);
    base.saturating_mul(1000).saturating_add(SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000)
}

/// An iroh identity for one session. Ephemeral: a ticket is minted per
/// session, so nothing needs to recognise this peer later — and two app
/// instances on one machine (the local demo) must not share one.
fn secret_key() -> architect::iroh_link::iroh::SecretKey {
    architect::iroh_link::iroh::SecretKey::generate()
}

/// Share the open set. Returns the ticket to hand others.
///
/// # Errors
/// When nothing is open, the engine cannot be read, or iroh cannot bind.
pub fn host(name: String) -> eyre::Result<String> {
    record(host_inner(name))
}

fn host_inner(name: String) -> eyre::Result<String> {
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let (ticket, live) = runtime.block_on(async move {
        let open = open_songs().await?;
        let set = set_key(&open);
        let id = session_id(&set);
        // The songs' own docs, history and all, are what go on the network.
        let mut docs = LIVE.lock().ok().map(|mut l| stop(l.take())).unwrap_or_default();
        let host = SetHost::new(id);
        let mut bridges = Vec::new();
        for (project, media, key, path) in open {
            let doc = docs.remove(&key).unwrap_or_default();
            let chart = chart_for(&path, &doc);
            let guid = project.guid().to_owned();
            let bridge = Bridge::host(project, media, doc, chart).await?;
            host.add(song_id(&set, &key), bridge.doc());
            bridges.push((Song { project: guid, key, doc: bridge.doc().clone() }, bridge));
        }
        let endpoint = architect::iroh_link::bind_endpoint(secret_key())
            .await
            .map_err(|e| eyre::eyre!("iroh: {e}"))?;
        let ticket = format!("fts-session:{}/{id}", endpoint.id());
        let router = host.mount(architect::LayerRouter::new());
        let serving = endpoint.clone();
        tokio::spawn(async move { architect::iroh_link::serve_router(&serving, router).await });

        let me = format!("host-{}", &endpoint.id().to_string()[..8]);
        let presence: Arc<dyn PresenceSink> = Arc::new(host.own_presence().await?);
        let started = start(bridges, Arc::clone(&presence), SharedClock::owned(), set, me, name, Some(host), Some(endpoint));
        Ok::<_, eyre::Report>((ticket.clone(), started.into_live(ticket, true, presence)))
    })?;
    if let Ok(mut slot) = LIVE.lock() {
        *slot = Some(live);
    }
    tracing::info!(collab.role = "host", "collab: sharing the set");
    Ok(ticket)
}

/// Join a session from its ticket. The same setlist must be open here.
///
/// # Errors
/// On a malformed ticket, a different set open, or a failed dial.
pub fn join(ticket: &str, name: String) -> eyre::Result<()> {
    record(join_inner(ticket, name))
}

fn join_inner(ticket: &str, name: String) -> eyre::Result<()> {
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let (endpoint_id, id) = parse_ticket(ticket)?;
    let live = runtime.block_on(async move {
        let open = open_songs().await?;
        let set = set_key(&open);
        if session_id(&set) != id {
            eyre::bail!("open the same setlist first — this ticket is for another one");
        }
        let endpoint = architect::iroh_link::bind_endpoint(secret_key())
            .await
            .map_err(|e| eyre::eyre!("iroh: {e}"))?;
        let dial = async |endpoint: &architect::iroh_link::iroh::Endpoint| {
            architect::iroh_link::connect(endpoint, endpoint_id)
                .await
                .map_err(|e| eyre::eyre!("iroh connect: {e}"))
        };
        let sync = vox_core::initiator_on(dial(&endpoint).await?)
            .establish::<crdt::sync::DocSyncClient>()
            .await
            .map_err(|e| eyre::eyre!("session sync handshake: {e:?}"))?;
        let presence_client = vox_core::initiator_on(dial(&endpoint).await?)
            .establish::<crdt::sync::DocPresenceClient>()
            .await
            .map_err(|e| eyre::eyre!("session presence handshake: {e:?}"))?;
        // The host's clock is the session's: pinged from here on.
        let clock = SharedClock::follow(
            vox_core::initiator_on(dial(&endpoint).await?)
                .establish::<session::sync::clock::SessionClockClient>()
                .await
                .map_err(|e| eyre::eyre!("session clock handshake: {e:?}"))?,
        );

        let mut peer = SetPeer::new(id);
        peer.run_presence(presence_client);
        let presence: Arc<dyn PresenceSink> = Arc::new(peer.presence().clone());
        // Every song, replicated from the host.
        let replicas: Vec<(SessionDoc, _)> = open
            .into_iter()
            .map(|song| (SetPeer::sync_song(song_id(&set, &song.2), sync.clone()), song))
            .collect();
        // The host's songs arrive first; only then may each bridge make
        // its project match. A song the host does not have keeps this
        // machine's own record, unshared.
        for _ in 0..400 {
            if replicas.iter().all(|(doc, _)| doc.has_session()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let mut docs = LIVE.lock().ok().map(|mut l| stop(l.take())).unwrap_or_default();
        let mut bridges = Vec::new();
        for (doc, (project, media, key, path)) in replicas {
            let guid = project.guid().to_owned();
            let bridge = if doc.has_session() {
                Bridge::join(project, media, doc).await?
            } else {
                let own = docs.remove(&key).unwrap_or_default();
                let chart = chart_for(&path, &own);
                Bridge::host(project, media, own, chart).await?
            };
            bridges.push((Song { project: guid, key, doc: bridge.doc().clone() }, bridge));
        }
        crate::studio::request_resync();
        let me = format!("peer-{}", &endpoint.id().to_string()[..8]);
        let started = start(bridges, Arc::clone(&presence), clock, set, me, name, None, Some(endpoint));
        let _keep = peer;
        Ok::<_, eyre::Report>(started.into_live(ticket.to_string(), false, presence))
    })?;
    if let Ok(mut slot) = LIVE.lock() {
        *slot = Some(live);
    }
    tracing::info!(collab.role = "peer", "collab: joined a session");
    Ok(())
}

fn parse_ticket(ticket: &str) -> eyre::Result<(architect::iroh_link::iroh::EndpointId, Uuid)> {
    let rest = ticket
        .trim()
        .strip_prefix("fts-session:")
        .ok_or_else(|| eyre::eyre!("not a session ticket"))?;
    let (endpoint, id) = rest.split_once('/').ok_or_else(|| eyre::eyre!("not a session ticket"))?;
    Ok((
        endpoint.parse().map_err(|e| eyre::eyre!("endpoint id: {e}"))?,
        id.parse().map_err(|e| eyre::eyre!("session id: {e}"))?,
    ))
}

/// A set Task keeps (`live-proto`): where to reach Task, and which
/// setlist — a member's org lane with its token, or a live share link's
/// guest lane with none.
#[derive(Clone, Debug)]
pub struct TaskSet {
    /// The vox URL (`wss://…/org/<org>/vox`, or `…/org/<org>/share/<token>/vox`).
    pub url: String,
    /// The member's bearer token; `None` on a share link.
    pub token: Option<String>,
    /// The setlist's id in Task.
    pub setlist: String,
}

/// How the environment names a set Task keeps (`FTS_COLLAB_TASK`): a
/// setlist's id, joined as a member of the library's org (`FTS_TASK_*`), or
/// `share:<live share link>`, joined as a guest of the link's set.
pub const TASK_SET_ENV: &str = "FTS_COLLAB_TASK";

impl TaskSet {
    /// The set `FTS_COLLAB_TASK` names, if it names one.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let value = std::env::var(TASK_SET_ENV).ok().filter(|v| !v.trim().is_empty())?;
        Some(Self::parse(value.trim()))
    }

    /// A setlist id, or `share:<link>`.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.strip_prefix("share:") {
            Some(link) => {
                let link = link.trim_end_matches('/');
                let base = link.split('?').next().unwrap_or(link);
                let url = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
                Self { url: format!("{url}/vox"), token: None, setlist: String::new() }
            }
            None => {
                let library = session_library::Library::from_env();
                Self { url: library.org_url(), token: library.token, setlist: value.to_owned() }
            }
        }
    }
}

/// [`join_task`] off the calling thread.
pub fn join_task_in_background(set: TaskSet, name: String) {
    std::thread::spawn(move || {
        let _ = join_task(&set, name);
    });
}

/// Join a set Task keeps: its songs' docs synced through Task, its
/// presence, Task's clock. Each open song meets the set's song of the same
/// library id; the first peer on a song seeds its doc from what it has open
/// (Task starts every song empty), and everyone after takes the doc's
/// state. A song the set does not have stays this machine's own.
///
/// # Errors
///
/// Task could not be reached or refused the set, or an engine call failed.
pub fn join_task(set: &TaskSet, name: String) -> eyre::Result<()> {
    record(join_task_inner(set, name))
}

fn join_task_inner(set: &TaskSet, name: String) -> eyre::Result<()> {
    use live_proto::LiveSessionsClient;
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let set = set.clone();
    let live = runtime.block_on(async move {
        let open = open_songs().await?;
        let token = set.token.as_deref();
        let lane: LiveSessionsClient = task_dial::establish_at(&set.url, token)
            .await
            .map_err(|e| eyre::eyre!("dialling {}: {e}", set.url))?;
        let joined = lane
            .join(set.setlist.clone())
            .await
            .map_err(|e| eyre::eyre!("joining the set: {e:?}"))?;
        let sync: crdt::sync::DocSyncClient = task_dial::establish_at(&set.url, token)
            .await
            .map_err(|e| eyre::eyre!("session sync: {e}"))?;
        let presence_client: crdt::sync::DocPresenceClient = task_dial::establish_at(&set.url, token)
            .await
            .map_err(|e| eyre::eyre!("session presence: {e}"))?;
        // Task's clock is the session's.
        let clock = SharedClock::follow_with(move || {
            let lane = lane.clone();
            async move { lane.now().await.ok() }
        });
        let presence_id: Uuid = joined.presence_id.parse().map_err(|e| eyre::eyre!("presence id: {e}"))?;
        let mut peer = SetPeer::new(presence_id);
        peer.run_presence(presence_client);
        let presence: Arc<dyn PresenceSink> = Arc::new(peer.presence().clone());

        // Each open song, replicated from the set's song of its id.
        let songs: HashMap<String, Uuid> = joined
            .songs
            .iter()
            .filter_map(|s| Some((s.slug.clone(), s.doc_id.parse().ok()?)))
            .collect();
        let replicas: Vec<(Option<SessionDoc>, _)> = open
            .into_iter()
            .map(|song| {
                let doc = songs
                    .get(&session_library::slugify(&song.2))
                    .map(|id| SetPeer::sync_song(*id, sync.clone()));
                (doc, song)
            })
            .collect();
        // What Task has arrives at once; a song still empty after that is
        // this peer's to seed.
        for _ in 0..80 {
            if replicas.iter().all(|(doc, _)| doc.as_ref().is_none_or(SessionDoc::has_session)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let mut docs = LIVE.lock().ok().map(|mut l| stop(l.take())).unwrap_or_default();
        let mut bridges = Vec::new();
        let mut seeded = 0usize;
        for (doc, (project, media, key, path)) in replicas {
            let guid = project.guid().to_owned();
            let bridge = match doc {
                Some(doc) if doc.has_session() => Bridge::join(project, media, doc).await?,
                Some(doc) => {
                    seeded += 1;
                    let chart = chart_for(&path, &doc);
                    Bridge::host(project, media, doc, chart).await?
                }
                None => {
                    let own = docs.remove(&key).unwrap_or_default();
                    let chart = chart_for(&path, &own);
                    Bridge::host(project, media, own, chart).await?
                }
            };
            bridges.push((Song { project: guid, key, doc: bridge.doc().clone() }, bridge));
        }
        crate::studio::request_resync();
        let me = format!("peer-{}", &Uuid::new_v4().simple().to_string()[..8]);
        tracing::info!(
            collab.setlist = %joined.setlist,
            collab.epoch = joined.epoch,
            collab.songs = bridges.len(),
            collab.seeded = seeded,
            "collab: joined a set Task keeps"
        );
        let started = start(bridges, Arc::clone(&presence), clock, joined.setlist.clone(), me, name, None, None);
        let _keep = peer;
        Ok::<_, eyre::Report>(started.into_live(set.url.clone(), false, presence))
    })?;
    if let Ok(mut slot) = LIVE.lock() {
        *slot = Some(live);
    }
    Ok(())
}

/// The half-built `Live` [`start`] returns; finished by the caller.
struct Started {
    stop: tokio::sync::watch::Sender<bool>,
    sync: Arc<Mutex<TransportSync>>,
    remote_chart: Arc<Mutex<HashMap<String, String>>>,
    songs: Vec<Song>,
    me: String,
    name: String,
    set: String,
}

impl Started {
    fn into_live(self, ticket: String, hosting: bool, presence: Arc<dyn PresenceSink>) -> Live {
        Live {
            status: Status {
                hosting,
                ticket,
                peers: 0,
                shared_transport: false,
                set: self.set,
                color: presence::color_for(&self.me),
                name: self.name,
            },
            stop: self.stop,
            sync: self.sync,
            remote_chart: self.remote_chart,
            songs: self.songs,
            presence,
            me: self.me,
            shared: true,
        }
    }
}

/// Spawn the session's task. `host` and `endpoint` are kept alive by it.
#[allow(clippy::too_many_arguments)]
fn start(
    bridges: Vec<(Song, Bridge)>,
    presence: Arc<dyn PresenceSink>,
    clock: SharedClock,
    set: String,
    me: String,
    name: String,
    host: Option<SetHost>,
    endpoint: Option<architect::iroh_link::iroh::Endpoint>,
) -> Started {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let sync = Arc::new(Mutex::new(TransportSync::new(me.clone())));
    let remote_chart = Arc::new(Mutex::new(HashMap::new()));
    let songs: Vec<Song> = bridges.iter().map(|(s, _)| s.clone()).collect();
    let started = Started {
        stop,
        sync: Arc::clone(&sync),
        remote_chart: Arc::clone(&remote_chart),
        songs: songs.clone(),
        me: me.clone(),
        name: name.clone(),
        set,
    };
    let mut bridges: Vec<Bridge> = bridges.into_iter().map(|(_, b)| b).collect();

    // Each song's engine changes, from its own slice of the event bus.
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel::<usize>();
    for (index, song) in songs.iter().enumerate() {
        let local_tx = local_tx.clone();
        let project = song.project.clone();
        tokio::spawn(async move {
            let Some(daw) = daw::rpc::Daw::try_get() else { return };
            let filter = daw_proto::event_bus::BusFilter {
                tracks: true,
                items: true,
                takes: true,
                markers: true,
                regions: true,
                tempo_map: true,
                project_guid: Some(project),
                ..daw_proto::event_bus::BusFilter::default()
            };
            let Ok(mut stream) = daw.events().subscribe(filter).await else { return };
            while let Ok(Some(_)) = stream.recv().await {
                if local_tx.send(index).is_err() {
                    break;
                }
            }
        });
    }

    // Each song's remote changes: any import into its doc.
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::unbounded_channel::<usize>();
    let subscriptions: Vec<_> = songs
        .iter()
        .enumerate()
        .map(|(index, song)| {
            let remote_tx = remote_tx.clone();
            song.doc.loro().subscribe_root(Arc::new(move |event| {
                if event.triggered_by == session::sync::loro::EventTriggerKind::Import {
                    let _ = remote_tx.send(index);
                }
            }))
        })
        .collect();

    tokio::spawn(async move {
        let _keep = (host.clone(), endpoint, subscriptions);
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        let mut out = Outbox::new(me.clone(), name);
        let mut roster = Roster::default();
        let mut seen: HashMap<String, session::sync::loro::LoroValue> = HashMap::default();
        // Sample-accurate following (daw-transport-sync): the leader's
        // stamped playhead, this engine locked to it.
        let mut lock = Lock::default();
        let key_of = |project: Option<String>| {
            project.and_then(|p| songs.iter().find(|s| s.project == p).map(|s| s.key.clone()))
        };
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                Some(first) = local_rx.recv() => {
                    // Let a burst (a drag, a rebuild) settle into one read
                    // per song it touched.
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let mut touched = BTreeSet::from([first]);
                    while let Ok(i) = local_rx.try_recv() {
                        touched.insert(i);
                    }
                    for i in touched {
                        if let Some(bridge) = bridges.get_mut(i)
                            && let Err(e) = bridge.local_changed().await
                        {
                            tracing::warn!(collab.error = %e, "collab: a local edit was not shared");
                        }
                    }
                }
                Some(first) = remote_rx.recv() => {
                    let mut touched = BTreeSet::from([first]);
                    while let Ok(i) = remote_rx.try_recv() {
                        touched.insert(i);
                    }
                    let showing = crate::open::current_song();
                    for i in touched {
                        let (Some(bridge), Some(song)) = (bridges.get_mut(i), songs.get(i)) else { continue };
                        match bridge.remote_changed().await {
                            Ok(changes) if !changes.is_empty() => {
                                for change in &changes {
                                    if let Change::ChartChanged(text) = change
                                        && let Ok(mut slot) = remote_chart.lock()
                                    {
                                        slot.insert(song.project.clone(), text.clone());
                                    }
                                }
                                if showing.as_deref() == Some(song.project.as_str()) {
                                    crate::studio::request_resync();
                                }
                            }
                            Ok(_) => {}
                            Err(e) => tracing::warn!(collab.error = %e, "collab: a remote edit did not apply"),
                        }
                    }
                }
                _ = tick.tick() => {
                    let now = crate::ghosts::now_ms();
                    let here = key_of(crate::open::current_song());
                    out.publish(presence.as_ref(), now, here.clone());
                    // Everyone else, into the roster.
                    let states = presence.states();
                    for (key, value) in &states {
                        if seen.get(key) != Some(value) {
                            roster.apply(&me, key, Some(value), now);
                        }
                    }
                    for key in seen.keys().filter(|k| !states.contains_key(*k)) {
                        roster.apply(&me, key, None, now);
                    }
                    seen = states;
                    // The tabs show everyone; the arrangement, the people
                    // on the song it shows.
                    // (By this machine's project for the song: the tabs
                    // know their songs by it.)
                    crate::ghosts::publish_everyone(
                        roster
                            .peers
                            .values()
                            .filter_map(|p| p.state.as_ref())
                            .filter_map(|s| {
                                let song = songs.iter().find(|x| s.song.as_ref() == Some(&x.key))?;
                                Some(crate::ghosts::Person {
                                    project: song.project.clone(),
                                    song: song.key.clone(),
                                    name: s.name.clone(),
                                    color: s.color,
                                })
                            })
                            .collect(),
                    );
                    let mut on_this_song = roster.clone();
                    on_this_song.peers.retain(|_, p| {
                        p.state.as_ref().is_some_and(|s| s.song.is_none() || s.song == here)
                    });
                    // The shared transport: whoever wrote it last, unless
                    // a press here is newer — then keep this engine on it.
                    let local = local_transport(&songs);
                    let (current, tick) = {
                        let Ok(mut sync) = sync.lock() else { continue };
                        if let Some(entry) = seen.get(transport::KEY).and_then(SharedTransport::decode) {
                            sync.remote(entry);
                        }
                        let tick = sync.tick(&local, now);
                        (sync.mode(), tick)
                    };
                    if let Some(entry) = tick.publish {
                        presence.set(transport::KEY, entry.encode());
                    }
                    let leader = sync.lock().ok().and_then(|s| s.current().map(|c| c.by.clone()));
                    let locked = lock.step(current, leader.as_deref(), &me, here.as_ref(), &seen, &clock, presence.as_ref());
                    for command in tick.commands {
                        // Locked to the leader's stamped playhead, the
                        // drift follower starts, stops and moves this
                        // engine (to the sample); the coarse commands are
                        // only for a song switch, or when it cannot lock.
                        if locked && !matches!(command, Command::SwitchSong(_)) {
                            continue;
                        }
                        obey(command, &songs);
                    }
                    crate::ghosts::publish(Some(on_this_song), 0.0, current == TransportMode::Independent);
                    if let Ok(mut live) = LIVE.lock()
                        && let Some(l) = live.as_mut()
                    {
                        l.status.peers = roster.peers.len();
                        l.status.shared_transport = current == TransportMode::Shared;
                    }
                }
            }
        }
        presence.delete(&presence::key(&me, presence::STATE));
        presence.delete(&presence::key(&me, presence::POINTER));
        presence.delete(&presence::key(&me, presence::PLAY));
    });
    started
}

/// This engine's lock to the leader of the shared transport.
///
/// Playing together, whoever pressed last leads: its engine plays on its
/// own, and every tick it stamps its playhead in the session's shared
/// clock ([`SyncPosition`] under its `sync` key). Everyone else locks to
/// that with daw-transport-sync's drift follower — a scheduled locate to
/// start or catch a jump, then a rate nudge of a fraction of a percent,
/// resampled, so the engines hold within a few samples and never click.
#[derive(Default)]
struct Lock {
    follower: daw_transport_sync::Follower,
    /// What this peer was doing last tick.
    role: Role,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Role {
    #[default]
    Apart,
    Leading,
    Following,
}

impl Lock {
    /// One tick. Returns whether this engine is locked to a leader (the
    /// coarse transport commands then stand down).
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        mode: TransportMode,
        leader: Option<&str>,
        me: &str,
        here: Option<&String>,
        seen: &HashMap<String, session::sync::loro::LoroValue>,
        clock: &SharedClock,
        presence: &dyn PresenceSink,
    ) -> bool {
        let backend = crate::open::current_song().and_then(|p| sync_backend(&p));
        let role = match (mode, leader) {
            (TransportMode::Shared, Some(l)) if l == me => Role::Leading,
            (TransportMode::Shared, Some(_)) => Role::Following,
            _ => Role::Apart,
        };
        if role != self.role {
            // A new role starts from rate 1 and nothing learned (the
            // mismatch is against a leader that may have changed).
            self.follower.reset();
            if let Some(b) = &backend {
                b.set_rate(1.0);
            }
            if self.role == Role::Leading {
                presence.delete(&transport::sync_key(me));
            }
            self.role = role;
        }
        let (Some(backend), Some(offset)) = (backend, clock.offset_micros()) else { return false };
        match role {
            Role::Apart => false,
            Role::Leading => {
                if let Some(snapshot) = backend.snapshot() {
                    let stamped = SyncPosition { song: here.cloned(), position: snapshot.position().shifted(offset) };
                    presence.set(&transport::sync_key(me), stamped.encode());
                }
                false
            }
            Role::Following => {
                let Some(lead) = leader
                    .and_then(|l| seen.get(&transport::sync_key(l)))
                    .and_then(SyncPosition::decode)
                else {
                    return false;
                };
                // Another song: the switch comes first (the coarse path).
                if lead.song.as_ref() != here {
                    return false;
                }
                let now = daw_transport_sync::clock::now_micros_f64();
                let correction = self.follower.tick(backend.as_ref(), &lead.position, offset, now);
                if !matches!(correction, daw_transport_sync::Correction::Hold) {
                    tracing::debug!(
                        sync.correction = ?correction,
                        sync.drift_us = self.follower.controller().last_drift().map(|d| d * 1e6),
                        sync.offset_us = offset,
                        "collab: following the leader"
                    );
                }
                true
            }
        }
    }
}

/// A song's sync backend (daw-transport-sync), by project — made once:
/// making one stands up the project's transport engine.
fn sync_backend(project: &str) -> Option<Arc<dyn daw_transport_sync::TransportBackend + Send + Sync>> {
    type Backends = HashMap<String, Arc<dyn daw_transport_sync::TransportBackend + Send + Sync>>;
    static BACKENDS: Mutex<Option<Backends>> = Mutex::new(None);
    let mut cache = BACKENDS.lock().ok()?;
    let backends = cache.get_or_insert_with(HashMap::new);
    if let Some(backend) = backends.get(project) {
        return Some(Arc::clone(backend));
    }
    // Remote: None — the per-buffer backend is the local engine's; locking a
    // follower to a remote transport over the facade is its own issue.
    let backend: Arc<dyn daw_transport_sync::TransportBackend + Send + Sync> = Arc::new(
        crate::open::with_local_engine(crate::open::LocalOnly::TransportSync, |daw| daw.sync_backend(project))
            .flatten()?,
    );
    backends.insert(project.to_owned(), Arc::clone(&backend));
    Some(backend)
}

/// Do what the shared transport asks of this engine — quietly: it is not
/// a press here, so it is not published back.
fn obey(command: Command, songs: &[Song]) {
    use crate::engine::{Move, transport_following};
    match command {
        Command::SwitchSong(key) => {
            if let Some(song) = songs.iter().find(|s| s.key == key)
                && let Ok(mut slot) = SONG_REQUEST.lock()
            {
                *slot = Some(song.project.clone());
            }
        }
        Command::Play { from } => transport_following(Move::PlayFrom, from),
        Command::Stop { at } => {
            transport_following(Move::Stop, at);
            transport_following(Move::Seek, at);
        }
        Command::Seek { to } => transport_following(Move::Seek, to),
    }
}

/// How far the playhead may stray from where the others think it is
/// before they are told again, seconds.
const JUMP: f64 = 0.25;

/// What this peer has told the others, so only changes go out.
struct Outbox {
    me: String,
    name: String,
    color: u32,
    state: Option<PeerState>,
    pointer: Throttle<Option<Pointer>>,
    pointer_sent: Option<Option<Pointer>>,
    play: Option<PlayState>,
    puppet_beat: Option<u64>,
}

impl Outbox {
    fn new(me: String, name: String) -> Self {
        let color = presence::color_for(&me);
        Self { me, name, color, state: None, pointer: Throttle::new(33.0), pointer_sent: None, play: None, puppet_beat: None }
    }

    fn publish(&mut self, sink: &dyn PresenceSink, now: f64, song: Option<String>) {
        if env_set("FTS_COLLAB_PUPPET") {
            self.puppet(sink, now, song);
            return;
        }
        let local = crate::ghosts::local();
        let edit = crate::cursor::current();
        let state = PeerState {
            name: self.name.clone(),
            color: self.color,
            song,
            view: "arrangement".into(),
            edit_cursor: edit.map(|e| e.at),
            time_selection: edit.and_then(|e| e.selection).map(|s| (s.start, s.end)),
            selected_tracks: local.selected_tracks,
            selected_items: local.selected_items,
            chart_caret: crate::chart_editor::local_caret(),
        };
        if self.state.as_ref() != Some(&state) {
            sink.set(&presence::key(&self.me, presence::STATE), state.encode());
            self.state = Some(state);
        }

        let pointer = self.pointer.offer(local.pointer, now).or_else(|| self.pointer.flush(now));
        if let Some(pointer) = pointer
            && self.pointer_sent.as_ref() != Some(&pointer)
        {
            let key = presence::key(&self.me, presence::POINTER);
            match &pointer {
                Some(pointer) => sink.set(&key, pointer.encode(now)),
                None => sink.delete(&key),
            }
            self.pointer_sent = Some(pointer);
        }

        // The play state goes out when it starts, stops or jumps — not as
        // it moves: the others extrapolate from the last one.
        if let Some(transport) = crate::engine::Transport::shared() {
            let (at, playing) = transport.read();
            let changed = self.play.is_none_or(|last| {
                last.playing != playing || (last.position_at(now) - at).abs() > JUMP
            });
            if changed {
                let play = PlayState { playing, position: at, at_ms: now, rate: 1.0 };
                sink.set(&presence::key(&self.me, presence::PLAY), play.encode());
                self.play = Some(play);
            }
        }
    }

    /// A peer that moves on its own: the pointer sweeping across the first
    /// minute of the song over the first few tracks, an edit cursor, a
    /// time selection and an item selected, playing. For the two-window
    /// test (`FTS_COLLAB_PUPPET=1` on one of them) and a demo's "someone
    /// else is here" — never set in normal use.
    #[allow(clippy::as_conversions, clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a small, positive time
    fn puppet(&mut self, sink: &dyn PresenceSink, now: f64, song: Option<String>) {
        let t = (now / 1000.0) % 8.0;
        let at = 4.0 + t * 7.0;
        let tracks: Vec<String> = crate::ghosts::local_rows();
        let track = tracks.get((t as usize / 2) % tracks.len().max(1)).cloned();
        if self.state.is_none() {
            let state = PeerState {
                name: self.name.clone(),
                color: self.color,
                song,
                view: "arrangement".into(),
                edit_cursor: Some(24.0),
                time_selection: Some((32.0, 48.0)),
                selected_tracks: tracks.iter().take(1).cloned().collect(),
                selected_items: crate::ghosts::local_item_guids().into_iter().take(2).collect(),
                chart_caret: None,
            };
            sink.set(&presence::key(&self.me, presence::STATE), state.encode());
            self.state = Some(state);
            let play = PlayState { playing: true, position: 10.0, at_ms: now, rate: 1.0 };
            sink.set(&presence::key(&self.me, presence::PLAY), play.encode());
        }
        // And an edit, through this engine like any other: every few
        // seconds the drums are muted or unmuted, which the others see
        // arrive through the doc.
        let beat = (now / 3000.0) as u64;
        if self.puppet_beat != Some(beat) {
            // Every fourth beat, everyone plays together from 20 s; the
            // beat after, apart again.
            if self.puppet_beat.is_some() && env_set("FTS_COLLAB_PUPPET_TRANSPORT") {
                let together = beat % 4 == 0;
                let press = SharedTransport {
                    mode: if together { TransportMode::Shared } else { TransportMode::Independent },
                    playing: together,
                    position: 20.0,
                    at_ms: now,
                    song: None,
                    seq: next_seq(),
                    by: self.me.clone(),
                };
                if together || beat % 4 == 1 {
                    sink.set(transport::KEY, press.encode());
                }
            }
            self.puppet_beat = Some(beat);
            // A caret in the chart, a line further down each time, as a
            // stable position the others resolve against their own copy.
            if let (Some(state), Some((doc, _, _))) = (self.state.as_mut(), chart_context()) {
                let chart = doc.read().chart;
                let starts: Vec<usize> = std::iter::once(0)
                    .chain(chart.match_indices('\n').map(|(i, _)| i + 1))
                    .filter(|i| *i < chart.len())
                    .collect();
                if let Some(at) = starts.get((beat as usize) % starts.len().max(1)) {
                    let at = chart[..*at].chars().count();
                    let end = at + 4;
                    state.chart_caret = doc.chart_cursor(at).zip(doc.chart_cursor(end));
                    sink.set(&presence::key(&self.me, presence::STATE), state.encode());
                }
            }
            if let Some(guid) = tracks.iter().find(|g| {
                crate::ghosts::local_track_name(g).is_some_and(|n| n.starts_with("Drums"))
            }) {
                let guid = guid.clone();
                tokio::spawn(async move {
                    let Some(daw) = daw::rpc::Daw::try_get() else { return };
                    let Ok(project) = daw.current_project().await else { return };
                    if let Ok(Some(track)) = project.tracks().by_guid(&guid).await {
                        let _ = track.toggle_mute().await;
                    }
                });
            }
        }
        // Three seconds on the lanes, three over the chart's page, three
        // over the lyrics: every kind of place a pointer can be.
        let place = match (now / 3000.0) as u64 % 3 {
            0 => Pointer::Timeline { at, track, y: 0.5 },
            1 => Pointer::Region { region: "chart".into(), x: 0.2 + 0.6 * (t % 1.0), y: 0.3 + 0.05 * t },
            _ => Pointer::Region { region: "lyrics".into(), x: 0.15 + 0.7 * (t % 1.0), y: 0.5 },
        };
        if let Some(Some(pointer)) = self.pointer.offer(Some(place), now) {
            sink.set(&presence::key(&self.me, presence::POINTER), pointer.encode(now));
        }
    }
}

#[cfg(test)]
mod task_set_tests {
    use super::TaskSet;

    #[test]
    fn a_share_link_is_joined_on_its_guest_lane() {
        let set = TaskSet::parse("share:https://task.example/org/days-to-praise/share/abc123?pw=x");
        assert_eq!(set.url, "wss://task.example/org/days-to-praise/share/abc123/vox");
        assert_eq!(set.token, None);
        assert_eq!(set.setlist, "", "the link's own set");
    }
}
