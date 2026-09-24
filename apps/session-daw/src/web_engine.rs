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
    Bundled { name: String, rpp_url: String, chart_url: Option<String> },
    /// A live share link to a set Task keeps (`live-proto`): the page joins
    /// it — its first song opened from that song's files link, everyone in
    /// the set in one session with it (`collab::join_task`), by `name`.
    Live { link: String, name: String },
}

/// Open the page's song the one way songs open (`open_core::open_song_in`,
/// from a memory folder): the prepared `.session` when the song carries
/// one, else its project prepared here (organized, built from its chart,
/// given its guide); the engine stood up in-process, and the session read
/// back as the studio lays it out. `guide` is a share link to the guide
/// sample library (as Ogg) the click, count and cue tracks play from;
/// without it they play synthesized ticks and beeps.
///
/// # Errors
///
/// The song could not be fetched, did not parse, the facade did not come
/// up, or the project could not be read back.
pub async fn open(source: &WebSource, guide: Option<&str>) -> eyre::Result<(EngineRef, StudioSession)> {
    use crate::folder::{Folder, Memory};
    use crate::song_stream::{Keep, ShareSource, StreamedSong, mirror};

    // A live set: joined first, for the song's files link.
    let mut live: Option<crate::collab::TaskSet> = None;
    let shared_link = match source {
        WebSource::Live { link, .. } => {
            use live_proto::LiveSessionsClient;
            let set = crate::collab::TaskSet::parse(&format!("share:{link}"));
            let lane: LiveSessionsClient =
                task_dial::establish_at(&set.url, None).await.map_err(|e| eyre::eyre!("dialling the set: {e}"))?;
            let joined = lane.join(String::new()).await.map_err(|e| eyre::eyre!("joining the set: {e:?}"))?;
            let files = joined
                .songs
                .iter()
                .find_map(|s| s.files.clone())
                .ok_or_else(|| eyre::eyre!("the set has no song with files to open"))?;
            tracing::info!(live.setlist = %joined.title, live.songs = joined.songs.len(), live.epoch = joined.epoch, "web: joined a live set");
            restart_on_next_epoch(&set.url, joined.setlist.clone(), joined.epoch);
            live = Some(crate::collab::TaskSet { setlist: joined.setlist, ..set });
            Some(files)
        }
        WebSource::Shared { link } => Some(link.clone()),
        WebSource::Bundled { .. } => None,
    };
    let (folder, path, streamed): (Arc<dyn Folder>, std::path::PathBuf, Option<StreamedSong>) = match (source, shared_link) {
        (_, Some(link)) => {
            let song = mirror(Arc::new(ShareSource::new(&link)?), Keep::Memory(Memory::new())).await?;
            (Arc::clone(&song.folder), song.project.clone(), Some(song))
        }
        (WebSource::Bundled { name, rpp_url, chart_url }, None) => {
            let memory = Memory::new();
            let dir = std::path::Path::new("/bundled");
            let rpp = dir.join(format!("{name}.RPP"));
            memory.insert(&rpp, fetch_bytes(rpp_url).await.map_err(|e| eyre::eyre!(e))?);
            if let Some(url) = chart_url {
                memory.insert(dir.join(format!("{name}.kf")), fetch_bytes(url).await.map_err(|e| eyre::eyre!(e))?);
            }
            (Arc::new(memory), rpp, None)
        }
        (_, None) => eyre::bail!("nothing to open"),
    };

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
    let prepare = crate::prepare::Prepare::for_song_in(folder.as_ref(), &path);
    let (opened, plan) =
        crate::open_core::open_song_in(folder.as_ref(), &standalone, &path, &prepare, crate::open_core::Media::Deferred)?;
    let name = opened.name.clone();
    let project_guid = opened.project_guid.clone();
    let rpp_text = crate::open_core::project_text_in(folder.as_ref(), &plan.open)?.text;
    let chart_text = prepare.chart.as_ref().and_then(|p| folder.read_to_string(p).ok());
    // The song on screen, and the chart it seeds a live set's doc with.
    crate::open::set_current_song(&project_guid);
    if let Some(chart) = &chart_text {
        crate::collab::remember_chart(&plan.open, chart);
    }

    let bundle = daw_standalone::bootstrap::build_in_process_daw(standalone.clone())
        .await
        .map_err(|e| eyre::eyre!("build_in_process_daw: {e:?}"))?;
    daw::init_from_parts(bundle.daw.clone());
    // The facade and the engine live as long as the page.
    std::mem::forget(bundle);
    crate::web_audio::install(standalone.clone(), &project_guid);
    let streamed = streamed.map(|song| {
        let song = Rc::new(song);
        stream_song(&standalone, &project_guid, &song);
        song
    });

    // In the live set with everyone else (the facade is up: the session's
    // bridges read and drive the engine through it).
    if let (Some(set), WebSource::Live { name, .. }) = (live, source) {
        if let Err(e) = crate::collab::join_task(&set, name.clone()) {
            tracing::warn!(collab.error = %e, "web: could not join the live set");
        }
    }

    // Say which step fails: `fetch` answers only yes or no.
    let facade = daw_control::Daw::try_get().ok_or_else(|| eyre::eyre!("the daw facade is not up"))?;
    let current = facade
        .current_project()
        .await
        .map_err(|e| eyre::eyre!("no current project: {e}"))?;
    current
        .tracks()
        .all()
        .await
        .map_err(|e| eyre::eyre!("the tracks could not be read: {e}"))?;
    let raw = daw_ui::studio::project::fetch()
        .await
        .ok_or_else(|| eyre::eyre!("could not read {name} back"))?;
    let planner = Planner {
        raw: Arc::new(raw),
        scene: Some(SCENE),
        kinds: Arc::new(crate::plan::Kinds::from_text(&rpp_text)),
    };
    let (project, rows) = planner.plan(&planner.raw);
    let previews = crate::midi::Previews::default();
    previews
        .fill(
            project
                .0
                .items
                .values()
                .flatten()
                .filter(|item| project.0.is_midi(&item.guid))
                .map(|item| (item.guid.clone(), item.length.as_seconds()))
                .collect(),
        )
        .await;
    // The waveforms: each take's original's peaks, fetched and reduced,
    // then drawn — the audio need not have arrived.
    if let Some(song) = streamed {
        let previews = previews.clone();
        let items: Vec<String> = daw_standalone::audio_engine::materialize::pending_media(&standalone, &project_guid)
            .into_iter()
            .map(|m| m.item_guid)
            .collect();
        wasm_bindgen_futures::spawn_local(async move {
            if song.load_peaks(crate::midi::WAVE_BLOCK).await > 0 {
                previews.fill_waves(items).await;
            }
        });
    }
    let chart = chart_text.as_deref().and_then(|text| {
        keyflow::parse(text)
            .inspect_err(|e| tracing::error!(error = %e, "chart: could not parse"))
            .ok()
            .map(Arc::new)
    });
    let engine = EngineRef {
        applier: Rc::new(crate::engine::Applier::start()),
    };
    Ok((
        engine,
        StudioSession {
            project,
            rows,
            previews,
            chart,
            chart_file: None,
            planner,
        },
    ))
}

/// A playground set (the public demo) starts over every few minutes: its
/// songs' docs are made new, and the ones this page syncs are no longer
/// served. When the set's epoch moves past `epoch`, the page opens again —
/// its song from the browser's cache, pristine, and into the new run.
fn restart_on_next_epoch(url: &str, setlist: String, epoch: u64) {
    use live_proto::LiveSessionsStreamClient;
    let url = url.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let stream: LiveSessionsStreamClient = match task_dial::establish_at(&url, None).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!(live.error = %e, "web: the set's resets cannot be followed");
                return;
            }
        };
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
                tracing::info!(live.epoch = moved, "web: the set starts over; opening it again");
                if let Some(window) = web_sys::window() {
                    let _ = window.location().reload();
                }
                return;
            }
        }
    });
}

/// Every take of a shared song attached as its proxy, streaming in — the
/// way a streamed song's takes are attached natively (`stream_in`), pumped
/// by this page's audio loop, fetched in the order they will be heard from
/// the play cursor.
fn stream_song(standalone: &daw_standalone::sync::Standalone, project: &str, song: &crate::song_stream::StreamedSong) {
    let media = daw_standalone::audio_engine::materialize::pending_media(standalone, project);
    let takes: Vec<_> = media.iter().filter_map(|m| song.attach(standalone, project, m)).collect();
    tracing::info!(stream.media = media.len(), stream.attached = takes.len(), "audio: streaming the song's proxies");
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
