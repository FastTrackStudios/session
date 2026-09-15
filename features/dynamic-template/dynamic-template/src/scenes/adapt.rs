//! Building facts: the adapters on the input side of the seam.
//!
//! Two of them, which is what makes the seam real rather than
//! hypothetical: the golden fixture's own tree, and a live track list
//! with the template's ext-state read back beside it. The engine never
//! learns which one it is talking to.

use std::collections::HashMap;

use daw_proto::Track;

use super::facts::{path_of, Fact, Segment};
use crate::golden_session::kind::TrackExt;
use crate::golden_session::rpp::Flat;
use crate::golden_session::Kind;

/// One half of a stereo pair, which the engine always rails.
///
/// A pair — OH, Rooms, a piano — is ONE stereo track
/// (`flow.scenes.reaper-model`); a session that has split it into an L
/// and an R has two rows for one signal, and the pair's folder is what
/// carries the width.
#[must_use]
pub fn is_pair_half(name: &str) -> bool {
    matches!(
        name.trim().to_uppercase().as_str(),
        "L" | "R" | "LEFT" | "RIGHT"
    )
}

/// Facts from the golden session's own tree.
///
/// The authoritative source: the fixture tree carries every kind and
/// every template group directly, with no parse in between, which is
/// what makes the fixture test a test of the ENGINE rather than of a
/// project-file reader.
#[must_use]
pub fn from_flat(flats: &[Flat]) -> Vec<Fact> {
    let mut walk = Walk::default();
    flats
        .iter()
        .enumerate()
        .map(|(index, flat)| {
            let template: Option<Vec<String>> = flat
                .template
                .map(|path| path.iter().map(|s| (*s).to_owned()).collect());
            walk.step(
                index,
                &Node {
                    guid: &flat.guid,
                    name: &flat.name,
                    depth: flat.depth,
                    is_folder: flat.is_folder,
                    kind: Some(flat.kind),
                    template: template.as_deref(),
                },
            )
        })
        .collect()
}

/// Facts from a live track list, with the template's ext-state beside
/// it.
///
/// `ext` is what [`crate::golden_session::read_kinds`] read back from
/// the project file, keyed by GUID. A track it says nothing about is a
/// track the user added: it has no kind and no template group, which is
/// exactly when a selector's `name` is the right escape hatch.
#[must_use]
pub fn from_tracks<S: std::hash::BuildHasher>(
    rows: &[(Track, u32)],
    ext: &HashMap<String, TrackExt, S>,
) -> Vec<Fact> {
    let mut walk = Walk::default();
    rows.iter()
        .enumerate()
        .map(|(index, (track, depth))| {
            let known = ext.get(&track.guid);
            let template: Option<Vec<String>> = known
                .and_then(|e| e.group.as_deref())
                .map(|g| g.split('/').map(str::to_owned).collect());
            walk.step(
                index,
                &Node {
                    guid: &track.guid,
                    name: &track.name,
                    depth: *depth,
                    is_folder: track.is_folder,
                    kind: known.and_then(|e| e.kind),
                    template: template.as_deref(),
                },
            )
        })
        .collect()
}

/// One track as the walk sees it, before its path is worked out.
struct Node<'a> {
    guid: &'a str,
    name: &'a str,
    depth: u32,
    is_folder: bool,
    kind: Option<Kind>,
    template: Option<&'a [String]>,
}

/// The tree walk both adapters share: it remembers the path of each
/// open folder so a track's own path is its parent's plus its own step.
#[derive(Default)]
struct Walk {
    /// `(depth, path)` for every folder currently open.
    open: Vec<(u32, Vec<Segment>)>,
}

impl Walk {
    fn step(&mut self, index: usize, node: &Node<'_>) -> Fact {
        let Node {
            guid,
            name,
            depth,
            is_folder,
            kind,
            template,
        } = *node;
        self.open.retain(|(at, _)| *at < depth);
        let parent: Vec<Segment> = self
            .open
            .last()
            .map(|(_, path)| path.clone())
            .unwrap_or_default();
        let mut own = Segment::named(name);
        own.kind = kind;
        if let Some(last) = template.and_then(<[String]>::last) {
            own.template = Some(last.clone());
        }
        let path = path_of(&parent, Some(&own), template);
        if is_folder {
            self.open.push((depth, path.clone()));
        }
        Fact {
            guid: guid.to_owned(),
            name: name.to_owned(),
            index: u32::try_from(index).unwrap_or(u32::MAX),
            depth,
            is_folder,
            kind,
            // A leaf does not extend the path it sits in: its own step
            // is its kind and its name, which the selector matches
            // directly rather than through the path.
            path: if is_folder { path } else { parent },
            performer: None,
            layer: None,
            channel: None,
            multi_mic: None,
            arrangement: None,
            pair_half: is_pair_half(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden_session::{maximal, rpp::flatten};

    /// Every template folder of the golden session can be reached by the
    /// kind the template wrote, without naming it — which is the whole
    /// claim the selectors rest on.
    #[test]
    fn the_golden_session_carries_its_taxonomy() {
        let facts = from_flat(&flatten(&maximal()));
        let by = |name: &str| {
            facts
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("no {name}"))
        };
        assert_eq!(by("Process").kind, Some(Kind::Process));
        assert_eq!(by("Compress").kind, Some(Kind::Compress));
        assert!(
            by("Dry").path.iter().any(|s| s.answers_to("compress")),
            "a compressor sits under a folder that answers to its kind"
        );
        assert!(by("In").path.iter().any(|s| s.answers_to("Kick")));
        assert!(
            by("Room Sim")
                .path
                .iter()
                .zip(["process", "fx"])
                .all(|(seg, want)| seg.answers_to(want)),
            "{:?}",
            by("Room Sim").path
        );
    }

    /// A leaf's path is where it sits, not a step of its own — so the
    /// mics of a Sum and the Sum share a path and a selector reaches
    /// both.
    #[test]
    fn a_leaf_sits_in_its_folders_path() {
        let facts = from_flat(&flatten(&maximal()));
        let sum = facts
            .iter()
            .find(|f| f.name == "Sum" && f.is_folder)
            .expect("a Sum");
        let mic = facts.iter().find(|f| f.name == "In").expect("a mic");
        assert_eq!(sum.path, mic.path);
    }
}
