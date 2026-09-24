//! A song streamed in from elsewhere, when it is not on this machine —
//! from the engine this window drives, or from the Task library.
//!
//! One way, whatever the source ([`SongSource`]): the small files — the
//! prepared `.session`, the chart, the lyrics, the waveform caches, each
//! proxy's page index — are mirrored into a local cache, so the song opens
//! the one way songs open; the proxies are not: each is attached as a take
//! that plays what has arrived (`stream_remote_ogg`, a disk-backed
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

use daw::standalone::audio_engine::materialize::{PendingMedia, stream_remote_ogg};
use daw::standalone::audio_engine::media_fetch::{FetchConfig, RangeFetch, StreamedTake};
use daw::standalone::sync::Standalone;
use fts_sample::ogg_index::OggIndex;
use fts_sample::sparse::SparseBytes;

/// Where a streamed song's files come from.
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

/// A source's answer, later.
pub type Pending<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

/// The engine this window drives: its project's song folder.
pub struct PeerSource {
    remote: String,
    files: daw::rpc::SongFiles,
}

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
pub struct TaskSource {
    library: session_library::Library,
    song: session_library::TaskSong,
}

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

/// A song mirrored from a source: where it opens locally, and its
/// proxies there to stream.
pub struct StreamedSong {
    /// The project to open (in the cache).
    pub project: PathBuf,
    cache: PathBuf,
    source: Arc<dyn SongSource>,
    /// Proxies by their file stem, lower-case: (path in the song, size).
    proxies: HashMap<String, (String, u64)>,
}

fn safe_name(text: &str) -> String {
    text.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Whether a song-folder path is audio the fetcher streams (a proxy) or
/// leaves alone (an original): everything else is small, and mirrored.
fn is_media(lower: &str) -> bool {
    lower.starts_with("media/") && !lower.starts_with("media/peaks/") && !lower.ends_with(".ogg.idx")
}

/// Mirror a song from `source` into `cache_root`: every small file fetched
/// whole (skipped when already here at its size), the media left to
/// stream; the source's own chart, if it keeps one, written in.
///
/// # Errors
///
/// The source lists nothing, or a file could not be fetched or written.
pub async fn mirror(source: Arc<dyn SongSource>, cache_root: &Path) -> eyre::Result<StreamedSong> {
    let list = source.list().await?;
    let first = list.first().ok_or_else(|| eyre::eyre!("the song folder is empty"))?;
    let cache = cache_root.join(safe_name(&source.label()));
    let project = cache.join(&first.0);
    let mut proxies = HashMap::new();
    let mut fetched = 0usize;
    for (path, size) in &list {
        let lower = path.to_lowercase();
        if lower.starts_with("media/proxies/") && lower.ends_with(".ogg") {
            let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            proxies.insert(stem, (path.clone(), *size));
            continue;
        }
        if is_media(&lower) || *size == 0 {
            continue;
        }
        let local = cache.join(path);
        if std::fs::metadata(&local).is_ok_and(|m| m.len() == *size) {
            continue;
        }
        let bytes = source.read(path.clone(), 0..*size).await.map_err(|e| eyre::eyre!(e))?;
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&local, bytes)?;
        fetched = fetched.saturating_add(1);
    }
    if let Some(chart) = source.chart().await?
        && let Some(folder) = project.parent()
    {
        session_library::write_chart(folder, &source.label(), &chart)?;
    }
    tracing::info!(
        stream.source = %source.label(),
        stream.files = list.len(),
        stream.fetched = fetched,
        stream.proxies = proxies.len(),
        "song-stream: mirrored"
    );
    Ok(StreamedSong { project, cache, source, proxies })
}

impl StreamedSong {
    /// Attach `media` (a take of the song opened into `daw` as `local`) as
    /// a stream of its proxy — silent until its bytes arrive. `None` when
    /// the peer has no indexed proxy for it.
    pub fn attach(&self, daw: &Standalone, local: &str, media: &PendingMedia) -> Option<StreamedTake> {
        let stem = Path::new(&media.path).file_stem()?.to_string_lossy().to_lowercase();
        let (path, size) = self.proxies.get(&stem)?.clone();
        let index_file = OggIndex::path_for(&self.cache.join(&path));
        let index = OggIndex::from_text(&std::fs::read_to_string(&index_file).ok()?)?;
        let store = self.cache.join(format!("{path}.part"));
        if let Some(parent) = store.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        let bytes = SparseBytes::on_disk(size, &store).ok()?;
        stream_remote_ogg(daw, local, &media.take_guid, Arc::clone(&bytes), index.clone());
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

    /// Start fetching `takes`' bytes in the order they will be heard from
    /// `playhead`; stops when `stop` is set.
    pub fn fetch(
        &self,
        takes: Arc<Mutex<Vec<StreamedTake>>>,
        playhead: Arc<dyn Fn() -> f64 + Send + Sync>,
        stop: Arc<AtomicBool>,
    ) {
        let fetch: Arc<dyn RangeFetch> = Arc::new(SourceFetch(Arc::clone(&self.source)));
        if let Some(runtime) = crate::open::runtime() {
            runtime.spawn(daw::standalone::audio_engine::media_fetch::drive(
                takes,
                fetch,
                playhead,
                FetchConfig::default(),
                stop,
            ));
        }
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
    fn fetch(
        &self,
        path: &str,
        range: std::ops::Range<u64>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send>> {
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
            let song = mirror(Arc::clone(&source), &cache).await.unwrap();
            eprintln!("mirror {:?}", t.elapsed());
            let mut takes = Vec::new();
            for (path, size) in song.proxies.values() {
                let index = OggIndex::from_text(&std::fs::read_to_string(OggIndex::path_for(&song.cache.join(path))).unwrap()).unwrap();
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
