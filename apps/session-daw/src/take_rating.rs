//! Rating vocal takes while recording: ★★★ / ★★ / ★ / ✕, dropped on the
//! active take of the vocal track as REAPER's own rank markers (`:)))`,
//! `:))`, `:)`, `:(` — [`daw_proto::TakeRating`]). They ARE the take's
//! review: REAPER shows and comps by them, and the session reads its
//! review back from the take markers.
//!
//! Two scopes, one marker:
//! - [`rate_at`] — a moment: the marker at the playhead, on the take that
//!   covers it (while he sings, or scrubbed back to afterwards).
//! - [`rate_take`] — the whole take: the marker at the take's start (the
//!   prompt at the end of a take).
//!
//! The active take is the vocal track's: the top-level folder whose name
//! says vocals, an armed track in it first (the one just recorded to).

use daw_proto::TakeRating;

/// A rating, best first — the order the buttons are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Three,
    Two,
    One,
    Miss,
}

impl Rating {
    pub const ALL: [Self; 4] = [Self::Three, Self::Two, Self::One, Self::Miss];

    /// What its button says.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Three => "★★★",
            Self::Two => "★★",
            Self::One => "★",
            Self::Miss => "✕",
        }
    }

    /// REAPER's rank for it.
    #[must_use]
    pub const fn rank(self) -> TakeRating {
        match self {
            Self::Three => TakeRating::UpRank(3),
            Self::Two => TakeRating::UpRank(2),
            Self::One => TakeRating::UpRank(1),
            Self::Miss => TakeRating::DownRank,
        }
    }

    /// Its colour: gold for the stars, red for a miss.
    #[must_use]
    pub const fn color(self) -> &'static str {
        match self {
            Self::Miss => "#f87171",
            _ => "#fbbf24",
        }
    }
}

/// Rate the moment at `at` (project seconds) on the vocal take under it.
pub fn rate_at(rating: Rating, at: f64) {
    run(async move {
        let Some((take, _)) = vocal_take(at).await else {
            tracing::warn!(
                rating.at = at,
                "take-rating: no vocal take under the playhead"
            );
            return;
        };
        mark(&take, rating, at).await;
    });
}

/// Rate the whole vocal take that covers `at` — the one just recorded,
/// given where recording started.
pub fn rate_take(rating: Rating, at: f64) {
    run(async move {
        let Some((take, start)) = vocal_take(at).await else {
            tracing::warn!(rating.at = at, "take-rating: no vocal take to rate");
            return;
        };
        mark(&take, rating, start).await;
    });
}

async fn mark(take: &daw::rpc::TakeHandle, rating: Rating, at: f64) {
    let position = daw_proto::Position::from_time(daw_proto::PositionInSeconds::from_seconds(at));
    match take
        .add_marker_at(position, &rating.rank().to_marker_name(), None)
        .await
    {
        Ok(Some(_)) => tracing::info!(
            rating.label = rating.label(),
            rating.at = at,
            "take-rating: marked"
        ),
        Ok(None) => tracing::warn!(rating.at = at, "take-rating: outside the take"),
        Err(e) => tracing::warn!(error = %e, "take-rating: the marker was refused"),
    }
}

/// The active take of the vocal track at `at`, and where its item starts.
async fn vocal_take(at: f64) -> Option<(daw::rpc::TakeHandle, f64)> {
    let daw = daw::rpc::Daw::try_get()?;
    let project = daw.current_project().await.ok()?;
    let tracks = project.tracks().all().await.ok()?;
    let vocal = vocal_tracks(&tracks);
    if vocal.is_empty() {
        return None;
    }
    let items = project.items().all().await.ok()?;
    // Under `at`, on a vocal track; an armed one first, then the latest.
    let item = items
        .iter()
        .filter_map(|item| {
            let rank = vocal
                .iter()
                .position(|(guid, _)| *guid == item.track_guid)?;
            let from = item.position.as_seconds();
            let to = from + item.length.as_seconds();
            (from <= at && at < to).then_some((vocal[rank].1, from, item))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))?;
    let handle = project.items().by_guid(&item.2.guid).await.ok()??;
    Some((handle.active_take(), item.1))
}

/// The vocal tracks, by guid, and whether each is armed: the tracks under
/// the top-level track whose name says vocals — a folder's children, or
/// that track itself when it is a plain track.
fn vocal_tracks(tracks: &[daw_proto::Track]) -> Vec<(String, bool)> {
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let top = |track: &daw_proto::Track| {
        let mut at = track;
        while let Some(parent) = at.parent_guid.as_deref().and_then(|g| by_guid.get(g)) {
            at = parent;
        }
        at.name.to_lowercase()
    };
    let is_vocal = |name: &str| name.contains("vocal") || name.contains("vox");
    let mut out: Vec<(String, bool)> = tracks
        .iter()
        .filter(|t| !t.is_folder && is_vocal(&top(t)))
        .map(|t| (t.guid.clone(), t.armed))
        .collect();
    if out.is_empty() {
        out = tracks
            .iter()
            .filter(|t| is_vocal(&t.name.to_lowercase()))
            .map(|t| (t.guid.clone(), t.armed))
            .collect();
    }
    out
}

/// Run `work` against the facade, off the event loop.
fn run(work: impl std::future::Future<Output = ()> + 'static + MaybeSend) {
    #[cfg(not(feature = "native"))]
    wasm_bindgen_futures::spawn_local(work);
    #[cfg(feature = "native")]
    {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        runtime.spawn(work);
    }
}

#[cfg(feature = "native")]
trait MaybeSend: Send {}
#[cfg(feature = "native")]
impl<T: Send> MaybeSend for T {}
#[cfg(not(feature = "native"))]
trait MaybeSend {}
#[cfg(not(feature = "native"))]
impl<T> MaybeSend for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(
        guid: &str,
        name: &str,
        parent: Option<&str>,
        folder: bool,
        armed: bool,
    ) -> daw_proto::Track {
        daw_proto::Track {
            guid: guid.into(),
            name: name.into(),
            parent_guid: parent.map(Into::into),
            is_folder: folder,
            armed,
            ..daw_proto::Track::default()
        }
    }

    #[test]
    fn the_vocal_tracks_are_the_ones_under_the_vocals_folder() {
        let tracks = [
            track("d", "Drums", None, true, false),
            track("k", "Kick", Some("d"), false, false),
            track("v", "VOCALS", None, true, false),
            track("l", "Lead", Some("v"), false, true),
            track("b", "BGV", Some("v"), false, false),
        ];
        assert_eq!(
            vocal_tracks(&tracks),
            vec![("l".to_owned(), true), ("b".to_owned(), false)]
        );
    }

    #[test]
    fn each_rating_is_reapers_rank_marker() {
        let names: Vec<String> = Rating::ALL
            .iter()
            .map(|r| r.rank().to_marker_name())
            .collect();
        assert_eq!(names, [":)))", ":))", ":)", ":("]);
    }
}
