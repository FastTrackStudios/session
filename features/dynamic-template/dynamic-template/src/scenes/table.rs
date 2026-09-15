//! The scenes themselves, as data.
//!
//! Nine scenes and a common prelude, written as [`Scene`] values rather
//! than as functions. Rust first (spec #48): every type derives `Facet`,
//! so the styx step that follows is a loader and nothing else — this
//! table becomes the built-in layer under a user's and a session's.
//!
//! Every selector says what a track IS rather than what it is called.
//! `Process`, `Compress`, `FX`, `Verb`, `MIX BUS`, `Guide`, `Keyflow`
//! and `Headphones` are taxonomy kinds the template writes when it
//! creates a folder, so renaming `Process` to `Parallel` changes nothing
//! about which scene finds it. A `name` appears only where the track is
//! a return a user picked — `Fat Plate` is a choice of unit, not a
//! structure.

use std::sync::OnceLock;

use super::language::Language;
use super::selector::{Role, Selector};
use super::types::{Audience, Effect, Fold, GroupBy, Rule, Scene, Size};

/// A selector on a taxonomy path prefix.
fn under(path: &[&str]) -> Selector {
    Selector {
        group: path.iter().map(|s| (*s).to_owned()).collect(),
        ..Selector::default()
    }
}

/// Every scene, in the order the number keys recall them.
///
/// Built once and shared: a table is a constant in spirit, but `Scene`
/// owns its strings so that the styx loader can produce the same type.
#[must_use]
pub fn scenes() -> &'static [Scene] {
    static SCENES: OnceLock<Vec<Scene>> = OnceLock::new();
    SCENES.get_or_init(build)
}

/// The scene a slug names.
#[must_use]
pub fn scene(slug: &str) -> Option<&'static Scene> {
    scenes().iter().find(|s| s.slug == slug)
}

/// The common prelude: what is true of every scene until it says
/// otherwise.
///
/// `flow.scenes.guide-folder` and `flow.scenes.keyflow-folder`: the
/// Guide folder sits at the top of every session and every scene shows
/// it collapsed unless a flow opens it, and the Keyflow folder under it
/// is collapsed in every scene — except in **Write** and **Produce**,
/// the two modes whose whole subject is the song's knowledge.
///
/// Data and overridable: these are ordinary rules, and they go first, so
/// a scene about the click can open the Guide folder by saying so.
///
/// `active_language` (`flow.vocals.language.active`) adds one hide rule
/// per sung language that is not the active one: `Selector::language`
/// needs no negation for this because the vocabulary is closed and
/// small — "not the active language" is just every *other* language,
/// enumerated. `None` (no switch has happened yet) adds none, which is
/// "hide nothing" rather than a guess.
// r[impl flow.scenes.guide-folder]
// r[impl flow.scenes.keyflow-folder]
// r[impl flow.vocals.language.active]
#[must_use]
pub fn prelude(mode: Option<&str>, active_language: Option<Language>) -> Vec<Rule> {
    let collapsed = Effect::at(Size::Compact).folded(Fold::Collapsed);
    let mut rules = vec![
        Rule::new(
            Selector {
                kind: Some("guide".to_owned()),
                ..Selector::default()
            },
            collapsed,
        ),
        Rule::new(
            Selector {
                kind: Some("keyflow".to_owned()),
                ..Selector::default()
            },
            collapsed,
        ),
    ];
    if matches!(mode, Some("write" | "produce")) {
        rules.push(Rule::new(
            Selector {
                kind: Some("keyflow".to_owned()),
                ..Selector::default()
            },
            Effect::default().folded(Fold::Open),
        ));
    }
    if let Some(active) = active_language {
        for language in Language::SUNG.into_iter().filter(|&l| l != active) {
            rules.push(Rule::new(
                Selector {
                    language: Some(language),
                    ..Selector::default()
                },
                Effect::hidden(),
            ));
        }
    }
    rules
}

/// The bus tree, out of the way: a scene about the instruments is not
/// about the mix.
///
/// One rule, on the tree's root kind, rather than one per bus: the fold
/// reaches everything under it, and a session with a bus the template
/// did not make still loses it with the rest.
fn hide_the_bus_tree() -> Rule {
    Rule::new(
        Selector {
            kind: Some("mix-bus".to_owned()),
            ..Selector::default()
        },
        Effect::hidden(),
    )
}

/// The five pieces a drum mix is made on — matched by the template
/// groups they stand for, so a kit with a sixth piece needs a line here
/// and a renamed one needs nothing.
const PIECES: [&str; 5] = ["Kick", "Snare", "Toms", "Cymbals", "Rooms"];

/// The kit's taxonomy path.
const KIT: [&str; 2] = ["Drums", "Drum Kit"];

/// One rule per kit piece: the piece folder and everything under it.
fn per_piece(effect: Effect) -> Vec<Rule> {
    PIECES
        .iter()
        .map(|piece| {
            Rule::new(
                Selector {
                    role: Role::Bus,
                    ..under(&[KIT[0], KIT[1], piece])
                },
                effect,
            )
        })
        .collect()
}

fn build() -> Vec<Scene> {
    vec![
        drum_tracking(),
        drum_mixing(),
        drum_overview(),
        drum_advanced(),
        drum_fx(),
        buses(),
        guitar_fx(),
        lead_vocal(),
        lead_vocal_fx(),
    ]
}

/// Tracking: the core microphones you are getting a sound on, and
/// nothing else that needs reading.
// r[impl flow.drums.tracking.full]
// r[impl flow.scenes.follow-mode]
fn drum_tracking() -> Scene {
    Scene {
        name: "Drum Tracking".to_owned(),
        slug: "drum-tracking".to_owned(),
        short: "Trk".to_owned(),
        instrument: "drums".to_owned(),
        modes: vec!["record".to_owned()],
        audience: Audience::Engineer,
        // Tracking sorts by performer (`flow.scenes.performer-order`).
        // The data says so here; emitting the header rows is #51, and
        // that ticket adds the emitting and nothing else.
        group_by: GroupBy::Performer,
        spec: vec![
            "flow.drums.tracking.full".to_owned(),
            "flow.scenes.follow-mode".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules: vec![
            // Everything hanging off the kit is present and unread.
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&KIT)
                },
                Effect::at(Size::Minimum),
            ),
            // Except the close mics of the two pieces a sound is got on
            // first — the ones with a level to watch and a phase to set.
            Rule::new(
                Selector {
                    kind: Some("source".to_owned()),
                    ..under(&[KIT[0], KIT[1], "Kick"])
                },
                Effect::at(Size::Working),
            ),
            Rule::new(
                Selector {
                    kind: Some("source".to_owned()),
                    ..under(&[KIT[0], KIT[1], "Snare"])
                },
                Effect::at(Size::Working),
            ),
            // What the kit is sent to is present and no more: a tracking
            // pass does not touch the parallel chain.
            Rule::new(under(&["process"]), Effect::at(Size::Minimum)),
            hide_the_bus_tree(),
        ],
    }
}

/// Mixing: the pieces are the instrument; every mic is a rail.
// r[impl flow.drums.mixing.scenes]
fn drum_mixing() -> Scene {
    let mut rules = vec![Rule::new(
        Selector {
            role: Role::Leaf,
            ..Selector::default()
        },
        Effect::at(Size::Minimum),
    )];
    rules.extend(per_piece(Effect::at(Size::Working)));
    // A piece's own folders are not the piece: the Sum that holds its
    // mics and the Verb bank beside it are structure, and structure is
    // read at the width of a bus.
    for kind in ["sum", "verb"] {
        rules.push(Rule::new(
            Selector {
                kind: Some(kind.to_owned()),
                role: Role::Bus,
                ..Selector::default()
            },
            Effect::at(Size::Compact),
        ));
    }
    rules.push(Rule::new(
        Selector {
            kind: Some("piece".to_owned()),
            role: Role::Bus,
            ..under(&[KIT[0], KIT[1], "Toms"])
        },
        Effect::at(Size::Compact),
    ));
    rules.push(hide_the_bus_tree());
    Scene {
        name: "Drum Mixing".to_owned(),
        slug: "drum-mixing".to_owned(),
        short: "Mix".to_owned(),
        instrument: "drums".to_owned(),
        modes: vec!["mix".to_owned()],
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.drums.mixing.scenes".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules,
    }
}

/// Overview: the kit as its pieces — each collapsed to one strip at
/// working width, the parallel chain gone. What the kit sounds like,
/// five faders. The player's view of a kit.
// r[impl flow.drums.mixing.scenes]
// r[impl flow.scenes.two-audiences]
fn drum_overview() -> Scene {
    let mut rules = per_piece(Effect::at(Size::Working).folded(Fold::Collapsed));
    rules.push(Rule::new(
        Selector {
            kind: Some("process".to_owned()),
            ..Selector::default()
        },
        Effect::hidden(),
    ));
    rules.push(hide_the_bus_tree());
    Scene {
        name: "Drum Overview".to_owned(),
        slug: "drum-overview".to_owned(),
        short: "Over".to_owned(),
        instrument: "drums".to_owned(),
        modes: vec!["mix".to_owned()],
        audience: Audience::Player,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.drums.mixing.scenes".to_owned(),
            "flow.scenes.two-audiences".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules,
    }
}

/// Advanced: the tracks under the pieces that are not the close mics —
/// each Sub, Fund and Trig, and every verb the kit carries — open, the
/// mics and the parallel chain present.
// r[impl flow.drums.mixing.scenes]
fn drum_advanced() -> Scene {
    let mut rules = vec![Rule::new(
        Selector {
            role: Role::Leaf,
            ..under(&KIT)
        },
        Effect::at(Size::Minimum),
    )];
    for kind in ["trigger", "fundamental", "sub"] {
        rules.push(Rule::new(
            Selector {
                kind: Some(kind.to_owned()),
                ..Selector::default()
            },
            Effect::at(Size::Working),
        ));
    }
    rules.push(Rule::new(
        Selector {
            kind: Some("verb".to_owned()),
            role: Role::Leaf,
            ..Selector::default()
        },
        Effect::at(Size::Working),
    ));
    rules.extend([
        // The instrument returns are not the kit's: they came along for
        // the kind, and this scene is about what hangs off the pieces.
        Rule::new(under(&["fx"]), Effect::at(Size::Compact)),
        Rule::new(under(&["process"]), Effect::at(Size::Minimum)),
        hide_the_bus_tree(),
    ]);
    Scene {
        name: "Drum Advanced".to_owned(),
        slug: "drum-advanced".to_owned(),
        short: "Adv".to_owned(),
        instrument: "drums".to_owned(),
        modes: Vec::new(),
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.drums.mixing.scenes".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules,
    }
}

/// The kit's effects: what it is sent to. The room sim in focus, the
/// verb banks at working width, the parallel compressors as tight rails
/// — once a compressor is dialled in it is a volume-balance game, and a
/// rail is a fader — and the kit itself present as rails.
// r[impl flow.drums.mixing.scenes]
fn drum_fx() -> Scene {
    Scene {
        name: "Drum FX".to_owned(),
        slug: "drum-fx".to_owned(),
        short: "FX".to_owned(),
        instrument: "drums".to_owned(),
        modes: Vec::new(),
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.drums.mixing.scenes".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules: vec![
            Rule::new(
                Selector {
                    role: Role::Bus,
                    ..Selector::default()
                },
                Effect::at(Size::Minimum),
            ),
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&KIT)
                },
                Effect::at(Size::Minimum),
            ),
            // The snare's three rooms are the one part of the kit this
            // scene is about: picking the room is a mute away.
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&[KIT[0], KIT[1], "Snare", "verb"])
                },
                Effect::at(Size::Working),
            ),
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&["process"])
                },
                Effect::at(Size::Minimum),
            ),
            Rule::new(
                Selector {
                    kind: Some("process".to_owned()),
                    ..Selector::default()
                },
                Effect::at(Size::Compact),
            ),
            Rule::new(
                Selector {
                    kind: Some("fx".to_owned()),
                    role: Role::Bus,
                    ..under(&["process"])
                },
                Effect::at(Size::Compact),
            ),
            Rule::new(
                Selector {
                    kind: Some("verb".to_owned()),
                    role: Role::Bus,
                    ..under(&["process"])
                },
                Effect::at(Size::Compact),
            ),
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&["process", "fx"])
                },
                Effect::at(Size::Working),
            ),
            // The room simulator is the subject: it is the one control
            // that decides what room the kit is in.
            Rule::new(
                Selector {
                    kind: Some("part".to_owned()),
                    role: Role::Leaf,
                    ..under(&["process", "fx"])
                },
                Effect::at(Size::Focus),
            ),
            hide_the_bus_tree(),
        ],
    }
}

/// The mix: the bus tree and nothing else — every bus at working width,
/// the folders that hold them compact, the instruments gone.
fn buses() -> Scene {
    Scene {
        name: "Buses".to_owned(),
        slug: "buses".to_owned(),
        short: "Bus".to_owned(),
        instrument: "bus".to_owned(),
        modes: Vec::new(),
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec!["flow.scenes.render".to_owned()],
        default: Effect::at(Size::Working),
        rules: vec![
            Rule::new(
                Selector {
                    role: Role::Bus,
                    ..Selector::default()
                },
                Effect::at(Size::Compact),
            ),
            Rule::new(
                Selector {
                    kind: Some("group".to_owned()),
                    ..Selector::default()
                },
                Effect::hidden(),
            ),
            Rule::new(
                Selector {
                    kind: Some("process".to_owned()),
                    ..Selector::default()
                },
                Effect::hidden(),
            ),
            Rule::new(under(&["fx"]), Effect::hidden()),
        ],
    }
}

/// The instrument bus: the electrics, acoustics, keys and synths at
/// working width with the instrument returns open beside them, the first
/// plate in focus, and the drums, their parallel chain and the vocals
/// out of the way.
fn guitar_fx() -> Scene {
    let mut rules = vec![Rule::new(
        Selector {
            role: Role::Bus,
            ..Selector::default()
        },
        Effect::at(Size::Compact),
    )];
    for group in [
        &["Guitars", "Electric"][..],
        &["Guitars", "Acoustic"],
        &["Keys"],
        &["Synths"],
        &["fx"],
    ] {
        rules.push(Rule::new(
            Selector {
                role: Role::Leaf,
                ..under(group)
            },
            Effect::at(Size::Working),
        ));
    }
    rules.extend([
        // A return a user picked, so a name: which plate is in front of
        // you is a choice of unit rather than a structure.
        Rule::new(
            Selector {
                name: Some("Fat Plate".to_owned()),
                ..under(&["fx"])
            },
            Effect::at(Size::Focus),
        ),
        Rule::new(under(&KIT), Effect::hidden()),
        Rule::new(
            Selector {
                kind: Some("process".to_owned()),
                ..Selector::default()
            },
            Effect::hidden(),
        ),
        Rule::new(under(&["Vocals"]), Effect::hidden()),
        hide_the_bus_tree(),
    ]);
    Scene {
        name: "Guitar FX".to_owned(),
        slug: "guitar-fx".to_owned(),
        short: "GFX".to_owned(),
        instrument: "guitar".to_owned(),
        modes: vec!["mix".to_owned()],
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec!["flow.scenes.render".to_owned()],
        default: Effect::at(Size::Minimum),
        rules,
    }
}

/// The vocal's taxonomy path.
const VOX: [&str; 2] = ["Vocals", "Lead"];

/// The lead vocal open, every return present as a short rail.
// r[impl flow.vocals.mixing.main]
fn lead_vocal() -> Scene {
    Scene {
        name: "Lead Vocal".to_owned(),
        slug: "lead-vocal".to_owned(),
        short: "Vox".to_owned(),
        instrument: "vocal".to_owned(),
        modes: vec!["mix".to_owned()],
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.vocals.mixing.main".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Compact),
        rules: vec![
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&VOX)
                },
                Effect::at(Size::Working),
            ),
            Rule::new(
                Selector {
                    role: Role::Leaf,
                    ..under(&[VOX[0], VOX[1], "fx"])
                },
                Effect::at(Size::Minimum),
            ),
            // The subject of the scene: its whole chain, top to bottom.
            Rule::new(
                Selector {
                    name: Some("Lead Vox".to_owned()),
                    ..under(&VOX)
                },
                Effect::at(Size::Focus),
            ),
        ],
    }
}

/// Editing the returns: one delay and one verb in focus, every other
/// return at the working width with its chain drawn, every folder a
/// rail. The returns are all live — a slap is a slap whether or not it
/// is the one being edited — so the scene shows them all working and
/// opens the two under the hands.
// r[impl flow.vocals.mixing.fx]
fn lead_vocal_fx() -> Scene {
    Scene {
        name: "Lead Vocal FX Edit".to_owned(),
        slug: "lead-vocal-fx".to_owned(),
        short: "VFX".to_owned(),
        instrument: "vocal".to_owned(),
        modes: vec!["edit".to_owned()],
        audience: Audience::Engineer,
        group_by: GroupBy::Arrangement,
        spec: vec![
            "flow.vocals.mixing.fx".to_owned(),
            "flow.scenes.render".to_owned(),
        ],
        default: Effect::at(Size::Working),
        rules: vec![
            Rule::new(
                Selector {
                    role: Role::Bus,
                    ..Selector::default()
                },
                Effect::at(Size::Minimum),
            ),
            Rule::new(
                Selector {
                    name: Some("Lead Vox".to_owned()),
                    ..under(&VOX)
                },
                Effect::at(Size::Focus),
            ),
            Rule::new(
                Selector {
                    name: Some("Short".to_owned()),
                    ..under(&[VOX[0], VOX[1], "fx", "Delay"])
                },
                Effect::at(Size::Focus),
            ),
            Rule::new(
                Selector {
                    name: Some("Long".to_owned()),
                    ..under(&[VOX[0], VOX[1], "fx", "Verb"])
                },
                Effect::at(Size::Focus),
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nine scenes, every slug distinct and every slug reachable.
    #[test]
    fn every_scene_is_reachable_by_its_slug() {
        assert_eq!(scenes().len(), 9);
        for s in scenes() {
            assert_eq!(
                scene(&s.slug).map(|f| f.slug.as_str()),
                Some(s.slug.as_str())
            );
        }
        assert!(scene("nonesuch").is_none());
    }

    /// The table, spelled out: which scene answers to which
    /// (mode, instrument, audience). Written out rather than derived,
    /// because this IS the follow-mode contract and a change to it
    /// should be a diff someone reads.
    // r[verify flow.scenes.follow-mode]
    // r[verify flow.scenes.two-audiences]
    #[test]
    fn the_table_claims_the_triples_it_is_supposed_to() {
        let claimed: Vec<(&str, &str, Vec<&str>, Audience)> = scenes()
            .iter()
            .map(|s| {
                (
                    s.slug.as_str(),
                    s.instrument.as_str(),
                    s.modes.iter().map(String::as_str).collect(),
                    s.audience,
                )
            })
            .collect();
        assert_eq!(
            claimed,
            vec![
                ("drum-tracking", "drums", vec!["record"], Audience::Engineer),
                ("drum-mixing", "drums", vec!["mix"], Audience::Engineer),
                ("drum-overview", "drums", vec!["mix"], Audience::Player),
                ("drum-advanced", "drums", vec![], Audience::Engineer),
                ("drum-fx", "drums", vec![], Audience::Engineer),
                ("buses", "bus", vec![], Audience::Engineer),
                ("guitar-fx", "guitar", vec!["mix"], Audience::Engineer),
                ("lead-vocal", "vocal", vec!["mix"], Audience::Engineer),
                ("lead-vocal-fx", "vocal", vec!["edit"], Audience::Engineer),
            ]
        );
    }

    /// A scene with no mode is recall-only: the number keys reach it,
    /// no mode opens it.
    // r[verify flow.scenes.follow-mode]
    #[test]
    fn a_scene_with_no_mode_is_recall_only() {
        let recall_only: Vec<&str> = scenes()
            .iter()
            .filter(|s| s.modes.is_empty())
            .map(|s| s.slug.as_str())
            .collect();
        assert_eq!(recall_only, ["drum-advanced", "drum-fx", "buses"]);
    }

    /// A session with the two folders the prelude is about: the Guide
    /// at the top, the Keyflow folder under it, and a kit beside them.
    fn with_the_guide() -> Vec<crate::scenes::Fact> {
        use crate::golden_session::Kind;
        use crate::scenes::{Fact, Segment};
        let guide = Segment::named("Guide").of(Kind::Guide);
        let keyflow = Segment::named("Keyflow").of(Kind::Keyflow);
        vec![
            Fact::folder("guide", "Guide", 0, 0)
                .of(Kind::Guide)
                .at(vec![guide.clone()]),
            Fact::leaf("click", "Click", 1, 1).at(vec![guide.clone()]),
            Fact::leaf("shaker", "Shaker", 2, 1).at(vec![guide]),
            Fact::folder("keyflow", "Keyflow", 3, 0)
                .of(Kind::Keyflow)
                .at(vec![keyflow.clone()]),
            Fact::leaf("chords", "CHORDS", 4, 1).at(vec![keyflow.clone()]),
            Fact::leaf("lines", "LINES", 5, 1).at(vec![keyflow]),
        ]
    }

    /// Every scene shows the Guide folder collapsed to one row, and no
    /// flow in the table opens it — opening it is a flow of its own.
    // r[verify flow.scenes.guide-folder]
    #[test]
    fn every_scene_collapses_the_guide_folder() {
        let facts = with_the_guide();
        for scene in scenes() {
            let rows =
                crate::scenes::resolve(scene, &facts, crate::scenes::Surface::Mixer, Some("mix"), None);
            let guide: Vec<&str> = rows
                .iter()
                .filter_map(crate::scenes::Row::guid)
                .filter(|g| matches!(*g, "guide" | "click" | "shaker"))
                .collect();
            assert_eq!(
                guide,
                ["guide"],
                "{} shows the click and the shaker",
                scene.slug
            );
        }
    }

    /// Every scene shows the Keyflow folder collapsed too — except in
    /// Write and Produce, whose subject is the song's knowledge.
    // r[verify flow.scenes.keyflow-folder]
    #[test]
    fn write_and_produce_are_the_only_modes_that_open_keyflow() {
        let facts = with_the_guide();
        let items = |mode: &str| {
            crate::scenes::resolve(
                scenes().first().expect("a scene"),
                &facts,
                crate::scenes::Surface::Mixer,
                Some(mode),
                None,
            )
            .iter()
            .filter_map(crate::scenes::Row::guid)
            .filter(|g| matches!(*g, "keyflow" | "chords" | "lines"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
        };
        assert_eq!(items("mix"), ["keyflow"]);
        assert_eq!(items("record"), ["keyflow"]);
        assert_eq!(items("write"), ["keyflow", "chords", "lines"]);
        assert_eq!(items("produce"), ["keyflow", "chords", "lines"]);
    }

    /// The prelude is the same two rules in every mode, plus the one
    /// that opens the Keyflow folder where the song's knowledge is the
    /// subject.
    // r[verify flow.scenes.keyflow-folder]
    #[test]
    fn write_and_produce_open_the_keyflow_folder() {
        assert_eq!(prelude(None, None).len(), 2);
        assert_eq!(prelude(Some("mix"), None).len(), 2);
        for mode in ["write", "produce"] {
            let rules = prelude(Some(mode), None);
            assert_eq!(rules.len(), 3, "{mode}");
            assert_eq!(
                rules.last().map(|r| r.effect.fold),
                Some(Some(Fold::Open)),
                "{mode} opens it"
            );
        }
    }
}
