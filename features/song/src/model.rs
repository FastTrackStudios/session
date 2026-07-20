//! Core durable-storage types: [`Song`], [`Arrangement`], and the
//! reference / manifest value types they carry.
//!
//! These are the STORED counterpart to the runtime `session_proto::Song`
//! (which is hydrated from this form later, out of scope here). Nothing in
//! this module depends on `keyflow`: a chart is a [`ChartRef`] (a relative
//! path within the song folder and/or an attachment id), never an embedded
//! `keyflow::Chart`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::key::Key;

/// Stable identity of a [`Song`].
pub type SongId = Uuid;

/// Stable identity of an [`Arrangement`] within a song.
pub type ArrangementId = Uuid;

/// A portable, self-contained song.
///
/// On disk this is a folder (see the crate docs and
/// `docs/song-folder-format.md`): a `song.md` index plus a per-arrangement
/// resource folder under `arrangements/`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    /// Stable identity. Generated on creation; never re-derived from the
    /// folder name, so renaming the folder doesn't orphan references.
    pub id: SongId,

    /// Human title (e.g. `"Great Are You Lord"`).
    pub title: String,

    /// Free-form tags for organisation / filtering.
    #[serde(default)]
    pub tags: Vec<String>,

    /// The arrangement used by default. MUST match the `id` of one of
    /// [`Song::arrangements`].
    pub default_arrangement: ArrangementId,

    /// One or more arrangements. Always non-empty in a well-formed song
    /// (the default arrangement is one of these).
    pub arrangements: Vec<Arrangement>,
}

impl Song {
    /// Look up an arrangement by id.
    #[must_use]
    pub fn arrangement(&self, id: ArrangementId) -> Option<&Arrangement> {
        self.arrangements.iter().find(|a| a.id == id)
    }

    /// The default arrangement, if present.
    #[must_use]
    pub fn default(&self) -> Option<&Arrangement> {
        self.arrangement(self.default_arrangement)
    }
}

/// One arrangement of a [`Song`] — a specific key + chart + parts layout.
///
/// A song has a default arrangement and may accrue alternates over time
/// (e.g. an acoustic version, a different key for a different vocalist).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arrangement {
    /// Stable identity within the song.
    pub id: ArrangementId,

    /// Human name (e.g. `"Default"`, `"Acoustic"`, `"Key of A"`).
    pub name: String,

    /// The musical key of this arrangement.
    pub key: Key,

    /// Reference to this arrangement's chart, if any. Keyflow-agnostic:
    /// a relative path and/or attachment id — never an embedded chart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_ref: Option<ChartRef>,

    /// Open, org-defined set of parts (stems, instrument charts, etc.).
    #[serde(default)]
    pub parts: PartsManifest,

    /// References to attachments (audio, PDFs, images) that belong to this
    /// arrangement. The real `AttachmentService` is wired elsewhere; here
    /// these are just references.
    #[serde(default)]
    pub attachment_refs: Vec<AttachmentRef>,
}

/// A reference to a chart resource. At least one of the two fields is
/// expected to be set; both may be, in which case `path` is authoritative
/// and `attachment_id` is a durable pointer that survives moves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartRef {
    /// Relative path within the song folder, e.g.
    /// `arrangements/default/chart.kf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Content-addressed / attachment-service id, if the chart is tracked
    /// as an attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
}

impl ChartRef {
    /// A chart referenced purely by relative path.
    #[must_use]
    pub fn from_path(path: impl Into<String>) -> Self {
        Self {
            path: Some(path.into()),
            attachment_id: None,
        }
    }
}

/// A reference to an attachment file that lives alongside the song.
///
/// This is intentionally a *reference* type only — no I/O, no
/// `AttachmentService` coupling (a separate workstream owns that).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRef {
    /// Attachment id (content-addressed or service-assigned).
    pub id: String,

    /// Relative path within the song folder, if the bytes live in-folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// SHA-256 hex digest of the attachment bytes, for integrity /
    /// content addressing. Optional until verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,

    /// Free-form kind hint (`"audio"`, `"pdf"`, `"image"`, …). Open set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// An open, org-defined set of [`Part`]s within an arrangement.
///
/// Minimal by design: a later workstream (W5) fleshes out part-filtered
/// views. For now this just needs to exist, round-trip, and hold named
/// parts with their resource references.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartsManifest {
    #[serde(default)]
    pub parts: Vec<Part>,
}

impl PartsManifest {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// One part (e.g. `"Lead Vocal"`, `"Electric Guitar"`, `"Click"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// Part name. Free-form / org-defined; not a closed enum.
    pub name: String,

    /// Resources associated with this part (charts, stems, notes …).
    #[serde(default)]
    pub resource_refs: Vec<ResourceRef>,
}

/// A generic resource reference used by [`Part`]s — a relative path
/// and/or attachment id, plus an optional kind hint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}
