//! A collapsed folder's own items, folded from its children's.
//!
//! # What this is for
//!
//! A drum kit is a folder tree because that is how the audio has to be
//! routed: a kick is a Sum with three mics under it, a tom is its mic
//! and its trigger. But the routing is a mix concern, and a mix concern
//! is not what an arrangement is for. While you are editing, a kick is
//! **one thing** — the mics were recorded together, comped together and
//! trimmed together, so what you want on screen is one row per drum with
//! the take on it, and the mics available when you go looking for them.
//!
//! REAPER draws nothing on a closed folder, so folding the kit away
//! today means folding the audio away with it. This is what fills that
//! row: the folder keeps its own track control panel — its name, its
//! fader, its colour, because the mix happens there — and the items on
//! it are its children's.
//!
//! # The rule
//!
//! Every boundary of every child, in order; the row is the spans between
//! them that at least one child is sounding across.
//!
//! One rule, and the two cases the desk cares about fall out of it
//! rather than being special-cased:
//!
//! - **The mics agree**, which is the normal case, because they are one
//!   performance. Their boundaries are the same boundaries, so the spans
//!   are the items, and the row looks exactly like the take.
//! - **They do not**, because one mic was punched in, or a trigger lane
//!   was trimmed on its own. Then the extra boundaries appear as extra
//!   spans, and the row says so. It is not a lie about a clean take and
//!   it is not a refusal to draw; it is the edit that is actually there.
//!
//! [`Span::whole`] is what tells the two apart, for anything downstream
//! that wants to draw the second differently.
//!
//! # Why items and not a picture
//!
//! [`crate::studio`]'s sibling `folder_item` module folds children's
//! PEAKS onto a column grid and draws one continuous picture across the
//! folder's whole extent. That is the right answer for an overview and
//! the wrong one here: a picture has no edges to grab, no title, and no
//! gaps between takes. These are real items, so the row hit-tests,
//! labels and selects like every other row, and an edit has somewhere to
//! land.
//!
//! # Editing, which is the point and is not built yet
//!
//! **An edit on a folded item is an edit on every child item under it.**
//! Drag the kick's take and its three mics move together; trim its edge
//! and they all trim to the same boundary; delete it and they all go.
//! That is the whole reason this row is worth having — a view you can
//! read but not work in is a view you stop using — and it is decided,
//! not speculative. It is simply not written yet.
//!
//! Everything it needs is here. A folded item's guid carries the folder
//! and the span ([`guid_of`], [`parse_guid`]), and the span carries the
//! real items ([`Span::from`]). So the path is: the hit test lands on a
//! folded guid, [`parse_guid`] says which fold, [`Span::from`] says which
//! items, and the edit is applied to all of them as ONE undo step.
//!
//! What is genuinely undecided is the ragged case — what trimming the
//! edge of a span where [`Span::whole`] is false should do, when only
//! some of the children start there. That, the undo grouping, and what
//! take lanes mean on a row that has no takes of its own are written up
//! on issue #114.
//!
//! Until then, one invariant holds absolutely: a `folded:` guid names
//! nothing in the session and must never reach the backend. It is a view
//! coordinate, and writing one would address an item that does not
//! exist.

use std::collections::HashMap;

use daw_proto::Item;

/// One stretch of a folded row: a span with children sounding across it.
#[derive(Clone, PartialEq, Debug)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    /// The child items sounding across it.
    ///
    /// Kept rather than counted because this is the only record of what
    /// a folded item actually IS. It is also the list an edit works
    /// through: moving, trimming or deleting the folded item means doing
    /// the same to every guid in here, in one undo step — see the module
    /// doc. In project order, so the list is stable between renders and
    /// two equal folds compare equal.
    pub from: Vec<String>,
    /// Whether every child sounding here starts and ends exactly here.
    ///
    /// True is the ordinary case and means the span is a take: one item,
    /// no edges of its own. False means the children disagreed and this
    /// is a piece of the disagreement — worth drawing as a fragment
    /// rather than as a take, because that is what it is.
    pub whole: bool,
}

impl Span {
    /// How long it lasts.
    #[must_use]
    pub fn length(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// The shortest span worth drawing, in seconds.
///
/// Two children whose boundaries differ by a microsecond are two
/// children that agree; the difference is a rounding in whatever wrote
/// the file, not an edit anybody made. Without a floor here every such
/// pair would produce a third span a micron wide between them, and a
/// clean take would render as three items.
const GRAIN: f64 = 0.001;

/// Fold a folder's children's items into the items its own row shows.
///
/// `children` is one slice per child track, each in position order.
#[must_use]
pub fn spans(children: &[&[Item]]) -> Vec<Span> {
    // Every edge anybody has, in order and without duplicates. The
    // spans are what lies between consecutive edges, so this is the
    // whole algorithm — the rest is asking who is sounding where.
    let mut edges: Vec<f64> = Vec::new();
    for lane in children {
        for item in *lane {
            edges.push(item.position.as_seconds());
            edges.push(item.position.as_seconds() + item.length.as_seconds());
        }
    }
    edges.sort_by(f64::total_cmp);
    edges.dedup_by(|a, b| (*a - *b).abs() < GRAIN);

    let mut out: Vec<Span> = Vec::new();
    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if end - start < GRAIN {
            continue;
        }
        // Who covers the MIDDLE, not who touches an edge: an item that
        // ends exactly where this span starts is not sounding in it.
        let middle = f64::midpoint(start, end);
        let mut from = Vec::new();
        let mut whole = true;
        for lane in children {
            for item in *lane {
                let at = item.position.as_seconds();
                let to = at + item.length.as_seconds();
                if at < middle && middle < to {
                    whole &= (at - start).abs() < GRAIN && (to - end).abs() < GRAIN;
                    from.push(item.guid.clone());
                }
            }
        }
        if from.is_empty() {
            continue;
        }
        out.push(Span {
            start,
            end,
            from,
            whole,
        });
    }
    out
}

/// The folded spans as items on the folder's own row.
///
/// Real [`Item`]s, and deliberately: everything that draws, labels,
/// hit-tests or selects a lane already works on items, and a second kind
/// of thing on a row would have to be taught to all of it.
///
/// The guid is derived from the folder and the span's place in it, which
/// makes it stable across a re-fold — a shape cache and a selection both
/// key on it — and recoverable, which is how an edit gets from the row
/// back to the mics under it. Nothing in the session has this guid, so
/// it can never be mistaken for a real item and written to REAPER.
#[must_use]
pub fn lane(folder: &daw_proto::Track, spans: &[Span]) -> Vec<Item> {
    spans
        .iter()
        .enumerate()
        .map(|(index, span)| Item {
            guid: guid_of(&folder.guid, index),
            track_guid: folder.guid.clone(),
            index: u32::try_from(index).unwrap_or(u32::MAX),
            position: daw_proto::PositionInSeconds::from_seconds(span.start),
            length: daw_proto::Duration::from_seconds(span.length()),
            // The folder's own colour and the folder's own name, because
            // the row IS the folder: its fader is what moves and its
            // name is what is written beside it. Taking a child's would
            // say the row belonged to one of the mics — a kick whose
            // takes are labelled `In 1` is a kick that looks like its
            // inside mic.
            color: folder.color,
            label: Some(format!("{} {}", folder.name, index.saturating_add(1))),
            ..Item::default()
        })
        .collect()
}

/// The guid a folded item gets.
#[must_use]
pub fn guid_of(folder_guid: &str, index: usize) -> String {
    format!("folded:{folder_guid}:{index}")
}

/// The folder a folded guid came from, and which of its spans.
///
/// The way back. An edit lands on a row, the row hands over a guid, and
/// this says which fold it belongs to — from there [`Span::from`] says
/// which real items have to move.
#[must_use]
pub fn parse_guid(guid: &str) -> Option<(&str, usize)> {
    let rest = guid.strip_prefix("folded:")?;
    let (folder, index) = rest.rsplit_once(':')?;
    Some((folder, index.parse().ok()?))
}

/// Every collapsed folder's folded lane, by folder guid.
///
/// `descendants` answers which tracks are under a folder — the caller's
/// job, because it is the caller that knows the tree and the collapse
/// state, and this module is meant to be testable with items written by
/// hand.
#[must_use]
pub fn lanes<'a>(
    folders: impl IntoIterator<Item = &'a daw_proto::Track>,
    descendants: &dyn Fn(&str) -> Vec<&'a [Item]>,
) -> HashMap<String, Vec<Item>> {
    folders
        .into_iter()
        .filter_map(|folder| {
            let children = descendants(&folder.guid);
            let spans = spans(&children);
            (!spans.is_empty()).then(|| (folder.guid.clone(), lane(folder, &spans)))
        })
        .collect()
}

/// Every track under `folder`, however deep.
#[must_use]
pub fn under<'a>(tracks: &'a [daw_proto::Track], folder: &str) -> Vec<&'a daw_proto::Track> {
    // By parent links rather than by the depth deltas beside them: a
    // session that has been dragged about has correct parents and
    // occasionally a stale depth, and a wrong answer here puts another
    // drum's take on this drum's row.
    let mut inside: std::collections::HashSet<&str> = std::collections::HashSet::new();
    inside.insert(folder);
    let mut out = Vec::new();
    // One pass in project order works because a child never precedes
    // its parent in a session file.
    for track in tracks {
        let parent = track.parent_guid.as_deref();
        if parent.is_some_and(|p| inside.contains(p)) {
            inside.insert(&track.guid);
            out.push(track);
        }
    }
    out
}

/// The folders on screen whose contents are not.
///
/// A scene that collapses a folder drops its rows and leaves the folder
/// itself, which is how REAPER ends up drawing an empty lane where the
/// kit used to be. These are the rows that want filling.
#[must_use]
pub fn shut<'a>(
    tracks: &'a [daw_proto::Track],
    shown: &[daw_proto::Track],
) -> Vec<&'a daw_proto::Track> {
    let on_screen: std::collections::HashSet<&str> =
        shown.iter().map(|t| t.guid.as_str()).collect();
    shown
        .iter()
        .filter(|track| track.is_folder)
        .filter_map(|track| tracks.iter().find(|t| t.guid == track.guid))
        .filter(|folder| {
            let inside = under(tracks, &folder.guid);
            !inside.is_empty() && !inside.iter().any(|t| on_screen.contains(t.guid.as_str()))
        })
        .collect()
}

/// Fold several children's peaks into the one the folder's row shows.
///
/// The loudest of them at each point, not the mean. A mean of three
/// mics of one drum is quieter than any of them, which draws a kit as
/// though it had been recorded further away the more microphones were
/// on it; the maximum is never narrower than any child and is what the
/// arrangement's own folder items settled on for the same reason.
///
/// Resampled onto the longest child's grid, because the children are one
/// performance and their peak lists are the same performance at whatever
/// resolution each was read at.
#[must_use]
pub fn wave(children: &[&[f32]]) -> Vec<f32> {
    let width = children.iter().map(|c| c.len()).max().unwrap_or(0);
    if width == 0 {
        return Vec::new();
    }
    (0..width)
        .map(|i| {
            children
                .iter()
                .filter(|child| !child.is_empty())
                .map(|child| {
                    // Where this bin falls in a child of a different
                    // length: the same fraction of the performance.
                    let at = i
                        .saturating_mul(child.len())
                        .checked_div(width)
                        .unwrap_or(0)
                        .min(child.len().saturating_sub(1));
                    child.get(at).copied().unwrap_or(0.0)
                })
                .fold(0.0_f32, f32::max)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Span, lane, parse_guid, spans};
    use daw_proto::{Duration, Item, PositionInSeconds};

    fn item(guid: &str, at: f64, len: f64) -> Item {
        Item {
            guid: guid.to_owned(),
            position: PositionInSeconds::from_seconds(at),
            length: Duration::from_seconds(len),
            ..Item::default()
        }
    }

    /// The case the feature exists for: three mics of one drum, recorded
    /// and comped together, so their boundaries are the same boundaries.
    /// The row is the take — two items, not six, and no edges invented
    /// between them.
    #[test]
    fn mics_that_agree_fold_to_the_take() {
        let top = [item("top1", 0.0, 4.0), item("top2", 8.0, 4.0)];
        let bottom = [item("bot1", 0.0, 4.0), item("bot2", 8.0, 4.0)];
        let trig = [item("trg1", 0.0, 4.0), item("trg2", 8.0, 4.0)];
        let folded = spans(&[&top[..], &bottom[..], &trig[..]]);
        assert_eq!(folded.len(), 2, "{folded:?}");
        assert!((folded[0].start - 0.0).abs() < 1e-9);
        assert!((folded[0].end - 4.0).abs() < 1e-9);
        assert!((folded[1].start - 8.0).abs() < 1e-9);
        assert!(folded.iter().all(|s| s.whole), "a clean take reads ragged");
        assert_eq!(folded[0].from.len(), 3, "the row lost a mic");
    }

    /// The gap between two takes stays a gap. Nothing is sounding there,
    /// so nothing is drawn there — a folded row that bridged its takes
    /// would claim audio the session does not have.
    #[test]
    fn a_gap_between_takes_is_not_filled_in() {
        let one = [item("a", 0.0, 4.0), item("b", 8.0, 4.0)];
        let folded = spans(&[&one[..]]);
        assert_eq!(folded.len(), 2);
        assert!((folded[0].end - 4.0).abs() < 1e-9);
        assert!((folded[1].start - 8.0).abs() < 1e-9);
    }

    /// A mic punched in on its own splits the row where it starts, and
    /// says which part of the span is the clean take and which is the
    /// punch. Three spans out of two items, because that is the edit
    /// that is actually there.
    #[test]
    fn a_punch_on_one_mic_shows_as_its_own_span() {
        let top = [item("top", 0.0, 8.0)];
        let punch = [item("punch", 2.0, 4.0)];
        let folded = spans(&[&top[..], &punch[..]]);
        assert_eq!(folded.len(), 3, "{folded:?}");
        assert!(!folded[0].whole, "a fragment claimed to be a take");
        assert_eq!(folded[0].from, ["top"]);
        assert_eq!(folded[1].from, ["top", "punch"]);
        assert_eq!(folded[2].from, ["top"]);
        assert!((folded[1].start - 2.0).abs() < 1e-9);
        assert!((folded[1].end - 6.0).abs() < 1e-9);
    }

    /// Boundaries a micron apart are boundaries that agree. Whatever
    /// wrote the file rounded; nobody made that edit, and a span a
    /// micron wide between two mics is not a thing to draw.
    #[test]
    fn a_rounding_is_not_an_edit() {
        let a = [item("a", 0.0, 4.0)];
        let b = [item("b", 0.000_02, 3.999_97)];
        let folded = spans(&[&a[..], &b[..]]);
        assert_eq!(folded.len(), 1, "{folded:?}");
        assert!(folded[0].whole);
    }

    /// A folder with nothing under it folds to nothing rather than to
    /// one empty item.
    #[test]
    fn an_empty_folder_folds_to_nothing() {
        assert!(spans(&[]).is_empty());
        assert!(spans(&[&[][..], &[][..]]).is_empty());
    }

    /// A zero-length item is not a span. It has no middle to be inside
    /// of, and a row of them would be a row of invisible items that
    /// still hit-test.
    #[test]
    fn a_zero_length_item_is_not_a_span() {
        let none = [item("z", 4.0, 0.0)];
        assert!(spans(&[&none[..]]).is_empty());
    }

    /// The loudest mic, not the average of them. Three mics of one drum
    /// averaged are quieter than any of them, which would draw a kit as
    /// though more microphones had moved it further away.
    #[test]
    fn a_folded_wave_takes_the_loudest_mic() {
        let quiet = [0.1_f32, 0.2, 0.1];
        let loud = [0.9_f32, 0.1, 0.4];
        assert_eq!(super::wave(&[&quiet[..], &loud[..]]), [0.9, 0.2, 0.4]);
    }

    /// Children read at different resolutions are the same performance,
    /// so the shorter one is stretched onto the longer one's grid rather
    /// than truncating it.
    #[test]
    fn a_shorter_peak_list_is_stretched_not_cut() {
        let coarse = [1.0_f32, 0.0];
        let fine = [0.0_f32, 0.0, 0.0, 0.5];
        let folded = super::wave(&[&coarse[..], &fine[..]]);
        assert_eq!(folded.len(), 4, "{folded:?}");
        assert!((folded[0] - 1.0).abs() < f32::EPSILON);
        assert!((folded[3] - 0.5).abs() < f32::EPSILON);
    }

    /// Nothing to fold folds to nothing, rather than to a row of silence
    /// that still draws a flat line.
    #[test]
    fn nothing_folds_to_nothing() {
        assert!(super::wave(&[]).is_empty());
        assert!(super::wave(&[&[][..]]).is_empty());
    }

    /// The guid a folded item gets leads back to the fold it came from,
    /// which is how an edit on the row reaches the mics under it.
    #[test]
    fn a_folded_guid_says_where_it_came_from() {
        let folder = daw_proto::Track {
            guid: "kick".to_owned(),
            name: "Kick".to_owned(),
            ..daw_proto::Track::default()
        };
        let made = lane(
            &folder,
            &[Span {
                start: 1.0,
                end: 3.0,
                from: vec!["top".to_owned()],
                whole: true,
            }],
        );
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].track_guid, "kick");
        assert_eq!(
            made[0].label.as_deref(),
            Some("Kick 1"),
            "the row lost its name"
        );
        assert!((made[0].length.as_seconds() - 2.0).abs() < 1e-9);
        assert_eq!(parse_guid(&made[0].guid), Some(("kick", 0)));
        assert_eq!(parse_guid("not-folded"), None);
    }
}
