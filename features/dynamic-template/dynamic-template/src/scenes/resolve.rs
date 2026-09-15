//! The one function: a scene and a session in, a row list out.
//!
//! Everything a scene means is decided here and nowhere else. The window
//! renders the row list as it is; the REAPER applier walks the same list
//! and writes show, fold and size onto real tracks. Two appliers, one
//! resolve — which is the whole reason the row list is the output rather
//! than a pile of per-surface booleans.

use super::facts::Fact;
use super::language::Language;
use super::selector::{Rank, Role, Selector};
use super::types::{Fold, Resolved, Scene, Size, Surface};

/// What a row is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A real track.
    Track(String),
    /// A synthesised header for one performer — `flow.scenes.performer-order`.
    ///
    /// Emitted by `group_by: performer`, which is #51. The variant is
    /// here so that ticket adds the emitting and nothing else, and so
    /// that every consumer written today already skips a row it cannot
    /// apply to a track.
    PerformerHeader(String),
}

/// One row of a resolved scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What the row is about.
    pub target: Target,
    /// How deep it sits, 0 at the top.
    pub depth: u32,
    /// How much room it gets.
    pub size: Size,
    /// What happens to the rows beneath it.
    pub fold: Fold,
}

impl Row {
    /// The track a row is about, when it is about one.
    #[must_use]
    pub const fn guid(&self) -> Option<&str> {
        match &self.target {
            Target::Track(guid) => Some(guid.as_str()),
            Target::PerformerHeader(_) => None,
        }
    }
}

/// Resolve a scene against a session, for one surface.
///
/// Rules are evaluated in order and the **last** one that matches a
/// track wins; the scene's `default` covers every track no rule matched.
/// A folder folded `Collapsed` or `Hidden` takes its rows with it, and a
/// `Hidden` folder takes its own row too.
///
/// Facts must arrive in project order — the row list is in that order,
/// and the fold walk depends on it.
///
/// # Engine invariants
///
/// Two things are true of every scene and cannot be said otherwise in
/// the table, so they are enforced here:
///
/// - **A stereo pair's half is always a rail.** The pair's folder
///   carries the processing and the width; its L and its R are one
///   signal seen twice, and a scene that opened them would be showing
///   the same thing at four times the cost.
/// - **A fold applies down the tree.** Rules say what happens to a
///   folder; what happens to its contents follows from that rather than
///   from a second rule nobody would remember to write.
///
/// `active_language` (`flow.vocals.language.active`) feeds the common
/// prelude the same way `mode` does: `None` hides nothing, `Some(lang)`
/// hides every source whose own language is neither `lang` nor
/// [`Language::All`] — see `table::prelude`.
// r[impl flow.vocals.language.active]
#[must_use]
pub fn resolve(
    scene: &Scene,
    facts: &[Fact],
    surface: Surface,
    mode: Option<&str>,
    active_language: Option<Language>,
) -> Vec<Row> {
    let effects = effects_for(scene, facts, surface, mode, active_language);
    let mut out: Vec<Row> = Vec::with_capacity(facts.len());
    // The depth of the shallowest folded folder we are inside, if any.
    let mut folded: Option<u32> = None;
    for (fact, effect) in facts.iter().zip(&effects) {
        if let Some(at) = folded {
            if fact.depth > at {
                continue;
            }
            folded = None;
        }
        if effect.fold == Fold::Hidden {
            // A hidden leaf drops its own row; a hidden folder drops
            // everything under it as well.
            if fact.is_folder {
                folded = Some(fact.depth);
            }
            continue;
        }
        if fact.is_folder && !effect.fold.keeps_children() {
            folded = Some(fact.depth);
        }
        let size = if fact.pair_half && !fact.is_folder {
            Size::Minimum
        } else {
            effect.size
        };
        out.push(Row {
            target: Target::Track(fact.guid.clone()),
            depth: fact.depth,
            size,
            fold: effect.fold,
        });
    }
    out
}

/// The effect each track ends up with, before the fold walk drops rows.
///
/// The common prelude goes first, so a scene that wants the Guide folder
/// open only has to say so — it is data, and it is overridable.
fn effects_for(
    scene: &Scene,
    facts: &[Fact],
    surface: Surface,
    mode: Option<&str>,
    active_language: Option<Language>,
) -> Vec<Resolved> {
    let mut effects = vec![Resolved::from(scene.default); facts.len()];
    let prelude = super::table::prelude(mode, active_language);
    for rule in prelude.iter().chain(&scene.rules) {
        let effect = rule.for_surface(surface);
        for i in matching(&rule.selector, facts) {
            if let Some(slot) = effects.get_mut(i) {
                *slot = effect.over(*slot);
            }
        }
    }
    effects
}

/// The indices of every fact a selector matches.
fn matching(selector: &Selector, facts: &[Fact]) -> Vec<usize> {
    let base: Vec<usize> = facts
        .iter()
        .enumerate()
        .filter(|(_, fact)| matches_but_for_rank(selector, fact))
        .map(|(i, _)| i)
        .collect();
    match selector.rank {
        Rank::All => base,
        Rank::TopmostPerInstrument => topmost_per_instrument(&base, facts),
    }
}

fn matches_but_for_rank(selector: &Selector, fact: &Fact) -> bool {
    let role_ok = match selector.role {
        Role::Any => true,
        Role::Bus => fact.is_folder,
        Role::Leaf => !fact.is_folder,
    };
    if !role_ok {
        return false;
    }
    if !path_starts_with(&fact.path, &selector.group) {
        return false;
    }
    if let Some(kind) = &selector.kind {
        if !fact
            .kind
            .is_some_and(|k| k.as_str().eq_ignore_ascii_case(kind))
        {
            return false;
        }
    }
    if let Some(language) = selector.language {
        if fact.language != Some(language) {
            return false;
        }
    }
    let dimensions = [
        (&selector.performer, fact.performer.as_deref()),
        (&selector.layer, fact.layer.as_deref()),
        (&selector.channel, fact.channel.as_deref()),
        (&selector.multi_mic, fact.multi_mic.as_deref()),
        (&selector.arrangement, fact.arrangement.as_deref()),
        (&selector.name, Some(fact.name.as_str())),
    ];
    dimensions.into_iter().all(|(want, have)| {
        want.as_ref()
            .is_none_or(|want| have.is_some_and(|have| have.eq_ignore_ascii_case(want)))
    })
}

/// Whether a taxonomy path begins with a selector's group path.
fn path_starts_with(path: &[super::facts::Segment], group: &[String]) -> bool {
    if group.len() > path.len() {
        return false;
    }
    group
        .iter()
        .zip(path)
        .all(|(want, segment)| segment.answers_to(want))
}

/// Keep only the lowest-index track per taxonomy group.
fn topmost_per_instrument(base: &[usize], facts: &[Fact]) -> Vec<usize> {
    let mut best: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for &i in base {
        let Some(fact) = facts.get(i) else { continue };
        let key = fact.group_key();
        match best.get(&key) {
            Some(&j) if facts.get(j).is_some_and(|f| f.index <= fact.index) => {}
            _ => {
                best.insert(key, i);
            }
        }
    }
    let mut out: Vec<usize> = best.into_values().collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden_session::Kind;
    use crate::scenes::facts::Segment;
    use crate::scenes::types::{Audience, Effect, GroupBy, Rule};

    fn scene(default: Effect, rules: Vec<Rule>) -> Scene {
        Scene {
            name: "Test".to_owned(),
            slug: "test".to_owned(),
            short: "Test".to_owned(),
            instrument: "drums".to_owned(),
            modes: Vec::new(),
            audience: Audience::Engineer,
            group_by: GroupBy::Arrangement,
            spec: Vec::new(),
            default,
            rules,
        }
    }

    fn kit() -> Vec<Fact> {
        let drums = Segment::named("Drum Kit")
            .of(Kind::Group)
            .standing_for("Drum Kit");
        let kick = Segment::named("Kick").of(Kind::Piece).standing_for("Kick");
        vec![
            Fact::folder("kit", "Drum Kit", 0, 0)
                .of(Kind::Group)
                .at(vec![drums.clone()]),
            Fact::folder("kick", "Kick", 1, 1)
                .of(Kind::Piece)
                .at(vec![drums.clone(), kick.clone()]),
            Fact::leaf("in", "In", 2, 2)
                .of(Kind::Source)
                .at(vec![drums.clone(), kick.clone()]),
            Fact::leaf("out", "Out", 3, 2)
                .of(Kind::Source)
                .at(vec![drums, kick]),
        ]
    }

    /// The rule that comes last is the one that happened — which is what
    /// lets a scene say "every leaf is a rail, except these".
    #[test]
    fn the_last_matching_rule_wins() {
        let scene = scene(
            Effect::at(Size::Compact),
            vec![
                Rule::new(
                    Selector {
                        role: Role::Leaf,
                        ..Selector::default()
                    },
                    Effect::at(Size::Minimum),
                ),
                Rule::new(
                    Selector {
                        kind: Some("source".to_owned()),
                        ..Selector::default()
                    },
                    Effect::at(Size::Working),
                ),
            ],
        );
        let rows = resolve(&scene, &kit(), Surface::Mixer, None, None);
        let size = |guid: &str| {
            rows.iter()
                .find(|r| r.guid() == Some(guid))
                .expect("the row")
                .size
        };
        assert_eq!(size("kit"), Size::Compact, "the default covers the folder");
        assert_eq!(size("in"), Size::Working, "the later rule won");
    }

    /// A collapsed folder keeps its own row and drops what it holds; a
    /// hidden one drops both.
    #[test]
    fn a_fold_reaches_the_rows_beneath_it() {
        let collapsed = scene(
            Effect::at(Size::Compact),
            vec![Rule::new(
                Selector {
                    kind: Some("piece".to_owned()),
                    ..Selector::default()
                },
                Effect::at(Size::Working).folded(Fold::Collapsed),
            )],
        );
        let rows = resolve(&collapsed, &kit(), Surface::Mixer, None, None);
        let guids: Vec<&str> = rows.iter().filter_map(Row::guid).collect();
        assert_eq!(guids, ["kit", "kick"], "the mics went with the fold");

        let hidden = scene(
            Effect::at(Size::Compact),
            vec![Rule::new(
                Selector {
                    kind: Some("piece".to_owned()),
                    ..Selector::default()
                },
                Effect::hidden(),
            )],
        );
        let rows = resolve(&hidden, &kit(), Surface::Mixer, None, None);
        let guids: Vec<&str> = rows.iter().filter_map(Row::guid).collect();
        assert_eq!(guids, ["kit"], "the piece went too");
    }

    /// The pair-half invariant: not a rule a scene can state, and not a
    /// rule a scene can override.
    #[test]
    fn a_stereo_pairs_half_is_always_a_rail() {
        let mut facts = kit();
        if let Some(fact) = facts.get_mut(2) {
            fact.pair_half = true;
        }
        let wide = scene(Effect::at(Size::Focus), Vec::new());
        let rows = resolve(&wide, &facts, Surface::Mixer, None, None);
        let size = |guid: &str| {
            rows.iter()
                .find(|r| r.guid() == Some(guid))
                .expect("the row")
                .size
        };
        assert_eq!(size("in"), Size::Minimum, "a half is a rail");
        assert_eq!(size("out"), Size::Focus, "and nothing else changed");
    }

    /// A selector's group path is a prefix, and each of its segments
    /// matches the kind, the template group or the name — which is what
    /// a renamed Process folder relies on.
    #[test]
    fn a_group_prefix_matches_a_renamed_folder() {
        let parallel = Segment::named("Parallel").of(Kind::Process);
        let facts = vec![
            Fact::folder("proc", "Parallel", 0, 0)
                .of(Kind::Process)
                .at(vec![parallel.clone()]),
            Fact::leaf("dry", "Dry", 1, 1)
                .of(Kind::Part)
                .at(vec![parallel]),
        ];
        let s = scene(
            Effect::at(Size::Compact),
            vec![Rule::new(
                Selector {
                    group: vec!["process".to_owned()],
                    ..Selector::default()
                },
                Effect::at(Size::Minimum),
            )],
        );
        let rows = resolve(&s, &facts, Surface::Mixer, None, None);
        assert!(rows.iter().all(|r| r.size == Size::Minimum), "{rows:?}");
    }

    /// A per-surface override is what lets one scene rail a track in the
    /// mixer and still give its row height in the arrangement.
    #[test]
    fn the_two_surfaces_can_disagree() {
        let s = scene(
            Effect::at(Size::Compact),
            vec![Rule {
                selector: Selector {
                    role: Role::Leaf,
                    ..Selector::default()
                },
                effect: Effect::at(Size::Minimum),
                arrange: Some(Effect::at(Size::Working)),
                mixer: None,
            }],
        );
        let mixer = resolve(&s, &kit(), Surface::Mixer, None, None);
        let arrange = resolve(&s, &kit(), Surface::Arrange, None, None);
        assert_eq!(mixer[2].size, Size::Minimum);
        assert_eq!(arrange[2].size, Size::Working);
    }

    /// One mic per piece, which is what an edit view of a kit is.
    #[test]
    fn topmost_per_instrument_keeps_one_of_each() {
        let s = scene(
            Effect::at(Size::Minimum),
            vec![Rule::new(
                Selector {
                    role: Role::Leaf,
                    rank: Rank::TopmostPerInstrument,
                    ..Selector::default()
                },
                Effect::at(Size::Working),
            )],
        );
        let rows = resolve(&s, &kit(), Surface::Mixer, None, None);
        let opened: Vec<&str> = rows
            .iter()
            .filter(|r| r.size == Size::Working)
            .filter_map(Row::guid)
            .collect();
        assert_eq!(opened, ["in"], "one mic of the piece, not both");
    }

    /// `Vocals / Ron / Main / {EN, ES, PT}` and `Ron / DBL / {EN, ES,
    /// PT}` — a lead's two layers, each with a source per language.
    fn ron() -> Vec<Fact> {
        use super::super::language::Language;

        let vocals = Segment::named("Vocals").of(Kind::Group);
        let ron = Segment::named("Ron");
        let main = Segment::named("Main");
        let dbl = Segment::named("DBL");
        vec![
            Fact::folder("vocals", "Vocals", 0, 0)
                .of(Kind::Group)
                .at(vec![vocals.clone()]),
            Fact::folder("ron", "Ron", 1, 1).at(vec![vocals.clone(), ron.clone()]),
            Fact::folder("main", "Main", 2, 2).at(vec![vocals.clone(), ron.clone(), main.clone()]),
            Fact::leaf("main-en", "EN", 3, 3)
                .of(Kind::Source)
                .at(vec![vocals.clone(), ron.clone(), main.clone()])
                .speaking(Language::En),
            Fact::leaf("main-es", "ES", 4, 3)
                .of(Kind::Source)
                .at(vec![vocals.clone(), ron.clone(), main.clone()])
                .speaking(Language::Es),
            Fact::leaf("main-pt", "PT", 5, 3)
                .of(Kind::Source)
                .at(vec![vocals.clone(), ron.clone(), main])
                .speaking(Language::Pt),
            Fact::folder("dbl", "DBL", 6, 2).at(vec![vocals.clone(), ron.clone(), dbl.clone()]),
            Fact::leaf("dbl-en", "EN", 7, 3)
                .of(Kind::Source)
                .at(vec![vocals.clone(), ron.clone(), dbl.clone()])
                .speaking(Language::En),
            Fact::leaf("dbl-es", "ES", 8, 3)
                .of(Kind::Source)
                .at(vec![vocals.clone(), ron.clone(), dbl.clone()])
                .speaking(Language::Es),
            Fact::leaf("dbl-pt", "PT", 9, 3)
                .of(Kind::Source)
                .at(vec![vocals, ron, dbl])
                .speaking(Language::Pt),
        ]
    }

    /// The active language hides every source whose language is neither
    /// active nor `All`; the mix tracks above them — `Vocals`, `Ron`,
    /// `Main`, `DBL` — carry no language and stay in view whatever the
    /// language.
    // r[verify flow.vocals.language]
    // r[verify flow.vocals.language.active]
    #[test]
    fn the_active_language_hides_the_others_and_keeps_the_mix_tracks() {
        use super::super::language::Language;

        let s = scene(Effect::at(Size::Compact), Vec::new());
        let facts = ron();
        let rows = resolve(&s, &facts, Surface::Mixer, None, Some(Language::En));
        let shown: Vec<&str> = rows.iter().filter_map(Row::guid).collect();
        assert_eq!(
            shown,
            ["vocals", "ron", "main", "main-en", "dbl", "dbl-en"],
            "only English sources survive, the mix tracks stay"
        );
    }

    /// Switching the language switches which sources it hides, in one
    /// step — no VCA, no separate switch, just a different value in.
    #[test]
    fn switching_the_language_switches_which_sources_show() {
        use super::super::language::Language;

        let s = scene(Effect::at(Size::Compact), Vec::new());
        let facts = ron();
        let rows = resolve(&s, &facts, Surface::Mixer, None, Some(Language::Es));
        let shown: Vec<&str> = rows.iter().filter_map(Row::guid).collect();
        assert_eq!(shown, ["vocals", "ron", "main", "main-es", "dbl", "dbl-es"]);
    }

    /// With no active language set, nothing is hidden — an untouched
    /// project shows every source rather than guessing which one.
    #[test]
    fn with_no_active_language_nothing_is_hidden() {
        let s = scene(Effect::at(Size::Compact), Vec::new());
        let facts = ron();
        let rows = resolve(&s, &facts, Surface::Mixer, None, None);
        assert_eq!(rows.len(), facts.len(), "nothing hidden");
    }

    /// A language-free source (`All`) is never hidden, whatever the
    /// active language is.
    #[test]
    fn all_is_never_hidden() {
        use super::super::language::Language;

        let vocals = Segment::named("Vocals").of(Kind::Group);
        let hey = Segment::named("Hey");
        let facts = vec![
            Fact::folder("hey", "Hey", 0, 0).at(vec![vocals.clone(), hey.clone()]),
            Fact::leaf("hey-all", "All", 1, 1)
                .of(Kind::Source)
                .at(vec![vocals, hey])
                .speaking(Language::All),
        ];
        let s = scene(Effect::at(Size::Compact), Vec::new());
        let rows = resolve(&s, &facts, Surface::Mixer, None, Some(Language::En));
        let shown: Vec<&str> = rows.iter().filter_map(Row::guid).collect();
        assert_eq!(shown, ["hey", "hey-all"]);
    }
}
