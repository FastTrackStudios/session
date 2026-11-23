//! A session's file: one format for every app that holds a session.
//!
//! A session on disk is an Architect document file (`crdt::file::DocFile`)
//! of kind [`crate::doc::KIND`]: its edits as steps, its id and kind in its
//! header, synced through an org like any document of a registered kind.
//! The DAW engine, Keyflow and any other host open and save it the same
//! way, here, so each can open the others' sessions: one session per song,
//! not one per app.
//!
//! A session written before this as a bare Loro snapshot (what the DAW
//! engine wrote) is brought into the format once, by [`SessionFile::import`]
//! or [`SessionFile::open_or_import`]: same content, a document file from
//! then on.
//!
//! A song's session lives inside the song's one file (`<Title>.session`,
//! the DAW's project store, `docs/design/one-file.md`): there it is not a
//! document file of its own but a part of that store, and is opened over
//! the store's persistence with [`SessionFile::over_persistence`]. The
//! session itself, its id and every edit are the same either way.
//!
//! Which a file is, is said by its first bytes ([`sniff`]), never by its
//! name.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crdt::CrdtDoc;
use crdt::file::{DiskStorage, DocFile, FileError, MAIN};
use crdt::persistence::Persistence as _;
use uuid::Uuid;

use crate::doc::{SessionDoc, kind};

/// Why a session file could not be opened, made or saved.
#[derive(Debug)]
pub enum SessionFileError {
    /// The file: missing, not a document file, another kind, unwritable.
    File(String),
    /// The document refused what it was given.
    Doc(String),
}

impl std::fmt::Display for SessionFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(e) => write!(f, "session file: {e}"),
            Self::Doc(e) => write!(f, "session document: {e}"),
        }
    }
}

impl std::error::Error for SessionFileError {}

type Result<T> = std::result::Result<T, SessionFileError>;

#[allow(clippy::needless_pass_by_value)] // An adapter for `map_err`.
fn file_error(e: FileError) -> SessionFileError {
    SessionFileError::File(e.to_string())
}

/// What a song's file starts with, as [`sniff`] reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sniffed {
    /// An Architect document file: a session file of its own
    /// (`<Title>.fts-session`, before the song's one file).
    DocFile,
    /// A SQLite (Turso) store: the song's one file, whose session is a
    /// part of it ([`SessionFile::over_persistence`]).
    Store,
    /// Neither.
    Other,
}

/// What the file starting with `head` (16 bytes are enough) is.
#[must_use]
pub fn sniff(head: &[u8]) -> Sniffed {
    if head.starts_with(b"ARCHDOC\0") {
        Sniffed::DocFile
    } else if head.starts_with(b"SQLite format 3\0") {
        Sniffed::Store
    } else {
        Sniffed::Other
    }
}

/// Where a session's edits go: its own document file, or the persistence
/// of the file it is a part of.
enum Backing {
    File(DocFile),
    Part(Arc<dyn crdt::persistence::Persistence>),
}

/// A session held open in its file. Edits to [`Self::session`] are steps
/// of the file as they are committed; [`Self::save`] folds them.
pub struct SessionFile {
    id: Uuid,
    backing: Backing,
    doc: CrdtDoc,
    session: SessionDoc,
}

impl SessionFile {
    /// Makes a new session file at `path`.
    ///
    /// # Errors
    ///
    /// The file could not be made.
    pub async fn create(path: &Path) -> Result<Self> {
        let storage = DiskStorage::open(path).map_err(file_error)?;
        let file = DocFile::create(storage, &kind())
            .await
            .map_err(file_error)?;
        Self::over(file).await
    }

    /// Opens the session file at `path`.
    ///
    /// # Errors
    ///
    /// No such file, not a document file (see [`Self::open_or_import`]),
    /// another kind's, or from a newer build.
    pub async fn open(path: &Path) -> Result<Self> {
        let storage = DiskStorage::open(path).map_err(file_error)?;
        let file = DocFile::open(storage, &kind()).await.map_err(file_error)?;
        Self::over(file).await
    }

    /// A session file at `path` for session `id` made on another device:
    /// empty until it syncs, of the same lineage.
    ///
    /// # Errors
    ///
    /// The file could not be made.
    pub async fn adopt(path: &Path, id: Uuid) -> Result<Self> {
        let storage = DiskStorage::open(path).map_err(file_error)?;
        let file = DocFile::adopt(storage, &kind(), id)
            .await
            .map_err(file_error)?;
        Self::over(file).await
    }

    /// A new session file at `path` holding the session in `snapshot`, a
    /// bare Loro snapshot (a session saved before the file format).
    ///
    /// # Errors
    ///
    /// The file could not be made, or `snapshot` is not a Loro document.
    pub async fn import(path: &Path, snapshot: &[u8]) -> Result<Self> {
        let this = Self::create(path).await?;
        this.session
            .doc()
            .import(snapshot)
            .map_err(|e| SessionFileError::Doc(e.to_string()))?;
        this.session.doc().commit();
        this.save().await?;
        Ok(this)
    }

    /// Opens the session at `path`: a session file as it is, or a bare
    /// Loro snapshot brought into the format in place (the snapshot kept
    /// beside it as `<path>.snapshot`), so an app opens a session another
    /// app saved either way.
    ///
    /// # Errors
    ///
    /// No such file, or neither a session file nor a Loro document.
    pub async fn open_or_import(path: &Path) -> Result<Self> {
        match Self::open(path).await {
            Err(SessionFileError::File(why)) if why.starts_with("not a document file") => {
                let snapshot = std::fs::read(path)
                    .map_err(|e| SessionFileError::File(format!("{}: {e}", path.display())))?;
                // Keep what was there; the session file takes its place.
                let mut kept = PathBuf::from(path).into_os_string();
                kept.push(".snapshot");
                std::fs::rename(path, &kept)
                    .map_err(|e| SessionFileError::File(format!("{}: {e}", path.display())))?;
                Self::import(path, &snapshot).await
            }
            other => other,
        }
    }

    async fn over(file: DocFile) -> Result<Self> {
        let id = file
            .id()
            .await
            .ok_or_else(|| SessionFileError::File("the file names no document".into()))?;
        let doc = CrdtDoc::open(id, file.persistence(MAIN))
            .await
            .map_err(|e| SessionFileError::Doc(e.to_string()))?;
        let session = SessionDoc::from_doc(doc.loro().clone());
        Ok(Self {
            id,
            backing: Backing::File(file),
            doc,
            session,
        })
    }

    /// The session `id` kept in `persistence`: a part of another file (the
    /// song's one file, whose store answers for its parts by id). What it
    /// holds is loaded; every commit is appended there, and [`Self::save`]
    /// folds it there.
    ///
    /// # Errors
    ///
    /// What the persistence holds would not load.
    pub async fn over_persistence(
        id: Uuid,
        persistence: Arc<dyn crdt::persistence::Persistence>,
    ) -> Result<Self> {
        let doc = CrdtDoc::open_shared(id, persistence.clone())
            .await
            .map_err(|e| SessionFileError::Doc(e.to_string()))?;
        let session = SessionDoc::from_doc(doc.loro().clone());
        Ok(Self {
            id,
            backing: Backing::Part(persistence),
            doc,
            session,
        })
    }

    /// The session's document id: what a pane, a share link and the sync
    /// driver name it by.
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// The session.
    #[must_use]
    pub const fn session(&self) -> &SessionDoc {
        &self.session
    }

    /// The document (for a sync driver: `crdt::sync::SyncedDoc` over it,
    /// made `durable`, or the org's).
    #[must_use]
    pub fn crdt(&self) -> CrdtDoc {
        self.doc.clone()
    }

    /// Writes everything the session holds to its file and folds the
    /// file's records into one (the whole state first, so a commit the
    /// document's writer task has not appended yet is not lost).
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub async fn save(&self) -> Result<()> {
        let state = self
            .doc
            .loro()
            .export(loro::ExportMode::Snapshot)
            .map_err(|e| SessionFileError::Doc(e.to_string()))?;
        match &self.backing {
            Backing::File(file) => file
                .persistence(MAIN)
                .write_snapshot(self.id, &state)
                .await
                .map_err(|e| SessionFileError::File(e.to_string()))?,
            // A part folds what its file holds with what it is handed: a
            // commit not appended yet is in `state`.
            Backing::Part(persistence) => persistence
                .compact(self.id, &state)
                .await
                .map_err(|e| SessionFileError::File(e.to_string()))?,
        }
        self.doc
            .compact(self.id)
            .await
            .map_err(|e| SessionFileError::File(e.to_string()))
    }
}
