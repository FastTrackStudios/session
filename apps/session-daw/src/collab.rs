//! Collaboration: share the open song with others, or join someone's.
//!
//! One task per session owns everything that must not race: the bridge
//! between this engine and the shared doc, this peer's presence, and the
//! shared transport's follower. It wakes on three things —
//!
//! - the engine changed (the daw event bus): read it back, write the doc;
//! - the doc changed remotely (a Loro import): make the engine match, and
//!   ask the arrangement to read it back;
//! - a ~30 Hz tick: publish what this peer is doing (throttled, and only
//!   what changed), fold everyone else's presence into the roster the
//!   arrangement draws, and keep the engine on the shared transport.
//!
//! Hosting serves the session over iroh; joining dials it. The ticket
//! (`fts-session:<endpoint id>/<session uuid>`) is what you hand someone.
//! A joiner opens the same song first — the media is read from its own
//! copy — and its engine is then made to match the host's doc.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use session::sync::engine::MediaRoot;
use session::sync::net::{CollabHost, CollabPeer, PresenceSink, session_id};
use session::sync::presence::{self, PeerState, PlayState, Pointer, Roster, Throttle};
use session::sync::transport::{self, Command, Follower, LocalTransport, SharedTransport, TransportMode};
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
}

struct Live {
    status: Status,
    stop: tokio::sync::watch::Sender<bool>,
    mode: Arc<Mutex<TransportMode>>,
    /// The chart text as the doc last had it, when a remote edit changed
    /// it — the chart editor takes it from here.
    remote_chart: Arc<Mutex<Option<String>>>,
    doc: SessionDoc,
    presence: Arc<dyn PresenceSink>,
    me: String,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

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
pub fn host_in_background(name: String, chart_file: Option<std::path::PathBuf>) {
    std::thread::spawn(move || {
        let _ = host(name, chart_file);
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
    LIVE.lock().ok()?.as_ref().map(|l| l.status.clone())
}

/// A chart the others changed since the editor last looked.
#[must_use]
pub fn take_remote_chart() -> Option<String> {
    let live = LIVE.lock().ok()?;
    live.as_ref()?.remote_chart.lock().ok()?.take()
}

/// The shared doc and presence, for the chart editor's carets.
#[must_use]
pub fn chart_context() -> Option<(SessionDoc, Arc<dyn PresenceSink>, String)> {
    let live = LIVE.lock().ok()?;
    let l = live.as_ref()?;
    Some((l.doc.clone(), Arc::clone(&l.presence), l.me.clone()))
}

/// The chart was edited here.
pub fn local_chart(text: &str) {
    let Ok(live) = LIVE.lock() else { return };
    let Some(l) = live.as_ref() else { return };
    // The bridge owns doc writes; a chart edit is recorded straight into
    // the text, which the next local reconcile leaves alone.
    let mut model = l.doc.read();
    if model.chart != text {
        model.chart = text.to_string();
        if let Err(e) = l.doc.write(&model, ORIGIN_LOCAL) {
            tracing::warn!(collab.chart_error = %e, "collab: the chart edit was not recorded");
        }
    }
}

/// Switch between everyone playing on their own and one transport.
pub fn set_shared_transport(shared: bool) {
    let Ok(mut live) = LIVE.lock() else { return };
    let Some(l) = live.as_mut() else { return };
    let mode = if shared { TransportMode::Shared } else { TransportMode::Independent };
    if let Ok(mut m) = l.mode.lock() {
        *m = mode;
    }
    l.status.shared_transport = shared;
    let (at, playing) = crate::engine::Transport::shared().map_or((0.0, false), |t| t.read());
    let state = SharedTransport {
        mode,
        playing,
        position: at,
        at_ms: crate::ghosts::now_ms(),
        song: crate::open::current_song(),
        seq: next_seq(),
        by: l.me.clone(),
    };
    l.presence.set(transport::KEY, state.encode());
}

/// Leave the session (or stop hosting it).
pub fn leave() {
    if let Ok(mut live) = LIVE.lock()
        && let Some(l) = live.take()
    {
        let _ = l.stop.send(true);
    }
    crate::ghosts::publish(None, 0.0, false);
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

/// Share the open song. Returns the ticket to hand others.
///
/// # Errors
/// When no song is open, the engine cannot be read, or iroh cannot bind.
pub fn host(name: String, chart_file: Option<std::path::PathBuf>) -> eyre::Result<String> {
    record(host_inner(name, chart_file))
}

fn host_inner(name: String, chart_file: Option<std::path::PathBuf>) -> eyre::Result<String> {
    leave();
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let chart = chart_file.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let (ticket, live) = runtime.block_on(async move {
        let (project, media, song) = current_project().await?;
        let id = session_id(&song);
        let bridge = Bridge::host(project, media, SessionDoc::new(), chart).await?;
        let host = CollabHost::new(id, bridge.doc());
        let endpoint = architect::iroh_link::bind_endpoint(secret_key())
            .await
            .map_err(|e| eyre::eyre!("iroh: {e}"))?;
        let ticket = format!("fts-session:{}/{id}", endpoint.id());
        let router = host.mount(architect::LayerRouter::new());
        let serving = endpoint.clone();
        tokio::spawn(async move { architect::iroh_link::serve_router(&serving, router).await });

        let doc = bridge.doc().clone();
        let me = format!("host-{}", &endpoint.id().to_string()[..8]);
        let presence: Arc<dyn PresenceSink> = Arc::new(host.clone());
        let live = start(bridge, Arc::clone(&presence), me, name, Some(host), Some(endpoint));
        Ok::<_, eyre::Report>((ticket.clone(), live.with(ticket, true, doc, presence)))
    })?;
    if let Ok(mut slot) = LIVE.lock() {
        *slot = Some(live);
    }
    tracing::info!(collab.role = "host", "collab: sharing the song");
    Ok(ticket)
}

/// Join a session from its ticket. The same song must be open here.
///
/// # Errors
/// On a malformed ticket, a different song open, or a failed dial.
pub fn join(ticket: &str, name: String) -> eyre::Result<()> {
    record(join_inner(ticket, name))
}

fn join_inner(ticket: &str, name: String) -> eyre::Result<()> {
    leave();
    let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("the engine is not up"))?;
    let (endpoint_id, id) = parse_ticket(ticket)?;
    let live = runtime.block_on(async move {
        let (project, media, song) = current_project().await?;
        if session_id(&song) != id {
            eyre::bail!("open the same song first — this ticket is for another one");
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

        let mut peer = CollabPeer::new(id);
        let doc = peer.doc().clone();
        let presence: Arc<dyn PresenceSink> = Arc::new(peer.presence().clone());
        tokio::spawn(async move {
            if let Err(e) = peer.run(&sync, &presence_client).await {
                tracing::warn!(collab.error = %e, "collab: the session connection ended");
            }
        });
        // The host's session arrives first; only then may the bridge make
        // this engine match it.
        for _ in 0..200 {
            if doc.has_session() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if !doc.has_session() {
            eyre::bail!("the host did not send the session");
        }
        let bridge = Bridge::join(project, media, doc.clone()).await?;
        crate::studio::request_resync();
        let me = format!("peer-{}", &endpoint.id().to_string()[..8]);
        let live = start(bridge, Arc::clone(&presence), me, name, None, Some(endpoint));
        Ok::<_, eyre::Report>(live.with(ticket.to_string(), false, doc, presence))
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

/// The open song: its engine project, where its media is, and the name
/// everyone who has it knows it by.
///
/// The name, not the project guid: the engine mints a guid per open, so
/// two copies of one song never share it. The file's stem is the same on
/// every machine (`Washed`); once songs live in the Task library this is
/// the library's song id.
async fn current_project() -> eyre::Result<(daw_control::Project, MediaRoot, String)> {
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("the daw facade is not up"))?;
    let project = daw.current_project().await?;
    let path = std::path::PathBuf::from(project.info().await?.path);
    let folder = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let song = path
        .file_stem()
        .map_or_else(|| project.guid().to_owned(), |s| s.to_string_lossy().into_owned());
    Ok((project, MediaRoot(folder), song))
}

/// The half-built `Live` [`start`] returns; finished by the caller.
struct Started {
    stop: tokio::sync::watch::Sender<bool>,
    mode: Arc<Mutex<TransportMode>>,
    remote_chart: Arc<Mutex<Option<String>>>,
    me: String,
}

impl Started {
    fn with(self, ticket: String, hosting: bool, doc: SessionDoc, presence: Arc<dyn PresenceSink>) -> Live {
        Live {
            status: Status { hosting, ticket, peers: 0, shared_transport: false },
            stop: self.stop,
            mode: self.mode,
            remote_chart: self.remote_chart,
            doc,
            presence,
            me: self.me,
        }
    }
}

/// Spawn the session's task. `host` and `endpoint` are kept alive by it.
fn start(
    mut bridge: Bridge,
    presence: Arc<dyn PresenceSink>,
    me: String,
    name: String,
    host: Option<CollabHost>,
    endpoint: Option<architect::iroh_link::iroh::Endpoint>,
) -> Started {
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let mode = Arc::new(Mutex::new(TransportMode::Independent));
    let remote_chart = Arc::new(Mutex::new(None));
    let started = Started {
        stop,
        mode: Arc::clone(&mode),
        remote_chart: Arc::clone(&remote_chart),
        me: me.clone(),
    };

    // Engine changes, from the event bus.
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    tokio::spawn(async move {
        let Some(daw) = daw::rpc::Daw::try_get() else { return };
        let filter = daw_proto::event_bus::BusFilter {
            tracks: true,
            items: true,
            takes: true,
            markers: true,
            regions: true,
            tempo_map: true,
            ..daw_proto::event_bus::BusFilter::default()
        };
        let Ok(mut stream) = daw.events().subscribe(filter).await else { return };
        while let Ok(Some(_)) = stream.recv().await {
            if local_tx.send(()).is_err() {
                break;
            }
        }
    });

    // Remote doc changes: any import.
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let subscription = bridge.doc().loro().subscribe_root(Arc::new(move |event| {
        if event.triggered_by == session::sync::loro::EventTriggerKind::Import {
            let _ = remote_tx.send(());
        }
    }));

    tokio::spawn(async move {
        let _keep = (host.clone(), endpoint, subscription);
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        let mut out = Outbox::new(me.clone(), name);
        let mut roster = Roster::default();
        let mut seen: std::collections::HashMap<String, session::sync::loro::LoroValue> = Default::default();
        let mut follower = Follower::default();
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                Some(()) = local_rx.recv() => {
                    // Let a burst (a drag, a rebuild) settle into one read.
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    while local_rx.try_recv().is_ok() {}
                    if let Err(e) = bridge.local_changed().await {
                        tracing::warn!(collab.error = %e, "collab: a local edit was not shared");
                    }
                }
                Some(()) = remote_rx.recv() => {
                    while remote_rx.try_recv().is_ok() {}
                    match bridge.remote_changed().await {
                        Ok(changes) if !changes.is_empty() => {
                            for change in &changes {
                                if let Change::ChartChanged(text) = change
                                    && let Ok(mut slot) = remote_chart.lock()
                                {
                                    *slot = Some(text.clone());
                                }
                            }
                            crate::studio::request_resync();
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(collab.error = %e, "collab: a remote edit did not apply"),
                    }
                }
                _ = tick.tick() => {
                    let now = crate::ghosts::now_ms();
                    out.publish(presence.as_ref(), now);
                    // Everyone else, into the roster the arrangement draws.
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
                    let mode = mode.lock().map_or(TransportMode::Independent, |m| *m);
                    crate::ghosts::publish(Some(roster.clone()), 0.0, mode == TransportMode::Independent);
                    if let Ok(mut live) = LIVE.lock()
                        && let Some(l) = live.as_mut()
                    {
                        l.status.peers = roster.peers.len();
                    }
                    if mode == TransportMode::Shared
                        && let Some(shared) = seen.get(transport::KEY).and_then(SharedTransport::decode)
                    {
                        follow(&mut follower, &shared, now);
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

/// Keep this engine on the shared transport.
fn follow(follower: &mut Follower, shared: &SharedTransport, now_ms: f64) {
    let Some(transport) = crate::engine::Transport::shared() else { return };
    let (at, playing) = transport.read();
    let local = LocalTransport { playing, position: at, song: crate::open::current_song() };
    for command in follower.step(shared, &local, now_ms) {
        match command {
            // Switching songs is the setlist's job; a lone song ignores it.
            Command::SwitchSong(_) => {}
            Command::Play { from } => crate::engine::transport(crate::engine::Move::PlayFrom, from),
            Command::Stop { at } => {
                crate::engine::transport(crate::engine::Move::Stop, at);
                crate::engine::transport(crate::engine::Move::Seek, at);
            }
            Command::Seek { to } => crate::engine::transport(crate::engine::Move::Seek, to),
        }
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
    pointer: Throttle<Option<(f64, Option<String>)>>,
    pointer_sent: Option<Option<(f64, Option<String>)>>,
    play: Option<PlayState>,
    puppet_beat: Option<u64>,
}

impl Outbox {
    fn new(me: String, name: String) -> Self {
        let color = presence::color_for(&me);
        Self { me, name, color, state: None, pointer: Throttle::new(33.0), pointer_sent: None, play: None, puppet_beat: None }
    }

    fn publish(&mut self, sink: &dyn PresenceSink, now: f64) {
        if std::env::var_os("FTS_COLLAB_PUPPET").is_some() {
            self.puppet(sink, now);
            return;
        }
        let local = crate::ghosts::local();
        let edit = crate::cursor::current();
        let state = PeerState {
            name: self.name.clone(),
            color: self.color,
            song: crate::open::current_song(),
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
                Some((at, track)) => sink.set(
                    &key,
                    Pointer::Timeline { at: *at, track: track.clone() }.encode(now),
                ),
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
    fn puppet(&mut self, sink: &dyn PresenceSink, now: f64) {
        let t = (now / 1000.0) % 8.0;
        let at = 4.0 + t * 7.0;
        let tracks: Vec<String> = crate::ghosts::local_rows();
        let track = tracks.get((t as usize / 2) % tracks.len().max(1)).cloned();
        if self.state.is_none() {
            let state = PeerState {
                name: self.name.clone(),
                color: self.color,
                song: crate::open::current_song(),
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
            self.puppet_beat = Some(beat);
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
        if let Some(pointer) = self.pointer.offer(Some((at, track)), now)
            && let Some((at, track)) = pointer
        {
            sink.set(
                &presence::key(&self.me, presence::POINTER),
                Pointer::Timeline { at, track }.encode(now),
            );
        }
    }
}
