//! The song library, in Task — the same one keyflow's library reads.
//!
//! A **song** is a `resources_proto::SongDoc` (`song:<slug>`) with its
//! chart attached; a **setlist** is a `songlist` collection of `song:`
//! references in order; a song's **session** — `.RPP`, prepared
//! `.session`, `.lrc`, `Media/Proxies/*.ogg`, and the originals when
//! they were uploaded — is a File Root at `session/<slug>`. Nothing here
//! is Session-specific in Task: the schema is keyflow's and ADR 0004's,
//! and the File Root is found by its path.
//!
//! [`Library`] dials an org with `task-dial` (the dial the web app and
//! the CLI share), lists setlists, and pulls a song's session into a
//! local folder that the app opens like any other.

use std::path::{Path, PathBuf};

use collection_proto::{CollectionKind, CollectionServiceClient};
use files_client::FilesClient;
use files_proto::{
    MediaServiceClient, MediaServiceStreamClient, RootPath, RootsServiceClient,
    TreeServiceClient, UploadServiceClient,
};
use links_proto::NodeKind;
use resources_proto::ResourcesServiceClient;

/// The collection kind a setlist is — keyflow's word for it.
pub const SETLIST_KIND: &str = "songlist";

/// The File Root a song's session lives in.
#[must_use]
pub fn session_root_dir(song_slug: &str) -> String {
    format!("session/{song_slug}")
}

/// A song's library id from its title, the way the library makes one: an
/// apostrophe drops out (`God, I'm` → `god-im`), any other run of
/// non-alphanumerics is one dash, lower-case, no dash at either end.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars().filter(|c| *c != '\'' && *c != '\u{2019}') {
        if c.is_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.extend(c.to_lowercase());
        } else {
            dash = true;
        }
    }
    out
}

/// The token the `task` CLI stored for `org`:
/// `$XDG_DATA_HOME/task/session-tokens/<org>.json` (default data home
/// `~/.local/share`), field `token`.
fn task_cli_token(org: &str) -> Option<String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let text = std::fs::read_to_string(data.join("task/session-tokens").join(format!("{org}.json"))).ok()?;
    // `"token":"…"` — a JWT, so no escapes to undo.
    let rest = text.split("\"token\"").nth(1)?;
    let start = rest.find('"')? + 1;
    let end = start + rest.get(start..)?.find('"')?;
    rest.get(start..end).map(str::to_owned)
}

/// One setlist, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setlist {
    pub id: String,
    pub title: String,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    pub slug: String,
    pub title: String,
    pub writers: Vec<String>,
    pub key: String,
}

/// An org's library on a Task server.
#[derive(Debug, Clone)]
pub struct Library {
    /// `ws://host:port` — the server, without the per-org path.
    pub server: String,
    pub org: String,
    /// The signed-in person's session token, when the server wants one.
    pub token: Option<String>,
}

impl Library {
    /// From `FTS_TASK_SERVER` (default `ws://127.0.0.1:18080`, Task's
    /// local server), `FTS_TASK_ORG` (default `acme-audio`, its demo
    /// org) and `FTS_TASK_TOKEN` — or, without one, the session the
    /// `task` CLI keeps after `task auth login` (so signing in once there
    /// is signing in here).
    #[must_use]
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let org = var("FTS_TASK_ORG").unwrap_or_else(|| "acme-audio".into());
        let token = var("FTS_TASK_TOKEN").or_else(|| task_cli_token(&org));
        Self {
            server: var("FTS_TASK_SERVER").unwrap_or_else(|| "ws://127.0.0.1:18080".into()),
            org,
            token,
        }
    }

    fn org_url(&self) -> String {
        format!("{}/org/{}/vox", self.server.trim_end_matches('/'), self.org)
    }

    async fn client<C: vox_core::FromVoxLane + 'static>(&self) -> eyre::Result<C> {
        task_dial::establish_at::<C>(&self.org_url(), self.token.as_deref())
            .await
            .map_err(|e| eyre::eyre!("dialling {}: {e}", self.org_url()))
    }

    async fn files(&self) -> eyre::Result<FilesClient> {
        Ok(FilesClient::new(
            self.client::<RootsServiceClient>().await?,
            self.client::<TreeServiceClient>().await?,
            self.client::<UploadServiceClient>().await?,
            self.client::<MediaServiceClient>().await?,
            self.client::<MediaServiceStreamClient>().await?,
        ))
    }

    /// Every setlist in the org, with its songs in order.
    ///
    /// # Errors
    /// When the server cannot be reached or refuses.
    pub async fn setlists(&self) -> eyre::Result<Vec<Setlist>> {
        let collections: CollectionServiceClient = self.client().await?;
        let resources: ResourcesServiceClient = self.client().await?;
        let lists = collections
            .list(self.org.clone(), Some(CollectionKind::new(SETLIST_KIND)))
            .await
            .map_err(|e| eyre::eyre!("listing setlists: {e:?}"))?;
        let mut out = Vec::new();
        for list in lists {
            let mut songs = Vec::new();
            for item in list.items.iter().filter(|i| i.node.kind == NodeKind::Song) {
                let slug = item.node.id.clone();
                let song = match resources.song(slug.clone()).await {
                    Ok(doc) => Song { slug, title: doc.title, writers: doc.writers, key: doc.key },
                    // A dangling reference still has a place in the order.
                    Err(_) => Song { title: slug.clone(), slug, writers: Vec::new(), key: String::new() },
                };
                songs.push(song);
            }
            out.push(Setlist { id: list.id, title: list.title, songs });
        }
        Ok(out)
    }

    /// Download a song's session into `into` (created), returning the
    /// folder: `into/<slug>/…`, the same layout it was uploaded from. A
    /// file already there at the same size is left alone.
    ///
    /// What Session plays from comes down — the `.RPP`, the `.session`,
    /// the `.lrc`, `Media/Proxies/`, `Media/Peaks/` — and the originals in `Media/` only
    /// with `originals`: a set streams from its proxies, and the WAVs are
    /// ten times the size.
    ///
    /// # Errors
    /// No session root for the song, or a transfer failed twice.
    pub async fn pull_session(&self, song: &str, into: &Path, originals: bool) -> eyre::Result<PathBuf> {
        let TaskSong { files, root, entries, .. } = self.song(song).await?;
        let mut files = files.lock().await.1.clone();
        let dest = into.join(song);
        for (rel, size) in entries {
            // Proxies and the waveform caches are what a set plays and
            // draws from; everything else in Media/ is an original.
            if is_original(&rel) && !originals {
                continue;
            }
            let local = dest.join(&rel);
            if std::fs::metadata(&local).is_ok_and(|meta| meta.len() == size) {
                continue;
            }
            if let Some(parent) = local.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Streamed to disk, never held whole; one retry on a fresh
            // connection, which is what a dropped stream needs.
            if let Err(first) = fetch_to(&files, root, &rel, &local).await {
                files = self.files().await?;
                fetch_to(&files, root, &rel, &local)
                    .await
                    .map_err(|e| eyre::eyre!("fetching {rel}: {e} (and before that: {first})"))?;
            }
        }
        self.pull_chart(song, &dest).await?;
        Ok(dest)
    }

    /// A song's session where it lies in the library: its files listed
    /// (the `.RPP` first — what a client opens; the prepared `.session`
    /// beside it opens instead), readable by range. What a client that
    /// streams the song in reads from, rather than pulling it whole.
    ///
    /// # Errors
    ///
    /// The server cannot be reached, or the song has no session root.
    pub async fn song(&self, song: &str) -> eyre::Result<TaskSong> {
        let files = self.files().await?;
        let dir = session_root_dir(song);
        let roots = files.roots.list().await.map_err(|e| eyre::eyre!("listing roots: {e:?}"))?;
        let root = roots
            .iter()
            .find(|r| r.path.as_deref().is_some_and(|p| p.trim_end_matches('/').ends_with(&dir)))
            .ok_or_else(|| eyre::eyre!("no session for song:{song} (no `{dir}` root)"))?;
        let id = files_client::root_id(root);
        let mut entries = Vec::new();
        let mut pending = vec![String::new()];
        while let Some(folder) = pending.pop() {
            let path = RootPath::parse(&folder).map_err(|e| eyre::eyre!("path {folder}: {e:?}"))?;
            let listed = files.tree.browse(id, path).await.map_err(|e| eyre::eyre!("browsing {folder}: {e:?}"))?;
            for entry in listed {
                let rel = if folder.is_empty() { entry.name.clone() } else { format!("{folder}/{}", entry.name) };
                if entry.is_dir {
                    pending.push(rel);
                } else {
                    entries.push((rel, entry.size.unwrap_or(0)));
                }
            }
        }
        // The project first: the top-level `.RPP`.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(at) = entries
            .iter()
            .position(|(rel, _)| !rel.contains('/') && rel.to_lowercase().ends_with(".rpp"))
        {
            let project = entries.remove(at);
            entries.insert(0, project);
        }
        Ok(TaskSong {
            slug: song.to_owned(),
            library: self.clone(),
            files: std::sync::Arc::new(tokio::sync::Mutex::new((0, files))),
            root: id,
            entries,
        })
    }

    /// The song's main chart (its keyflow source) from the library — THE
    /// chart: it replaces whatever `.kf` the session's files carried.
    /// `None` when the song has none.
    ///
    /// # Errors
    ///
    /// The server cannot be reached, or refused.
    pub async fn song_chart(&self, song: &str) -> eyre::Result<Option<String>> {
        let resources: ResourcesServiceClient = self.client().await?;
        let charts = resources
            .list_charts(format!("song:{song}"))
            .await
            .map_err(|e| eyre::eyre!("charts of song:{song}: {e:?}"))?;
        let Some(main) = charts.iter().find(|c| c.is_default).or(charts.first()) else {
            return Ok(None);
        };
        let chart = resources
            .chart(main.slug.clone())
            .await
            .map_err(|e| eyre::eyre!("chart:{}: {e:?}", main.slug))?;
        Ok(Some(chart.source))
    }

    /// The song's main chart, written beside its `.RPP` as `<Song>.kf` —
    /// where the app looks for it. The chart lives in the library, not in
    /// the session's files: one chart, the same one keyflow edits.
    async fn pull_chart(&self, song: &str, dest: &Path) -> eyre::Result<()> {
        let Some(chart) = self.song_chart(song).await? else {
            return Ok(());
        };
        write_chart(dest, song, &chart)
    }

    /// Pull every song of a setlist and write a `.setlist` beside them
    /// that the app opens: `into/<Setlist>.setlist`.
    ///
    /// # Errors
    /// As [`Self::pull_session`].
    pub async fn pull_setlist(&self, setlist: &Setlist, into: &Path, originals: bool) -> eyre::Result<PathBuf> {
        std::fs::create_dir_all(into)?;
        let mut lines = vec!["# Pulled from the Task library. One song folder per line.".to_owned()];
        for song in &setlist.songs {
            self.pull_session(&song.slug, into, originals).await?;
            lines.push(song.slug.clone());
        }
        let file = into.join(format!("{}.setlist", setlist.title));
        std::fs::write(&file, lines.join("\n") + "\n")?;
        Ok(file)
    }
}

/// Write a song's library chart into its folder as `<Song>.kf` (the
/// `.RPP`'s stem), replacing any other `.kf` there — the app opens the one
/// `.kf` beside the song, and refuses to guess between two.
///
/// # Errors
///
/// The folder cannot be read or written.
pub fn write_chart(dest: &Path, song: &str, chart: &str) -> eyre::Result<()> {
    let files: Vec<PathBuf> = std::fs::read_dir(dest)?.flatten().map(|e| e.path()).collect();
    for old in files.iter().filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("kf"))) {
        std::fs::remove_file(old)?;
    }
    let stem = files
        .iter()
        .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpp")))
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| song.to_owned());
    std::fs::write(dest.join(format!("{stem}.kf")), chart)?;
    Ok(())
}

/// Whether a song-root path is an original (the WAVs in `Media/`), not a
/// proxy or a waveform cache. Case-blind: macOS is, and a root uploaded
/// from it can carry `Media/peaks/` for the folder the app calls Peaks.
#[must_use]
pub fn is_original(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.starts_with("media/") && !lower.starts_with("media/proxies/") && !lower.starts_with("media/peaks/")
}

/// A song's session in the library, read where it lies (see
/// [`Library::song`]).
#[derive(Clone)]
pub struct TaskSong {
    /// The library's song id (`washed`).
    pub slug: String,
    library: Library,
    /// The connection reads go over, and how many times it has been
    /// replaced — so a dead one is redialled once, not by every read that
    /// found it dead.
    files: std::sync::Arc<tokio::sync::Mutex<(u64, FilesClient)>>,
    root: files_proto::id::RootId,
    /// Every file of the session root, with its size — the project first.
    pub entries: Vec<(String, u64)>,
}

/// How long one range read may take before its connection is taken for
/// dead: a stalled read would otherwise hold a fetch slot forever.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

impl TaskSong {
    /// The bytes `range` of `path` in the song's session — a seek, not a
    /// download. A read that fails or stalls redials the library and is
    /// tried once more: a streamed song outlives any one connection.
    ///
    /// # Errors
    ///
    /// The server refused, or the file is not there, twice.
    pub async fn read(&self, path: &str, range: std::ops::Range<u64>) -> eyre::Result<Vec<u8>> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let (generation, files) = self.files.lock().await.clone();
        let first = match self.read_on(&files, path, &range).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => e,
        };
        let files = {
            let mut held = self.files.lock().await;
            if held.0 == generation {
                let fresh = self.library.files().await.map_err(|e| eyre::eyre!("{first} (and redialling: {e})"))?;
                *held = (generation + 1, fresh);
            }
            held.1.clone()
        };
        self.read_on(&files, path, &range).await.map_err(|e| eyre::eyre!("{e} (and before that: {first})"))
    }

    async fn read_on(&self, files: &FilesClient, path: &str, range: &std::ops::Range<u64>) -> eyre::Result<Vec<u8>> {
        tokio::time::timeout(READ_TIMEOUT, files.read_range(self.root, path, range.start, range.end - 1))
            .await
            .map_err(|_| eyre::eyre!("{path} {range:?}: no answer in {}s", READ_TIMEOUT.as_secs()))?
            .map_err(|e| eyre::eyre!("{path} {range:?}: {e}"))
    }
}

/// Stream one file of a root to `local`, through a temporary name so a
/// broken transfer never leaves a file that looks complete.
async fn fetch_to(
    files: &FilesClient,
    root: files_proto::id::RootId,
    rel: &str,
    local: &Path,
) -> eyre::Result<()> {
    use std::io::Write as _;
    let partial = local.with_extension("part");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&partial)?);
    let mut failed = None;
    files
        .read_to(root, rel, |chunk| {
            if failed.is_none()
                && let Err(e) = out.write_all(chunk)
            {
                failed = Some(e);
            }
        })
        .await
        .map_err(|e| eyre::eyre!("{e}"))?;
    if let Some(e) = failed {
        return Err(e.into());
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&partial, local)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_title_slugs_as_the_library_does() {
        assert_eq!(super::slugify("God, I'm Just Grateful"), "god-im-just-grateful");
        assert_eq!(super::slugify("Always On Time"), "always-on-time");
        assert_eq!(super::slugify("  Thank God I’m Free!"), "thank-god-im-free");
    }
}
