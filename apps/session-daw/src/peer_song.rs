//! A song streamed in from the engine this window drives, when it is not
//! on this machine.
//!
//! The engine serves its song folder (`daw::rpc::SongFiles`). The small
//! files — the prepared `.session`, the chart, the lyrics, the waveform
//! caches, each proxy's page index — are mirrored into a local cache, so
//! the song opens the one way songs open. The proxies are not: each is
//! attached as a take that plays what has arrived
//! (`stream_remote_ogg`, a disk-backed `SparseBytes`), and one fetcher per
//! song brings their bytes in the order they will be heard
//! (`media_fetch::drive`) through [`PeerFetch`], a range-read on the
//! engine. The originals stay where they are.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use daw::standalone::audio_engine::materialize::{PendingMedia, stream_remote_ogg};
use daw::standalone::audio_engine::media_fetch::{FetchConfig, RangeFetch, StreamedTake};
use daw::standalone::sync::Standalone;
use fts_sample::ogg_index::OggIndex;
use fts_sample::sparse::SparseBytes;

/// A song mirrored from a peer: where it opens locally, and its proxies
/// there to stream.
pub struct PeerSong {
    /// The project to open (in the cache).
    pub project: PathBuf,
    cache: PathBuf,
    files: daw::rpc::SongFiles,
    /// Proxies by their file stem, lower-case: (path in the song, size).
    proxies: HashMap<String, (String, u64)>,
}

/// Whether a song-folder path is audio the fetcher streams (a proxy) or
/// leaves alone (an original): everything else is small, and mirrored.
fn is_media(lower: &str) -> bool {
    lower.starts_with("media/") && !lower.starts_with("media/peaks/") && !lower.ends_with(".ogg.idx")
}

/// Mirror the peer's song `remote` (its project on the engine) into
/// `cache_root`: every small file fetched whole (skipped when already
/// here at its size), the media left to stream.
///
/// # Errors
///
/// The engine serves no song folder for it (REAPER, a song never saved),
/// or a file could not be fetched or written.
pub async fn mirror(remote: &str, cache_root: &Path) -> eyre::Result<PeerSong> {
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("no facade"))?;
    let files = daw.project(remote).await?.song_files();
    let list = files.list().await?;
    let first = list.first().ok_or_else(|| eyre::eyre!("the song folder is empty"))?;
    let cache = cache_root.join(safe_name(remote));
    let project = cache.join(&first.path);
    let mut proxies = HashMap::new();
    let mut fetched = 0usize;
    for file in &list {
        let lower = file.path.to_lowercase();
        if lower.starts_with("media/proxies/") && lower.ends_with(".ogg") {
            let stem = Path::new(&file.path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            proxies.insert(stem, (file.path.clone(), file.size));
            continue;
        }
        if is_media(&lower) || file.size == 0 {
            continue;
        }
        let local = cache.join(&file.path);
        if std::fs::metadata(&local).is_ok_and(|m| m.len() == file.size) {
            continue;
        }
        let bytes = files.read(&file.path, 0..file.size).await?;
        if let Some(parent) = local.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&local, bytes)?;
        fetched = fetched.saturating_add(1);
    }
    tracing::info!(
        stream.song = remote,
        stream.files = list.len(),
        stream.fetched = fetched,
        stream.proxies = proxies.len(),
        "peer-song: mirrored"
    );
    Ok(PeerSong { project, cache, files, proxies })
}

fn safe_name(text: &str) -> String {
    text.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

impl PeerSong {
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
        let fetch: Arc<dyn RangeFetch> = Arc::new(PeerFetch { files: self.files.clone() });
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

/// How many of `takes` have all their bytes.
#[must_use]
pub fn arrived(takes: &Mutex<Vec<StreamedTake>>) -> usize {
    takes
        .lock()
        .map(|t| t.iter().filter(|t| t.bytes.missing(&(0..t.bytes.len())).is_empty()).count())
        .unwrap_or(0)
}

/// Bytes from the engine's song folder.
struct PeerFetch {
    files: daw::rpc::SongFiles,
}

impl RangeFetch for PeerFetch {
    fn fetch(
        &self,
        path: &str,
        range: std::ops::Range<u64>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send>> {
        let files = self.files.clone();
        let path = path.to_owned();
        Box::pin(async move { files.read(&path, range).await.map_err(|e| e.to_string()) })
    }
}

/// Stop a song's fetcher when dropped.
pub struct FetchGuard(pub Arc<AtomicBool>);

impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
