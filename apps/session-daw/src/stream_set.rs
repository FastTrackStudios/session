//! A set streamed in from Task, played by this window's own engine: a live
//! set joined by its link (the public demo, or one someone shared), or a
//! setlist in an org's library, as a member.
//!
//! Each song comes in the one way a song streams in ([`crate::song_stream`]):
//! its small files mirrored to a cache on disk, so it opens the one way a
//! song opens ([`Setlist::open_streamed`]) with its media deferred; then
//! each take is attached to a stream of its proxy, and one fetcher per song
//! brings their bytes in, what the playhead will reach first first
//! ([`stream`]). The first song opens the set; the rest open behind it,
//! one by one ([`take_arrivals`]). A live set is joined as well
//! ([`crate::collab::join_task`]): its people, its leader, its edits.
//!
//! Blocking, like [`Setlist::open`]: a window runs it on a thread of its
//! own, and hears each step through `progress` — the loading screen's.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use daw::standalone::audio_engine::materialize::pending_media;
use daw::standalone::audio_engine::media_fetch::StreamedTake;

use crate::collab::TaskSet;
use crate::loading::Progress;
use crate::setlist::{Arrival, Opening, Setlist};
use crate::song_stream::{FetchGuard, Keep, ShareSource, SongSource, StreamedSong, TaskSource};

/// A setlist in an org's library, as [`setlists`] lists them.
pub use session_library::Setlist as LibrarySetlist;

/// A set Task keeps, to stream in.
pub enum Remote {
    /// A live share link (`https://…/org/<org>/share/<token>`): joined as a
    /// guest called `name`, each song from its files link.
    Live { link: String, name: String },
    /// A setlist in the org's library, joined as the member signed in to
    /// it, called `name`.
    Library {
        library: session_library::Library,
        setlist: session_library::Setlist,
        name: String,
    },
}

/// The library's setlists, for picking one.
///
/// # Errors
///
/// The library could not be reached, or refused.
pub fn setlists(library: &session_library::Library) -> eyre::Result<Vec<session_library::Setlist>> {
    crate::open::engine_runtime()?.block_on(library.setlists())
}

/// Open `remote`'s songs into this window's engine, the first current, and
/// join the set. A song that cannot be brought in is left out, as a song
/// that will not open is.
///
/// # Errors
///
/// The set could not be joined, or none of its songs opened.
pub fn open(remote: Remote, progress: &dyn Fn(Progress)) -> eyre::Result<Setlist> {
    use crate::task_set::until_ok;
    let runtime = crate::open::engine_runtime()?;
    // What opens a song spawns tasks (`architect::platform::spawn`): on
    // the runtime, from this thread as from the one the rest open on.
    let _entered = runtime.enter();
    let (sources, set, name) = match remote {
        Remote::Live { link, name } => {
            let set = TaskSet::parse(&format!("share:{link}"));
            progress(Progress::Joining { retry: None });
            let joined = runtime.block_on(until_ok(
                Some(5),
                || crate::task_set::join(&set.url),
                |why| progress(Progress::Joining { retry: Some(why) }),
            ))?;
            tracing::info!(live.setlist = %joined.title, live.songs = joined.songs.len(), "stream-set: joined a live set");
            let sources: Vec<(String, Source)> = joined
                .songs
                .iter()
                .filter_map(|song| Some((song.title.clone(), Source::Share(song.files.clone()?))))
                .collect();
            let set = TaskSet {
                setlist: joined.setlist,
                ..set
            };
            (sources, set, name)
        }
        Remote::Library {
            library,
            setlist,
            name,
        } => {
            // Each song is looked up in the library when its turn comes —
            // not all of them before the first can open.
            let sources: Vec<(String, Source)> = setlist
                .songs
                .iter()
                .map(|song| {
                    (
                        song.title.clone(),
                        Source::Library(library.clone(), song.slug.clone()),
                    )
                })
                .collect();
            let set = TaskSet {
                url: library.org_url(),
                token: library.token.clone(),
                setlist: setlist.id.clone(),
            };
            (sources, set, name)
        }
    };
    let mut sources = sources.into_iter();
    let cache = cache_dir();
    // The first song that comes in opens the set; the rest open behind it.
    let (setlist, rest) = loop {
        let Some((title, source)) = sources.next() else {
            eyre::bail!("none of the set's songs could be brought in");
        };
        progress(Progress::Fetching {
            title: title.clone(),
            retry: None,
        });
        let song = match runtime.block_on(bring_in(&source, &cache)) {
            Ok(song) => song,
            Err(e) => {
                tracing::warn!(song.title = %title, error = %e, "stream-set: a song could not be brought in; the set goes on without it");
                continue;
            }
        };
        progress(Progress::Opening {
            title: title.clone(),
        });
        let mut setlist = Setlist::open_streamed(vec![Opening {
            path: song.project.clone(),
            title: Some(title),
            streamed: Some(song),
        }])?;
        let rest: Vec<(String, Source)> = sources.collect();
        setlist.pending = rest.iter().map(|(title, _)| title.clone()).collect();
        break (setlist, rest);
    };
    crate::collab::join_task_in_background(set, name);
    let (arrive, arrivals) = tokio::sync::mpsc::unbounded_channel();
    if let Ok(mut slot) = ARRIVALS.lock() {
        *slot = Some(arrivals);
    }
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        for (title, source) in rest {
            let opened = runtime
                .block_on(bring_in(&source, &cache))
                .and_then(|song| {
                    Setlist::open_behind(Opening {
                        path: song.project.clone(),
                        title: Some(title.clone()),
                        streamed: Some(song),
                    })
                });
            let arrival = match opened {
                Ok(song) => Arrival::Song(song),
                Err(e) => {
                    tracing::warn!(song.title = %title, error = %e, "stream-set: a song did not open; the set goes on without it");
                    Arrival::Failed(title)
                }
            };
            if arrive.send(arrival).is_err() {
                break;
            }
        }
    });
    Ok(setlist)
}

/// Where a song of the set comes from.
enum Source {
    /// A live set's song: its files link.
    Share(String),
    /// A library setlist's song: its slug in the library.
    Library(session_library::Library, String),
}

impl Source {
    /// The song, reached: a share link parsed, or the library asked for it.
    async fn reach(&self) -> eyre::Result<Arc<dyn SongSource>> {
        Ok(match self {
            Self::Share(link) => Arc::new(ShareSource::new(link)?),
            Self::Library(library, slug) => Arc::new(TaskSource::of(library.clone(), slug).await?),
        })
    }
}

/// A song reached and its small files mirrored into `cache` — tried again
/// a few times, since Task may be busy.
async fn bring_in(source: &Source, cache: &std::path::Path) -> eyre::Result<StreamedSong> {
    let reached = source.reach().await?;
    crate::task_set::until_ok(
        Some(3),
        || crate::song_stream::mirror(Arc::clone(&reached), Keep::Disk(cache.to_path_buf())),
        |_| {},
    )
    .await
}

/// The set's songs after the first, as each opens — once, for the window
/// that shows them.
static ARRIVALS: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Arrival>>> = Mutex::new(None);

/// The songs arriving behind the first (see [`open`]), for the window to
/// add to its set; `None` once taken, or when nothing is arriving.
pub fn take_arrivals() -> Option<tokio::sync::mpsc::UnboundedReceiver<Arrival>> {
    ARRIVALS.lock().ok()?.take()
}

/// Where streamed songs' small files are kept: the same bytes at the same
/// version are not fetched twice.
fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Session")
        .join("streamed")
}

/// A song's streams, kept playing for as long as the process runs.
struct Streaming {
    _song: StreamedSong,
    _takes: Arc<Mutex<Vec<StreamedTake>>>,
    _fetching: FetchGuard,
}

static STREAMS: Mutex<Vec<Streaming>> = Mutex::new(Vec::new());

/// Attach every take of the song just opened as `local` to a stream of its
/// proxy in `song`, and start bringing their bytes in from the song's
/// playhead on.
pub fn stream(song: StreamedSong, local: &str) {
    let Some(daw) = crate::open::local_engine() else {
        tracing::warn!("stream-set: no engine to stream into");
        return;
    };
    let media = pending_media(daw, local);
    let takes: Vec<StreamedTake> = media
        .iter()
        .filter_map(|m| {
            song.attach(daw, local, m)
                .map(|attached| attached.take)
                .or_else(|| {
                    tracing::warn!(stream.media = %m.path, "stream-set: no indexed proxy for this take");
                    None
                })
        })
        .collect();
    tracing::info!(
        stream.song = local,
        stream.media = media.len(),
        stream.attached = takes.len(),
        "stream-set: takes attached to their streams"
    );
    let takes = Arc::new(Mutex::new(takes));
    let stop = Arc::new(AtomicBool::new(false));
    let playhead: Arc<dyn Fn() -> f64 + Send + Sync> = match daw.sync_backend(local) {
        Some(backend) => Arc::new(move || {
            daw_transport_sync::TransportBackend::snapshot(&backend)
                .map_or(0.0, |s| s.playhead_seconds)
        }),
        None => Arc::new(|| 0.0),
    };
    song.fetch(Arc::clone(&takes), playhead, Arc::clone(&stop));
    if let Ok(mut streams) = STREAMS.lock() {
        streams.push(Streaming {
            _song: song,
            _takes: takes,
            _fetching: FetchGuard(stop),
        });
    }
}
