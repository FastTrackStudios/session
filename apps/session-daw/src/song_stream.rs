//! A song streamed in from elsewhere, when it is not on this machine —
//! from the engine this window drives, or from the Task library.
//!
//! One way, whatever the source ([`SongSource`]) and wherever it runs: the
//! small files — the prepared `.session`, the chart, the lyrics, the
//! waveform caches, each proxy's page index — are mirrored ([`Keep`]: a
//! cache on disk natively, memory in a browser), so the song opens the one
//! way songs open (`open_core::open_song_in`, through the mirror's
//! [`crate::folder::Folder`]); the proxies are not: each is attached as a
//! take that plays what has arrived (`attach_remote_ogg` over a
//! `SparseBytes`), and one fetcher per song brings their bytes in the order
//! they will be heard (`media_fetch::drive`) by range-reads on the source.
//! The originals stay where they are.
//!
//! - [`PeerSource`] — the engine's song folder (`daw::rpc::SongFiles`).
//! - [`TaskSource`] — the song's session File Root in the library
//!   (`session_library::TaskSong`); its library chart replaces the folder's.
//! - [`ShareSource`] — a Task share link to the song's session (the public
//!   demo: no account): its JSON listing, its documents whole, its proxies
//!   as the link's audio rendition with HTTP ranges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use daw::standalone::audio_engine::materialize::PendingMedia;
use daw::standalone::audio_engine::media_fetch::{FetchConfig, Fetching, RangeFetch, StreamedTake};
use daw::standalone::sync::Standalone;

use crate::folder::{Folder, Memory};
use fts_sample::ogg_index::OggIndex;
use fts_sample::sparse::SparseBytes;

/// Where a song's files come from.
#[cfg(not(target_arch = "wasm32"))]
pub trait SongSource: Send + Sync + 'static {
    /// A name for its cache folder.
    fn label(&self) -> String;
    /// Every file of the song folder with its size, the project first.
    fn list(&self) -> Pending<eyre::Result<Vec<(String, u64)>>>;
    /// The bytes `range` of `path`.
    fn read(&self, path: String, range: std::ops::Range<u64>) -> Pending<Result<Vec<u8>, String>>;
    /// The song's chart, when the source keeps it apart from the folder
    /// (the library's) — it replaces the folder's `.kf`.
    fn chart(&self) -> Pending<eyre::Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }
}

/// Where a song's files come from (see the native definition: here a
/// request is a JS promise, and not `Send`).
#[cfg(target_arch = "wasm32")]
pub trait SongSource: 'static {
    /// A name for its cache folder.
    fn label(&self) -> String;
    /// Every file of the song folder with its size, the project first.
    fn list(&self) -> Pending<eyre::Result<Vec<(String, u64)>>>;
    /// The bytes `range` of `path`.
    fn read(&self, path: String, range: std::ops::Range<u64>) -> Pending<Result<Vec<u8>, String>>;
    /// The song's chart, when the source keeps it apart from the folder
    /// (the library's) — it replaces the folder's `.kf`.
    fn chart(&self) -> Pending<eyre::Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }
}

/// A source's answer, later.
#[cfg(not(target_arch = "wasm32"))]
pub type Pending<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
/// A source's answer, later.
#[cfg(target_arch = "wasm32")]
pub type Pending<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T>>>;

/// The engine this window drives: its project's song folder.
#[cfg(feature = "native")]
pub struct PeerSource {
    remote: String,
    files: daw::rpc::SongFiles,
}

#[cfg(feature = "native")]
impl PeerSource {
    /// The song folder of `remote` (a project on the facade's engine).
    ///
    /// # Errors
    ///
    /// No facade, or no such project.
    pub async fn new(remote: &str) -> eyre::Result<Self> {
        let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("no facade"))?;
        Ok(Self { remote: remote.to_owned(), files: daw.project(remote).await?.song_files() })
    }
}

#[cfg(feature = "native")]
impl SongSource for PeerSource {
    fn label(&self) -> String {
        format!("peer-{}", self.remote)
    }

    fn list(&self) -> Pending<eyre::Result<Vec<(String, u64)>>> {
        let files = self.files.clone();
        Box::pin(async move { Ok(files.list().await?.into_iter().map(|f| (f.path, f.size)).collect()) })
    }

    fn read(&self, path: String, range: std::ops::Range<u64>) -> Pending<Result<Vec<u8>, String>> {
        let files = self.files.clone();
        Box::pin(async move { files.read(&path, range).await.map_err(|e| e.to_string()) })
    }
}

/// A song in the Task library: its session File Root, and its chart.
#[cfg(feature = "native")]
pub struct TaskSource {
    library: session_library::Library,
    song: session_library::TaskSong,
}

#[cfg(feature = "native")]
impl TaskSource {
    /// The library song `slug` (`washed`), from the library the
    /// environment names (`FTS_TASK_SERVER`, `FTS_TASK_ORG`, the token).
    ///
    /// # Errors
    ///
    /// The library cannot be reached, or has no session for the song.
    pub async fn new(slug: &str) -> eyre::Result<Self> {
        let library = session_library::Library::from_env();
        let song = library.song(slug).await?;
        Ok(Self { library, song })
    }
}

#[cfg(feature = "native")]
impl SongSource for TaskSource {
    fn label(&self) -> String {
        format!("task-{}", self.song.slug)
    }

    fn list(&self) -> Pending<eyre::Result<Vec<(String, u64)>>> {
        // The session's own charts give way to the library's.
        let entries: Vec<(String, u64)> = self
            .song
            .entries
            .iter()
            .filter(|(rel, _)| rel.contains('/') || !rel.to_lowercase().ends_with(".kf"))
            .cloned()
            .collect();
        Box::pin(async move { Ok(entries) })
    }

    fn read(&self, path: String, range: std::ops::Range<u64>) -> Pending<Result<Vec<u8>, String>> {
        let song = self.song.clone();
        Box::pin(async move { song.read(&path, range).await.map_err(|e| e.to_string()) })
    }

    fn chart(&self) -> Pending<eyre::Result<Option<String>>> {
        let library = self.library.clone();
        let slug = self.song.slug.clone();
        Box::pin(async move { library.song_chart(&slug).await })
    }
}

/// A Task share link to a song's session
/// (`https://task…/org/<org>/share/<token>`, `?pw=` when it has one).
pub struct ShareSource {
    link: reqwest::Url,
    password: Option<String>,
    http: reqwest::Client,
}

impl ShareSource {
    /// The session `link` shares.
    ///
    /// # Errors
    ///
    /// `link` is not a URL.
    pub fn new(link: &str) -> eyre::Result<Self> {
        let mut link = reqwest::Url::parse(link.trim().trim_end_matches('/'))?;
        let password = link.query_pairs().find(|(k, _)| k == "pw").map(|(_, v)| v.into_owned());
        link.set_query(None);
        Ok(Self { link, password, http: reqwest::Client::new() })
    }

    /// The link's route `route` (`list`, `doc`, `rendition/audio`) for the
    /// song-folder path `path`, each segment escaped.
    fn url(&self, route: &str, path: &str) -> reqwest::Url {
        let mut url = self.link.clone();
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.extend(route.split('/')).extend(path.split('/').filter(|s| !s.is_empty()));
        }
        if let Some(pw) = &self.password {
            url.query_pairs_mut().append_pair("pw", pw);
        }
        url
    }

    /// The link's token, for the cache folder's name — never the password.
    fn token(&self) -> String {
        self.link.path_segments().and_then(Iterator::last).unwrap_or_default().to_owned()
    }
}

/// The media path whose audio rendition is the proxy at `proxy`
/// (`Media/Proxies/Bass.ogg` → `Media/Bass.ogg`: a link streams the
/// committed proxy beside a take by the take's stem), when it is one.
fn rendition_of(proxy: &str) -> Option<String> {
    let (dir, name) = proxy.rsplit_once('/')?;
    let parent = dir.strip_suffix("Proxies")?.trim_end_matches('/');
    let name = name.strip_suffix(".ogg")?;
    Some(if parent.is_empty() { format!("{name}.ogg") } else { format!("{parent}/{name}.ogg") })
}

impl SongSource for ShareSource {
    fn label(&self) -> String {
        format!("share-{}", self.token())
    }

    fn list(&self) -> Pending<eyre::Result<Vec<(String, u64)>>> {
        let request = self.http.get(self.url("list", ""));
        Box::pin(async move {
            let listed: serde_json::Value = request.send().await?.error_for_status()?.json().await?;
            let mut entries: Vec<(String, u64)> = listed["entries"]
                .as_array()
                .ok_or_else(|| eyre::eyre!("the link lists no entries"))?
                .iter()
                .filter_map(|e| Some((e["path"].as_str()?.to_owned(), e["size"].as_u64().unwrap_or(0))))
                .collect();
            // The project first: the top-level `.RPP`, as every source.
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some(at) = entries.iter().position(|(p, _)| !p.contains('/') && p.to_lowercase().ends_with(".rpp")) {
                let project = entries.remove(at);
                entries.insert(0, project);
            }
            Ok(entries)
        })
    }

    fn read(&self, path: String, range: std::ops::Range<u64>) -> Pending<Result<Vec<u8>, String>> {
        let ranged = rendition_of(&path).map(|media| self.url("rendition/audio", &media));
        let request = match &ranged {
            Some(url) => self
                .http
                .get(url.clone())
                .header(reqwest::header::RANGE, format!("bytes={}-{}", range.start, range.end.saturating_sub(1))),
            // A document comes whole (it is small); the range is cut here.
            None => self.http.get(self.url("doc", &path)),
        };
        let partial = ranged.is_some();
        Box::pin(async move {
            let what = |e: &dyn std::fmt::Display| format!("{path} {range:?}: {e}");
            let response = request.send().await.and_then(reqwest::Response::error_for_status).map_err(|e| what(&e))?;
            let served_range = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            let bytes = response.bytes().await.map_err(|e| what(&e))?;
            if partial && served_range {
                return Ok(bytes.to_vec());
            }
            let from = usize::try_from(range.start).unwrap_or(usize::MAX);
            let to = usize::try_from(range.end).unwrap_or(usize::MAX).min(bytes.len());
            bytes.get(from..to).map(<[u8]>::to_vec).ok_or_else(|| what(&"shorter than asked"))
        })
    }
}

/// Where a mirrored song's small files are kept.
#[derive(Clone, Debug)]
pub enum Keep {
    /// A cache folder on this machine (under it, one folder per source),
    /// its proxies' bytes in `.part` files beside them.
    #[cfg(not(target_arch = "wasm32"))]
    Disk(PathBuf),
    /// Memory — a browser's; the proxies' bytes too.
    Memory(Memory),
}

impl Keep {
    /// The folder the mirrored files are read back from.
    fn folder(&self) -> Arc<dyn Folder> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => Arc::new(crate::folder::Disk),
            Self::Memory(memory) => Arc::new(memory.clone()),
        }
    }

    /// Whether a song-folder file (lower-case path) is mirrored here. A
    /// browser skips the waveform caches: its peak store reads them by
    /// disk path, which it has none of, so held in memory they would only
    /// be weight.
    fn wants(&self, lower: &str) -> bool {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => true,
            Self::Memory(_) => !lower.starts_with("media/peaks/"),
        }
    }

    /// How the fetcher holds what it brings: all of it on disk; in memory,
    /// a window around the playhead (the rest is in the browser's cache).
    fn fetch_config(&self) -> FetchConfig {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => FetchConfig::default(),
            Self::Memory(_) => FetchConfig {
                resident: Some(daw::standalone::audio_engine::media_fetch::Resident::BROWSER),
                ..FetchConfig::default()
            },
        }
    }

    /// Where a source labelled `label` is mirrored.
    fn base(&self, label: &str) -> PathBuf {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(root) => root.join(safe_name(label)),
            Self::Memory(_) => Path::new("/").join(safe_name(label)),
        }
    }

    /// Whether `path` is already here at `size` bytes.
    fn has(&self, path: &Path, size: u64) -> bool {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => std::fs::metadata(path).is_ok_and(|m| m.len() == size),
            Self::Memory(memory) => memory.len_of(path) == Some(size),
        }
    }

    fn write(&self, path: &Path, bytes: Vec<u8>) -> eyre::Result<()> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, bytes)?;
            }
            Self::Memory(memory) => memory.insert(path, bytes),
        }
        Ok(())
    }

    /// Room for the `size` bytes of the proxy at `path`, as they arrive.
    fn sparse(&self, path: &Path, size: u64) -> Option<Arc<SparseBytes>> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Disk(_) => {
                let store = PathBuf::from(format!("{}.part", path.display()));
                std::fs::create_dir_all(store.parent()?).ok()?;
                SparseBytes::on_disk(size, &store).ok()
            }
            Self::Memory(_) => {
                let _ = path;
                Some(SparseBytes::in_memory(size))
            }
        }
    }
}

/// A song mirrored from a source: where it opens, and its proxies to
/// stream.
pub struct StreamedSong {
    /// The project to open (in the mirror).
    pub project: PathBuf,
    /// The mirror it opens from.
    pub folder: Arc<dyn Folder>,
    keep: Keep,
    base: PathBuf,
    source: Arc<dyn SongSource>,
    /// Proxies by their file stem, lower-case: (path in the song, size).
    proxies: HashMap<String, (String, u64)>,
    /// Waveform caches by their original's file name, lower-case
    /// (`bass.wav`): (path in the song, size). Ours before REAPER's.
    peaks: HashMap<String, (String, u64)>,
    /// The streams attached so far, by their original's file name.
    attached: Mutex<Vec<(String, daw::standalone::audio_engine::streamed::Streamed)>>,
}

fn safe_name(text: &str) -> String {
    text.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Whether a song-folder path is audio the fetcher streams (a proxy) or
/// leaves alone (an original): everything else is small, and mirrored.
fn is_media(lower: &str) -> bool {
    lower.starts_with("media/") && !lower.starts_with("media/peaks/") && !lower.ends_with(".ogg.idx")
}

/// Mirror a song from `source` into `keep`: every small file fetched whole
/// (skipped when already there at its size), the media left to stream; the
/// source's own chart, if it keeps one, written in.
///
/// # Errors
///
/// The source lists nothing, or a file could not be fetched or kept.
pub async fn mirror(source: Arc<dyn SongSource>, keep: Keep) -> eyre::Result<StreamedSong> {
    let list = source.list().await?;
    let first = list.first().ok_or_else(|| eyre::eyre!("the song folder is empty"))?;
    let label = source.label();
    let base = keep.base(&label);
    let project = base.join(&first.0);
    let mut proxies = HashMap::new();
    let mut peaks: HashMap<String, (String, u64)> = HashMap::new();
    let mut fetched = 0usize;
    for (path, size) in &list {
        let lower = path.to_lowercase();
        if let Some(cache) = lower.strip_prefix("media/peaks/") {
            // `bass.wav.sessionpeaks` or `bass.wav.reapeaks`: ours wins.
            if let Some((original, ext)) = cache.rsplit_once('.')
                && (ext == "sessionpeaks" || (ext == "reapeaks" && !peaks.contains_key(original)))
            {
                peaks.insert(original.to_owned(), (path.clone(), *size));
            }
        }
        if lower.starts_with("media/proxies/") && lower.ends_with(".ogg") {
            let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            proxies.insert(stem, (path.clone(), *size));
            continue;
        }
        if is_media(&lower) || *size == 0 || !keep.wants(&lower) {
            continue;
        }
        let local = base.join(path);
        if keep.has(&local, *size) {
            continue;
        }
        let bytes = source.read(path.clone(), 0..*size).await.map_err(|e| eyre::eyre!(e))?;
        keep.write(&local, bytes)?;
        fetched = fetched.saturating_add(1);
    }
    if let Some(chart) = source.chart().await? {
        write_chart(&keep, &project, &label, &chart)?;
    }
    tracing::info!(
        stream.source = %label,
        stream.files = list.len(),
        stream.fetched = fetched,
        stream.proxies = proxies.len(),
        "song-stream: mirrored"
    );
    Ok(StreamedSong { project, folder: keep.folder(), keep, base, source, proxies, peaks, attached: Mutex::default() })
}

/// The source's chart, as THE chart beside `project` (the folder's own
/// gave way to it in the source's listing).
fn write_chart(keep: &Keep, project: &Path, label: &str, chart: &str) -> eyre::Result<()> {
    match keep {
        #[cfg(feature = "native")]
        Keep::Disk(_) => {
            if let Some(folder) = project.parent() {
                session_library::write_chart(folder, label, chart)?;
            }
            Ok(())
        }
        #[allow(unreachable_patterns)]
        _ => {
            let stem = project.file_stem().map_or_else(|| label.to_owned(), |s| s.to_string_lossy().into_owned());
            keep.write(&project.with_file_name(format!("{stem}.kf")), chart.as_bytes().to_vec())
        }
    }
}

impl StreamedSong {
    /// Attach `media` (a take of the song opened into `daw` as `local`) as
    /// a stream of its proxy — silent until its bytes arrive. `None` when
    /// the peer has no indexed proxy for it.
    pub fn attach(&self, daw: &Standalone, local: &str, media: &PendingMedia) -> Option<StreamedTake> {
        let stem = Path::new(&media.path).file_stem()?.to_string_lossy().to_lowercase();
        let (path, size) = self.proxies.get(&stem)?.clone();
        let at = self.base.join(&path);
        let index = OggIndex::from_text(&self.folder.read_to_string(&OggIndex::path_for(&at)).ok()?)?;
        let bytes = self.keep.sparse(&at, size)?;
        let feeder = daw::standalone::audio_engine::materialize::attach_remote_ogg(
            daw,
            local,
            &media.take_guid,
            Arc::clone(&bytes),
            index.clone(),
        );
        if let (Ok(mut attached), Some(name)) = (self.attached.lock(), Path::new(&media.path).file_name()) {
            attached.push((name.to_string_lossy().to_lowercase(), feeder.source().clone()));
        }
        // Natively the butler thread decodes it; in a browser, the page's
        // audio loop.
        #[cfg(not(target_arch = "wasm32"))]
        daw::standalone::audio_engine::streamed::butler_adopt(feeder);
        #[cfg(all(feature = "web", target_arch = "wasm32"))]
        crate::web_audio::adopt(feeder.boxed());
        #[cfg(all(not(feature = "web"), target_arch = "wasm32"))]
        drop(feeder);
        Some(StreamedTake {
            path,
            bytes,
            index,
            start: media.start,
            end: media.end,
            source_offset: media.source_offset,
            playrate: media.playrate,
        })
    }

    /// Fetch each attached take's waveform — its original's peaks cache,
    /// reduced for a view that never draws blocks finer than `block`
    /// samples ([`daw::standalone::reapeaks::ReaPeaks::coarsened`]) — and
    /// hand it to the take's stream, one at a time. Returns how many took
    /// one. For a holder with no disk to find the caches on (a browser); a
    /// mirror on disk has them where the peak store looks.
    pub async fn load_peaks(&self, block: u32) -> usize {
        use daw::standalone::reapeaks::ReaPeaks;
        let attached = self.attached.lock().map(|a| a.clone()).unwrap_or_default();
        let mut loaded = 0usize;
        for (original, streamed) in attached {
            let Some((path, size)) = self.peaks.get(&original).cloned() else { continue };
            match self.source.read(path.clone(), 0..size).await {
                Ok(bytes) => match ReaPeaks::parse(&bytes) {
                    Ok(peaks) => {
                        streamed.set_peaks(Arc::new(peaks.coarsened(block)));
                        loaded = loaded.saturating_add(1);
                    }
                    Err(e) => tracing::warn!(media = %path, error = %e, "song-stream: a waveform cache did not parse"),
                },
                Err(e) => tracing::warn!(media = %path, error = %e, "song-stream: a waveform cache did not arrive"),
            }
        }
        tracing::info!(stream.source = %self.source.label(), stream.waveforms = loaded, "song-stream: waveforms in");
        loaded
    }

    /// Start fetching `takes`' bytes in the order they will be heard from
    /// `playhead`; stops when `stop` is set.
    pub fn fetch(
        &self,
        takes: Arc<Mutex<Vec<StreamedTake>>>,
        playhead: Arc<dyn Fn() -> f64 + Send + Sync>,
        stop: Arc<AtomicBool>,
    ) {
        let fetch: Arc<dyn RangeFetch> = Arc::new(SourceFetch(Arc::clone(&self.source)));
        // In a browser, through its cache: what the window let go comes
        // back from there, and a reload streams nothing twice.
        #[cfg(target_arch = "wasm32")]
        let fetch: Arc<dyn RangeFetch> = {
            let cached = Arc::new(CachedFetch::new(fetch, &self.source.label(), self.versions()));
            wasm_bindgen_futures::spawn_local(Arc::clone(&cached).warm(Arc::clone(&stop)));
            cached
        };
        let driving =
            daw::standalone::audio_engine::media_fetch::drive(takes, fetch, playhead, self.keep.fetch_config(), stop);
        #[cfg(feature = "native")]
        if let Some(runtime) = crate::open::runtime() {
            runtime.spawn(driving);
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(driving);
    }
}

impl StreamedSong {
    /// Each proxy's size and version (its page index's hash: a proxy made
    /// again is indexed again), by its path in the song.
    #[cfg(target_arch = "wasm32")]
    fn versions(&self) -> HashMap<String, (u64, u64)> {
        self.proxies
            .values()
            .map(|(path, size)| {
                let index = self.folder.read(&OggIndex::path_for(&self.base.join(path))).unwrap_or_default();
                (path.clone(), (*size, fnv(&index)))
            })
            .collect()
    }
}

/// FNV-1a: a stable, dependency-free hash for cache keys.
#[cfg(target_arch = "wasm32")]
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// A song's proxies through the browser's Cache Storage — off the wasm
/// heap, and kept across reloads: each proxy in aligned blocks, a block
/// answered from the cache when it is there and fetched (then kept) when
/// not. What the resident window lets go comes back from here, and
/// [`Self::warm`] fills the cache with the whole song in the background,
/// so a seek anywhere is local once it has.
#[cfg(target_arch = "wasm32")]
pub struct CachedFetch {
    inner: Arc<dyn RangeFetch>,
    label: String,
    /// Size and version by path.
    files: HashMap<String, (u64, u64)>,
}

#[cfg(target_arch = "wasm32")]
impl CachedFetch {
    /// Bytes per cached block.
    const BLOCK: u64 = 256 * 1024;
    /// The Cache Storage cache the blocks live in.
    const CACHE: &'static str = "fts-session-media";

    fn new(inner: Arc<dyn RangeFetch>, label: &str, files: HashMap<String, (u64, u64)>) -> Self {
        Self { inner, label: safe_name(label), files }
    }

    /// The cache key of block `block` of `path` (a URL: Cache Storage keys
    /// are requests; this origin is never fetched).
    fn key(&self, path: &str, version: u64, block: u64) -> String {
        let path: String = path.split('/').map(|s| String::from(js_sys::encode_uri_component(s))).collect::<Vec<_>>().join("/");
        format!("https://fts-cache.invalid/{}/{version:016x}/{path}/{block}", self.label)
    }

    async fn cache() -> Result<web_sys::Cache, String> {
        use wasm_bindgen::JsCast as _;
        let caches = web_sys::window().ok_or("no window")?.caches().map_err(|e| format!("no cache storage: {e:?}"))?;
        let cache = wasm_bindgen_futures::JsFuture::from(caches.open(Self::CACHE)).await.map_err(|e| format!("{e:?}"))?;
        cache.dyn_into().map_err(|_| "not a cache".to_owned())
    }

    /// Block `block` of `path`: from the cache, or fetched and kept.
    async fn block(&self, cache: &web_sys::Cache, path: &str, block: u64) -> Result<Vec<u8>, String> {
        use wasm_bindgen::JsCast as _;
        let (size, version) = self.files.get(path).copied().ok_or_else(|| format!("{path}: not a proxy of this song"))?;
        let key = self.key(path, version, block);
        let hit = wasm_bindgen_futures::JsFuture::from(cache.match_with_str(&key)).await.map_err(|e| format!("{e:?}"))?;
        if let Ok(response) = hit.dyn_into::<web_sys::Response>() {
            let body = response.array_buffer().map_err(|e| format!("{e:?}"))?;
            let body = wasm_bindgen_futures::JsFuture::from(body).await.map_err(|e| format!("{e:?}"))?;
            return Ok(js_sys::Uint8Array::new(&body).to_vec());
        }
        let start = block * Self::BLOCK;
        let mut bytes = self.inner.fetch(path, start..(start + Self::BLOCK).min(size)).await?;
        if let Ok(response) = web_sys::Response::new_with_opt_u8_array(Some(&mut bytes)) {
            // A cache that is full or refused only costs a fetch next time.
            let _ = wasm_bindgen_futures::JsFuture::from(cache.put_with_str(&key, &response)).await;
        }
        Ok(bytes)
    }

    /// Fill the cache with every block of every proxy, one at a time,
    /// until done or `stop`.
    async fn warm(self: Arc<Self>, stop: Arc<AtomicBool>) {
        let Ok(cache) = Self::cache().await else { return };
        let mut files: Vec<(&String, &(u64, u64))> = self.files.iter().collect();
        files.sort();
        let blocks = files.iter().map(|(_, (size, _))| size.div_ceil(Self::BLOCK)).max().unwrap_or(0);
        // Block by block across every file: the whole band's next stretch
        // before any one track's end.
        for block in 0..blocks {
            for (path, (size, _)) in &files {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if block * Self::BLOCK < *size
                    && let Err(e) = self.block(&cache, path, block).await
                {
                    tracing::debug!(media = %path, block, error = %e, "song-stream: warming the cache skipped a block");
                }
            }
        }
        tracing::info!(stream.source = %self.label, stream.files = files.len(), "song-stream: the song is in the browser's cache");
    }
}

#[cfg(target_arch = "wasm32")]
impl RangeFetch for CachedFetch {
    fn fetch(&self, path: &str, range: std::ops::Range<u64>) -> Fetching {
        let this = Self { inner: Arc::clone(&self.inner), label: self.label.clone(), files: self.files.clone() };
        let path = path.to_owned();
        Box::pin(async move {
            if range.is_empty() {
                return Ok(Vec::new());
            }
            let cache = Self::cache().await?;
            let mut out = Vec::with_capacity(usize::try_from(range.end - range.start).unwrap_or(0));
            for block in range.start / Self::BLOCK..=(range.end - 1) / Self::BLOCK {
                let bytes = this.block(&cache, &path, block).await?;
                let at = block * Self::BLOCK;
                let from = usize::try_from(range.start.saturating_sub(at)).unwrap_or(0);
                let to = usize::try_from((range.end - at).min(bytes.len() as u64)).unwrap_or(0);
                out.extend_from_slice(bytes.get(from..to).ok_or_else(|| format!("{path}: block {block} is short"))?);
            }
            Ok(out)
        })
    }
}

/// How many of `takes` have all they play.
#[must_use]
pub fn arrived(takes: &Mutex<Vec<StreamedTake>>) -> usize {
    takes
        .lock()
        .map(|t| t.iter().filter(|t| t.complete()).count())
        .unwrap_or(0)
}

/// Bytes from a song's source.
struct SourceFetch(Arc<dyn SongSource>);

impl RangeFetch for SourceFetch {
    fn fetch(&self, path: &str, range: std::ops::Range<u64>) -> Fetching {
        self.0.read(path.to_owned(), range)
    }
}

/// Stop a song's fetcher when dropped.
pub struct FetchGuard(pub Arc<AtomicBool>);

impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proxy_is_the_rendition_of_its_take_stem() {
        assert_eq!(rendition_of("Media/Proxies/Keys 1.ogg").as_deref(), Some("Media/Keys 1.ogg"));
        assert_eq!(rendition_of("Proxies/Bass.ogg").as_deref(), Some("Bass.ogg"));
        assert_eq!(rendition_of("Media/Proxies/Bass.ogg.idx"), None);
        assert_eq!(rendition_of("Media/Bass.wav"), None);
    }

    #[test]
    fn a_share_link_escapes_paths_and_keeps_its_password_out_of_the_label() {
        let source = ShareSource::new("http://task.test/org/demo/share/tok123?pw=secret").unwrap();
        assert_eq!(source.label(), "share-tok123");
        assert_eq!(
            source.url("doc", "Media/Proxies/Keys 1.ogg.idx").as_str(),
            "http://task.test/org/demo/share/tok123/doc/Media/Proxies/Keys%201.ogg.idx?pw=secret"
        );
        assert_eq!(source.url("list", "").as_str(), "http://task.test/org/demo/share/tok123/list?pw=secret");
    }

    /// Probe: a whole song from a live share link through the fetcher
    /// (`FTS_PROBE_LINK=<link> cargo test -p session-daw --lib probe -- --ignored --nocapture`).
    #[test]
    #[ignore = "needs a live Task share link"]
    fn probe_share_link_throughput() {
        let link = std::env::var("FTS_PROBE_LINK").unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async {
            let source: Arc<dyn SongSource> = Arc::new(ShareSource::new(&link).unwrap());
            let cache = std::env::temp_dir().join(format!("fts-probe-{}", std::process::id()));
            let t = std::time::Instant::now();
            let song = mirror(Arc::clone(&source), Keep::Disk(cache.clone())).await.unwrap();
            eprintln!("mirror {:?}", t.elapsed());
            let mut takes = Vec::new();
            for (path, size) in song.proxies.values() {
                let index = OggIndex::from_text(&std::fs::read_to_string(OggIndex::path_for(&song.base.join(path))).unwrap()).unwrap();
                let secs = index.frames as f64 / f64::from(index.sample_rate);
                takes.push(StreamedTake { path: path.clone(), bytes: SparseBytes::in_memory(*size), index, start: 0.0, end: secs, source_offset: 0.0, playrate: 1.0 });
            }
            let total: u64 = takes.iter().map(|t| t.bytes.len()).sum();
            let takes = Arc::new(Mutex::new(takes));
            let t = std::time::Instant::now();
            daw::standalone::audio_engine::media_fetch::drive(
                takes,
                Arc::new(SourceFetch(source)),
                {
                    // A playhead moving from 30 s, as a playing remote's.
                    let began = std::time::Instant::now();
                    Arc::new(move || 30.0 + began.elapsed().as_secs_f64())
                },
                FetchConfig::default(),
                Arc::new(AtomicBool::new(false)),
            )
            .await;
            eprintln!("fetched {total} bytes in {:?}", t.elapsed());
            let _ = std::fs::remove_dir_all(cache);
        });
    }
}
