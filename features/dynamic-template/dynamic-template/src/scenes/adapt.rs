//! Building facts: the adapters on the input side of the seam.
//!
//! Two of them, which is what makes the seam real rather than
//! hypothetical: the golden fixture's own tree, and a live track list
//! with the template's ext-state read back beside it. The engine never
//! learns which one it is talking to.

use std::collections::HashMap;

use daw_proto::Track;

use super::facts::{path_of, Fact, Segment};
use super::language::{classify_language, Language};
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
/// open folder so a track's own path is its parent's plus its own step,
/// and separately the language of each open Language folder so a
/// source under one inherits it without repeating the name.
#[derive(Default)]
struct Walk {
    /// `(depth, path)` for every folder currently open.
    open: Vec<(u32, Vec<Segment>)>,
    /// `(depth, language)` for every open folder that IS a language —
    /// `flow.vocals.language`'s folder-inheritance case.
    language_open: Vec<(u32, Language)>,
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
        self.language_open.retain(|(at, _)| *at < depth);
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
        // The track's own name says a language first; failing that, the
        // nearest enclosing Language folder does.
        // r[impl flow.vocals.language]
        let own_language = classify_language(name);
        let language = own_language.or_else(|| self.language_open.last().map(|(_, l)| *l));
        if is_folder {
            if let Some(own_language) = own_language {
                self.language_open.push((depth, own_language));
            }
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
            language,
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

    /// A source track named for a language classifies to it directly —
    /// `Vocals / Ron / Main / EN`.
    #[test]
    // r[verify flow.vocals.language]
    fn a_named_source_carries_its_own_language() {
        use crate::scenes::Language;

        let mut walk = Walk::default();
        let mut node = |guid: &'static str, name: &'static str, depth: u32, is_folder: bool| {
            walk.step(
                0,
                &Node {
                    guid,
                    name,
                    depth,
                    is_folder,
                    kind: None,
                    template: None,
                },
            )
        };
        node("vocals", "Vocals", 0, true);
        node("ron", "Ron", 1, true);
        node("main", "Main", 2, true);
        let en = node("en", "EN", 3, false);
        let es = node("es", "ES", 3, false);
        assert_eq!(en.language, Some(Language::En));
        assert_eq!(es.language, Some(Language::Es));
    }

    /// A Language folder's children inherit it without repeating the
    /// name — the comp stack under `EN` (`COMP`/`EDIT`/`TUNE`) is
    /// English because the folder above it is, not because any of them
    /// is named for a language.
    #[test]
    // r[verify flow.vocals.language]
    fn a_language_folders_children_inherit_it() {
        use crate::scenes::Language;

        let mut walk = Walk::default();
        let mut node = |guid: &'static str, name: &'static str, depth: u32, is_folder: bool| {
            walk.step(
                0,
                &Node {
                    guid,
                    name,
                    depth,
                    is_folder,
                    kind: None,
                    template: None,
                },
            )
        };
        node("vocals", "Vocals", 0, true);
        node("ron", "Ron", 1, true);
        node("main", "Main", 2, true);
        node("en", "EN", 3, true); // a Language folder, not a leaf
        let comp = node("comp", "COMP", 4, false);
        let edit = node("edit", "EDIT", 4, false);
        let tune = node("tune", "TUNE", 4, false);
        assert_eq!(comp.language, Some(Language::En));
        assert_eq!(edit.language, Some(Language::En));
        assert_eq!(tune.language, Some(Language::En));
    }

    /// Closing a language folder and opening a sibling one stops the
    /// first's inheritance — a source under `ES` is Spanish, not
    /// English, even though both sit under the same `Main`.
    #[test]
    // r[verify flow.vocals.language]
    fn closing_a_language_folder_stops_its_inheritance() {
        let mut walk = Walk::default();
        let mut node = |guid: &'static str,
                        name: &'static str,
                        depth: u32,
                        is_folder: bool| {
            walk.step(
                0,
                &Node {
                    guid,
                    name,
                    depth,
                    is_folder,
                    kind: None,
                    template: None,
                },
            )
        };
        node("main", "Main", 0, true);
        node("en", "EN", 1, true);
        node("en_comp", "COMP", 2, false);
        let es_comp = node("es_comp", "COMP", 1, false); // EN closed: depth back to 1
        assert_eq!(es_comp.language, None, "no language folder is open here");
    }

    /// The mix tracks above the language dimension — the performer's
    /// own folder, `Main`, `DBL` — carry no language at all, which is
    /// what keeps them in view whatever language is active.
    #[test]
    // r[verify flow.vocals.language]
    fn the_mix_tracks_above_the_dimension_carry_no_language() {
        let facts = from_flat(&flatten(&maximal()));
        for name in ["Vocals", "Lead", "Doubles", "Harmonies"] {
            let fact = facts.iter().find(|f| f.name == name);
            if let Some(fact) = fact {
                assert_eq!(fact.language, None, "{name} should carry no language");
            }
        }
    }
}
