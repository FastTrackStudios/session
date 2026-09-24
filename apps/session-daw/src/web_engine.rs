//! The engine in a browser: daw-standalone in-process, the session opened
//! from text.
//!
//! The native app opens a song from disk, stands up the `daw` facade on a
//! tokio runtime, and reads the session back ([`crate::open`],
//! `StudioSession::open`). A page has no disk and no threads, so it opens
//! the song the same one way from a memory folder
//! ([`crate::open_core::open_song_in`] over [`crate::folder::Memory`],
//! mirrored from a share link by [`crate::song_stream`]) and awaits
//! everything instead of blocking: `build_in_process_daw`,
//! `daw::init_from_parts`, then the same read-back and row plan
//! ([`crate::studio::Planner`]). The engine clients
//! ([`crate::engine::Applier`], [`crate::engine::Transport`]) have web
//! implementations that find the facade this installs.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::studio::{Planner, StudioSession};

/// The scene the web demo lays the arrangement out by — the desktop app's.
const SCENE: &str = "drum-mixing";

/// The engine, as the page's panels reach it.
#[derive(Clone)]
pub struct EngineRef {
    applier: Rc<Option<crate::engine::Applier>>,
}

impl PartialEq for EngineRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.applier, &other.applier)
    }
}

impl EngineRef {
    /// Carry out an edit. Dropped (logged) with no engine.
    pub fn send(&self, edit: crate::engine::Edit) {
        match self.applier.as_ref() {
            Some(applier) => applier.send(edit),
            None => tracing::debug!(?edit, "no engine to carry out the edit"),
        }
    }

    /// Where the play cursor is, in seconds.
    #[must_use]
    pub fn position(&self) -> f64 {
        crate::engine::Transport::shared().map_or(0.0, |t| t.read().0)
    }
}

/// What the page opens.
#[derive(Clone, Debug, PartialEq)]
pub enum WebSource {
    /// A song shared by a Task share link: its files mirrored into memory,
    /// its proxies streamed by range, what will be heard first first.
    Shared { link: String },
    /// A copy bundled with the page: its project and chart by URL; silent.
    Bundled {
        name: String,
        rpp_url: String,
        chart_url: Option<String>,
    },
    /// A live share link to a set Task keeps (`live-proto`): the page joins
    /// it — its first song opened from that song's files link, everyone in
    /// the set in one session with it (`collab::join_task`), by `name`.
    Live { link: String, name: String },
}

/// Where opening the page has got to: what its loading screen says.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Reaching Task and joining the set — `retry` says why the last try
    /// failed, while it keeps trying.
    Joining { retry: Option<String> },
    /// Bringing the first song's files in.
    Fetching {
        title: String,
        retry: Option<String>,
    },
    /// Opening it into the engine.
    Opening { title: String },
}

/// The set's other songs, as each opens after the first is on screen.
pub enum Arrival {
    Song(crate::setlist::Song),
    /// A song that could not be opened: its tab goes.
    Failed(String),
}

/// A song the page will open, and where it comes from.
struct Wanted {
    title: Option<String>,
    from: Origin,
}

enum Origin {
    /// A Task share link to the song's session folder.
    Share(String),
    /// A copy bundled with the page.
    Bundled {
        name: String,
        rpp_url: String,
        chart_url: Option<String>,
    },
}

/// A song fetched for opening: the folder it sits in, its project's path
/// there, and — shared — its proxies to stream.
struct Fetched {
    folder: Arc<dyn crate::folder::Folder>,
    path: std::path::PathBuf,
    streamed: Option<crate::song_stream::StreamedSong>,
}

/// A song opened into the page's engine, before it is read back.
struct Loaded {
    title: String,
    project: String,
    rpp_text: String,
    chart_text: Option<String>,
    streamed: Option<crate::song_stream::StreamedSong>,
}

/// What the page keeps of each song it opened: its streamed media, started
/// the first time the song is picked ([`switch_song`]) — a set of seven
/// streams only the song on screen.
struct WebSong {
    streamed: Option<Rc<crate::song_stream::StreamedSong>>,
    previews: crate::midi::Previews,
    started: std::cell::Cell<bool>,
}

struct Web {
    standalone: daw_standalone::sync::Standalone,
    songs: HashMap<String, WebSong>,
}

thread_local! {
    static WEB: std::cell::RefCell<Option<Web>> = const { std::cell::RefCell::new(None) };
    static ARRIVALS: std::cell::RefCell<Option<tokio::sync::mpsc::UnboundedReceiver<Arrival>>> =
        const { std::cell::RefCell::new(None) };
}

/// The set's other songs as they open — once, for the view that shows
/// them (see [`open`]).
pub fn take_arrivals() -> Option<tokio::sync::mpsc::UnboundedReceiver<Arrival>> {
    ARRIVALS.with(|a| a.borrow_mut().take())
}

/// Open the page the one way songs open (`open_core::open_song_in`, from
/// a memory folder), first song first: a live set is joined (and joined
/// again until Task answers — it may be restarting), its first song
/// fetched, opened, read back and returned to be shown, and the set's
/// other songs opened into the same engine behind it, one by one
/// ([`take_arrivals`]) — each joining the live session as it does.
/// `progress` hears each step for the loading screen. `guide` is a share
/// link to the guide sample library (as Ogg) the click, count and cue
/// tracks play from; without it they play synthesized ticks and beeps.
///
/// # Errors
///
/// The first song could not be opened or read back, or the facade did
/// not come up.
pub async fn open(
    source: &WebSource,
    guide: Option<&str>,
    progress: impl Fn(Progress),
) -> eyre::Result<(EngineRef, crate::setlist::Setlist)> {
    // What to open: a live set's songs (joined first, for their files
    // links), or the one song a link or the page names.
    let mut live: Option<crate::collab::TaskSet> = None;
    let mut wanted: Vec<Wanted> = match source {
        WebSource::Live { link, .. } => {
            let set = crate::collab::TaskSet::parse(&format!("share:{link}"));
            progress(Progress::Joining { retry: None });
            let joined = until_ok(
                None,
                || join_set(&set.url),
                |why| {
                    progress(Progress::Joining { retry: Some(why) });
                },
            )
            .await?;
            tracing::info!(live.setlist = %joined.title, live.songs = joined.songs.len(), live.epoch = joined.epoch, "web: joined a live set");
            watch_set(set.url.clone(), joined.setlist.clone(), joined.epoch);
            let wanted = joined
                .songs
                .iter()
                .filter_map(|s| {
                    Some(Wanted {
                        title: Some(s.title.clone()),
                        from: Origin::Share(s.files.clone()?),
                    })
                })
                .collect();
            live = Some(crate::collab::TaskSet {
                setlist: joined.setlist,
                ..set
            });
            wanted
        }
        WebSource::Shared { link } => vec![Wanted {
            title: None,
            from: Origin::Share(link.clone()),
        }],
        WebSource::Bundled {
            name,
            rpp_url,
            chart_url,
        } => vec![Wanted {
            title: None,
            from: Origin::Bundled {
                name: name.clone(),
                rpp_url: rpp_url.clone(),
                chart_url: chart_url.clone(),
            },
        }],
    };
    if wanted.is_empty() {
        eyre::bail!("the set has no songs with files to open");
    }
    let rest = wanted.split_off(1);
    let first = wanted.remove(0);
    let first_title = first.title.clone().unwrap_or_default();

    progress(Progress::Fetching {
        title: first_title.clone(),
        retry: None,
    });
    let fetched = until_ok(
        Some(8),
        || fetch_song(&first),
        |why| {
            progress(Progress::Fetching {
                title: first_title.clone(),
                retry: Some(why),
            });
        },
    )
    .await?;
    progress(Progress::Opening {
        title: first_title.clone(),
    });
    let standalone = daw_standalone::sync::Standalone::new();
    // The guide instrument, before the guide tracks it plays on are made.
    let library = match guide {
        Some(base) => guide_library(base).await,
        None => HashMap::new(),
    };
    crate::guide_instrument::install(
        &standalone,
        crate::guide_instrument::Library::Files {
            files: Arc::new(library),
            ext: "ogg",
        },
    );
    let loaded = open_fetched(&standalone, first.title, fetched)?;
    let bundle = daw_standalone::bootstrap::build_in_process_daw(standalone.clone())
        .await
        .map_err(|e| eyre::eyre!("build_in_process_daw: {e:?}"))?;
    daw::init_from_parts(bundle.daw.clone());
    // The facade and the engine live as long as the page.
    std::mem::forget(bundle);
    let (song, kept) = read_back(loaded).await?;
    let project = song.project.clone();
    WEB.with(|web| {
        *web.borrow_mut() = Some(Web {
            standalone: standalone.clone(),
            songs: HashMap::from([(project.clone(), kept)]),
        });
    });
    crate::web_audio::install(standalone.clone(), &project);
    switch_song(&project);

    // In the live set with everyone else (the facade is up: the session's
    // bridges read and drive the engine through it). The songs that open
    // after this join it as they do (`collab::song_opened`).
    if let (Some(set), WebSource::Live { name, .. }) = (live, source) {
        if let Err(e) = crate::collab::join_task(&set, name.clone()) {
            tracing::warn!(collab.error = %e, "web: could not join the live set");
        }
    }

    // The set's other songs, behind the one on screen, in set order.
    let mut setlist = crate::setlist::Setlist::of(vec![song]);
    setlist.pending = rest
        .iter()
        .map(|w| w.title.clone().unwrap_or_default())
        .collect();
    let (arrive, arrivals) = tokio::sync::mpsc::unbounded_channel();
    ARRIVALS.with(|a| *a.borrow_mut() = Some(arrivals));
    wasm_bindgen_futures::spawn_local(async move {
        for want in rest {
            let title = want.title.clone().unwrap_or_default();
            let opened = async {
                let fetched = until_ok(Some(6), || fetch_song(&want), |_| {}).await?;
                read_back(open_fetched(&standalone, want.title.clone(), fetched)?).await
            }
            .await;
            let arrival = match opened {
                Ok((song, kept)) => {
                    let project = song.project.clone();
                    WEB.with(|web| {
                        if let Some(web) = web.borrow_mut().as_mut() {
                            web.songs.insert(project.clone(), kept);
                        }
                    });
                    crate::collab::song_opened(&project);
                    Arrival::Song(song)
                }
                Err(e) => {
                    tracing::warn!(song.title = %title, error = %e, "web: a song did not open; the set goes on without it");
                    Arrival::Failed(title)
                }
            };
            if arrive.send(arrival).is_err() {
                break;
            }
        }
    });

    let engine = EngineRef {
        applier: Rc::new(crate::engine::Applier::start()),
    };
    Ok((engine, setlist))
}

/// Try `attempt` until it works — or `limit` times — waiting between
/// tries from a second, doubling to fifteen; `failed` hears why each one
/// did not, and when the next is.
async fn until_ok<T, Fut>(
    limit: Option<u32>,
    mut attempt: impl FnMut() -> Fut,
    mut failed: impl FnMut(String),
) -> eyre::Result<T>
where
    Fut: std::future::Future<Output = eyre::Result<T>>,
{
    let mut wait = std::time::Duration::from_secs(1);
    let mut tries = 0u32;
    loop {
        match attempt().await {
            Ok(done) => return Ok(done),
            Err(e) => {
                tries += 1;
                if limit.is_some_and(|limit| tries >= limit) {
                    return Err(e);
                }
                tracing::debug!(error = %e, tries, "web: trying again");
                failed(format!("{e} — trying again in {} s", wait.as_secs()));
                architect::platform::sleep(wait).await;
                wait = (wait * 2).min(std::time::Duration::from_secs(15));
            }
        }
    }
}

/// Join the set a live link opens, on the Task at `url`.
async fn join_set(url: &str) -> eyre::Result<live_proto::LiveSet> {
    use live_proto::LiveSessionsClient;
    let lane: LiveSessionsClient = task_dial::establish_at(url, None)
        .await
        .map_err(|e| eyre::eyre!("Task is not answering ({e})"))?;
    lane.join(String::new())
        .await
        .map_err(|e| eyre::eyre!("the set could not be joined ({e:?})"))
}

/// A song's files, fetched: mirrored from its share link into memory (its
/// proxies stream later, by range), or the page's bundled copy.
async fn fetch_song(want: &Wanted) -> eyre::Result<Fetched> {
    use crate::folder::Memory;
    use crate::song_stream::{Keep, ShareSource, mirror};
    match &want.from {
        Origin::Share(link) => {
            let song = mirror(
                Arc::new(ShareSource::new(link)?),
                Keep::Memory(Memory::new()),
            )
            .await?;
            Ok(Fetched {
                folder: Arc::clone(&song.folder),
                path: song.project.clone(),
                streamed: Some(song),
            })
        }
        Origin::Bundled {
            name,
            rpp_url,
            chart_url,
        } => {
            let memory = Memory::new();
            let dir = std::path::Path::new("/bundled");
            let rpp = dir.join(format!("{name}.RPP"));
            memory.insert(
                &rpp,
                fetch_bytes(rpp_url).await.map_err(|e| eyre::eyre!(e))?,
            );
            if let Some(url) = chart_url {
                memory.insert(
                    dir.join(format!("{name}.kf")),
                    fetch_bytes(url).await.map_err(|e| eyre::eyre!(e))?,
                );
            }
            Ok(Fetched {
                folder: Arc::new(memory),
                path: rpp,
                streamed: None,
            })
        }
    }
}

/// Open a fetched song into `standalone`: prepared (its `.session` when it
/// carries one, else organized, built from its chart, given its guide) —
/// it does not become current.
fn open_fetched(
    standalone: &daw_standalone::sync::Standalone,
    title: Option<String>,
    song: Fetched,
) -> eyre::Result<Loaded> {
    let prepare = crate::prepare::Prepare::for_song_in(song.folder.as_ref(), &song.path);
    let (opened, plan) = crate::open_core::open_song_in(
        song.folder.as_ref(),
        standalone,
        &song.path,
        &prepare,
        crate::open_core::Media::Deferred,
    )?;
    let rpp_text = crate::open_core::project_text_in(song.folder.as_ref(), &plan.open)?.text;
    let chart_text = prepare
        .chart
        .as_ref()
        .and_then(|p| song.folder.read_to_string(p).ok());
    // The chart a live set's doc for this song is seeded with.
    if let Some(chart) = &chart_text {
        crate::collab::remember_chart(&plan.open, chart);
    }
    Ok(Loaded {
        title: title.unwrap_or_else(|| opened.name.clone()),
        project: opened.project_guid.clone(),
        rpp_text,
        chart_text,
        streamed: song.streamed,
    })
}

/// A song the page opened, read back as the studio lays it out — by its
/// project, so a song opening behind the one on screen does not move it.
async fn read_back(song: Loaded) -> eyre::Result<(crate::setlist::Song, WebSong)> {
    // Say which step fails: `fetch` answers only yes or no.
    let facade =
        daw_control::Daw::try_get().ok_or_else(|| eyre::eyre!("the daw facade is not up"))?;
    let project = facade
        .project(song.project.clone())
        .await
        .map_err(|e| eyre::eyre!("{} is not in the engine: {e}", song.title))?;
    let raw = daw_ui::studio::project::fetch_of(project.clone())
        .await
        .ok_or_else(|| eyre::eyre!("could not read {} back", song.title))?;
    let planner = Planner {
        raw: Arc::new(raw),
        scene: Some(SCENE),
        kinds: Arc::new(crate::plan::Kinds::from_text(&song.rpp_text)),
    };
    let (studio, rows) = planner.plan(&planner.raw);
    let previews = crate::midi::Previews::default();
    previews
        .fill_in(
            &project,
            studio
                .0
                .items
                .values()
                .flatten()
                .filter(|item| studio.0.is_midi(&item.guid))
                .map(|item| (item.guid.clone(), item.length.as_seconds()))
                .collect(),
        )
        .await;
    let chart = song.chart_text.as_deref().and_then(|text| {
        keyflow::parse(text)
            .inspect_err(|e| tracing::error!(error = %e, "chart: could not parse"))
            .ok()
            .map(Arc::new)
    });
    let session = StudioSession {
        project: studio,
        rows,
        previews: previews.clone(),
        chart,
        chart_file: None,
        planner,
    };
    let kept = WebSong {
        streamed: song.streamed.map(Rc::new),
        previews,
        started: std::cell::Cell::new(false),
    };
    Ok((
        crate::setlist::Song::of(song.title, song.project, session),
        kept,
    ))
}

/// Make `project` the song on screen — what picking a setlist tab does on
/// a page (`crate::open::switch_song`): the engine's current project, the
/// audio moved to it, and — the first time — its proxies streaming and its
/// waveforms loading.
pub fn switch_song(project: &str) {
    WEB.with(|web| {
        let web = web.borrow();
        let Some(web) = web.as_ref() else { return };
        web.standalone.set_current_project(project);
        crate::open::set_current_song(project);
        crate::web_audio::switch(project);
        let Some(song) = web.songs.get(project) else {
            return;
        };
        if song.started.replace(true) {
            return;
        }
        let Some(streamed) = song.streamed.clone() else {
            return;
        };
        stream_song(&web.standalone, project, &streamed);
        // The waveforms: each take's original's peaks, fetched and reduced,
        // then drawn — the audio need not have arrived.
        let previews = song.previews.clone();
        let items: Vec<String> =
            daw_standalone::audio_engine::materialize::pending_media(&web.standalone, project)
                .into_iter()
                .map(|m| m.item_guid)
                .collect();
        wasm_bindgen_futures::spawn_local(async move {
            if streamed.load_peaks(crate::midi::WAVE_BLOCK).await > 0 {
                previews.fill_waves(items).await;
            }
        });
    });
}

/// Keep the page in the set it joined. A playground set (the public
/// demo) starts over every few minutes — its songs' docs made new, the
/// ones this page syncs no longer served — so when the set's epoch moves
/// past `epoch` the page opens again, its songs from the browser's cache,
/// into the new run. And when the connection to Task goes (Task
/// restarting, a network blip), the set is joined again once Task answers
/// — which is also what has Task serve the set's docs again after a
/// restart; the session's replicas reconnect by themselves. Only a set
/// that moved on meanwhile opens the page again.
fn watch_set(url: String, setlist: String, epoch: u64) {
    use live_proto::LiveSessionsStreamClient;
    let reload = || {
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
    };
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            if let Ok(stream) =
                task_dial::establish_at::<LiveSessionsStreamClient>(&url, None).await
            {
                let (tx, mut rx) = vox::channel::<live_proto::LiveEpoch>();
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = stream.epochs(tx).await;
                });
                while let Ok(Some(message)) = rx.recv().await {
                    let mut next = None;
                    let _ = message.map(|e| next = Some((e.setlist.clone(), e.epoch)));
                    if let Some((set, moved)) = next
                        && set == setlist
                        && moved > epoch
                    {
                        tracing::info!(
                            live.epoch = moved,
                            "web: the set starts over; opening it again"
                        );
                        reload();
                        return;
                    }
                }
            }
            // The connection went: join again once Task answers.
            tracing::info!("web: lost Task; joining the set again");
            let Ok(joined) = until_ok(None, || join_set(&url), |_| {}).await else {
                continue;
            };
            if joined.setlist == setlist && joined.epoch != epoch {
                tracing::info!(
                    live.epoch = joined.epoch,
                    "web: the set moved on while away; opening it again"
                );
                reload();
                return;
            }
            tracing::info!("web: back in the set");
        }
    });
}

/// Every take of a shared song attached as its proxy, streaming in — the
/// way a streamed song's takes are attached natively (`stream_in`), pumped
/// by this page's audio loop, fetched in the order they will be heard from
/// the play cursor.
fn stream_song(
    standalone: &daw_standalone::sync::Standalone,
    project: &str,
    song: &crate::song_stream::StreamedSong,
) {
    let media = daw_standalone::audio_engine::materialize::pending_media(standalone, project);
    let takes: Vec<_> = media
        .iter()
        .filter_map(|m| song.attach(standalone, project, m))
        .collect();
    tracing::info!(
        stream.media = media.len(),
        stream.attached = takes.len(),
        "audio: streaming the song's proxies"
    );
    let takes = Arc::new(std::sync::Mutex::new(takes));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    song.fetch(
        takes,
        Arc::new(|| crate::engine::Transport::shared().map_or(0.0, |t| t.read().0)),
        Arc::clone(&stop),
    );
    // Fetching for as long as the page lives.
    std::mem::forget(stop);
}

/// A root-relative path as a URL path: each segment encoded.
fn url_path(path: &str) -> String {
    path.split('/')
        .map(|segment| String::from(js_sys::encode_uri_component(segment)))
        .collect::<Vec<_>>()
        .join("/")
}

/// A URL's whole body.
async fn fetch_bytes(url: &str) -> Result<Arc<[u8]>, String> {
    use wasm_bindgen::JsCast as _;
    let window = web_sys::window().ok_or("no window")?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response")?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body = response.array_buffer().map_err(|e| format!("{e:?}"))?;
    let body = wasm_bindgen_futures::JsFuture::from(body)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&body).to_vec().into())
}

/// The guide library's files MIDI-mode playback needs, fetched together
/// from the library's share link. A file that does not arrive is left out
/// (the instrument synthesizes its slot).
async fn guide_library(base: &str) -> HashMap<String, Vec<u8>> {
    let wanted = session_guide::samples::library::files(
        crate::guide_instrument::CLICK,
        crate::guide_instrument::VOICE,
        "ogg",
    );
    let base = base.trim_end_matches('/');
    let fetches = wanted.into_iter().map(|path| async move {
        let url = format!("{base}/download/{}", url_path(&path));
        match fetch_bytes(&url).await {
            Ok(bytes) => Some((path, bytes.to_vec())),
            Err(e) => {
                tracing::warn!(path, error = %e, "guide: a sample did not arrive");
                None
            }
        }
    });
    let files: HashMap<String, Vec<u8>> = futures_util::future::join_all(fetches)
        .await
        .into_iter()
        .flatten()
        .collect();
    tracing::info!(samples = files.len(), "guide: the sample library arrived");
    files
}
