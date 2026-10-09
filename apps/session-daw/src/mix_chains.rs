//! The mix's processing, from the FTS processors built in: what a
//! prepared song is mixed with in this window, no plugin loaded.
//!
//! - Every recorded track gets a **Tone** chain — FTS EQ, Comp, Saturate,
//!   the three the strip's Tone rack draws — bypassed: a channel ready to
//!   mix, costing nothing until it is switched on.
//! - Every bus gets FTS EQ and Comp, bypassed.
//! - The drum Process folder's parallel compressors get FTS Comp set
//!   from gentle to crushed, its rooms FTS Reverb set from a small room
//!   to an arena: on, since a muted track's chain is not run
//!   (`daw_standalone` render), and they are muted until picked.
//! - A drum piece keyed from its trigger (`key_drum_gates`, a send onto
//!   its channels 3/4) gets FTS Gate, on and keyed from that sidechain,
//!   ahead of its Tone chain: the threshold read off the trigger's own
//!   hits ([`gate_threshold`]).
//! - The golden mix's returns — Kick's room, Snare's non-linear, the
//!   vocals' long and short verbs, slap and octave down — their FX set,
//!   on ([`RETURNS`]).
//! - The instrument folders and the folders directly in them get the
//!   bus chain, bypassed, as the golden mix's do.
//!
//! A track that already has FX keeps them and gets nothing added.

#![cfg(feature = "native")]

use daw::service::{
    Effects, FxChainContext, FxRef, FxTarget, Items as _, ProjectContext, SetParameterRequest,
    Tracks,
};

/// The Tone chain, in order.
const TONE: &[&str] = &["FTS EQ", "FTS Comp", "FTS Saturate"];

/// A bus's chain.
const BUS: &[&str] = &["FTS EQ", "FTS Comp"];

/// The returns the golden mix has, by the folder they sit in and their
/// name: the FX and its settings, all wet — they are fed by sends.
const RETURNS: &[(&str, &str, &str, &[(&str, &str)])] = &[
    (
        "kick",
        "verb",
        "FTS Reverb",
        &[
            ("Space", "Room"),
            ("Size", "25%"),
            ("Decay", "25%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "snare",
        "nonlin",
        "FTS Reverb",
        &[("Space", "Non-Linear"), ("Decay", "40%"), ("Mix", "100%")],
    ),
    (
        "fx",
        "long verb",
        "FTS Reverb",
        &[
            ("Space", "Concert Hall"),
            ("Decay", "70%"),
            ("Pre-Delay", "40.0 ms"),
            ("Mix", "100%"),
        ],
    ),
    (
        "fx",
        "short verb",
        "FTS Reverb",
        &[("Space", "Plate"), ("Decay", "30%"), ("Mix", "100%")],
    ),
    (
        "fx",
        "slap",
        "FTS Delay",
        &[
            ("Time L", "110.0 ms"),
            ("Link", "On"),
            ("Feedback", "10%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "fx",
        "oct down",
        "FTS Pitch",
        &[("Semitones", "-12"), ("Mix", "100%")],
    ),
];

/// The gate on a keyed drum piece, before its threshold: Kick and Toms
/// shut all the way, Snare to 20 dB under — its room stays in the
/// gaps.
fn gate_settings(piece: &str) -> &'static [(&'static str, &'static str)] {
    if piece.eq_ignore_ascii_case("snare") {
        &[
            ("Key", "Sidechain"),
            ("Range", "20.0 dB"),
            ("Release", "150.0 ms"),
        ]
    } else {
        &[("Key", "Sidechain")]
    }
}

/// Each parallel compressor's settings, in the plugin's own words.
const COMPRESS: &[Preset] = &[
    (
        "Dry",
        &[
            ("Style", "Clean"),
            ("Ratio", "2.0:1"),
            ("Threshold", "-10.0 dB"),
            ("Attack", "30.0 ms"),
            ("Release", "150 ms"),
        ],
    ),
    (
        "Tight",
        &[
            ("Style", "VCA"),
            ("Ratio", "4.0:1"),
            ("Threshold", "-25.0 dB"),
            ("Attack", "1.0 ms"),
            ("Release", "50 ms"),
        ],
    ),
    (
        "Punch",
        &[
            ("Style", "FET"),
            ("Ratio", "4.0:1"),
            ("Threshold", "-25.0 dB"),
            ("Attack", "10.0 ms"),
            ("Release", "80 ms"),
        ],
    ),
    (
        "Smash",
        &[
            ("Style", "FET"),
            ("Ratio", "20.0:1"),
            ("Threshold", "-40.0 dB"),
            ("Attack", "0.5 ms"),
            ("Release", "100 ms"),
            ("Drive", "30%"),
        ],
    ),
    (
        "Crunch",
        &[
            ("Style", "FET"),
            ("Ratio", "20.0:1"),
            ("Threshold", "-50.0 dB"),
            ("Attack", "0.1 ms"),
            ("Release", "60 ms"),
            ("Drive", "60%"),
            ("Character", "Clip"),
        ],
    ),
];

/// Each room's settings: fed by a send, so all wet.
const ROOMS: &[Preset] = &[
    (
        "Room Sim",
        &[
            ("Space", "Room"),
            ("Size", "20%"),
            ("Decay", "20%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "Wood Room",
        &[
            ("Space", "Chamber"),
            ("Size", "35%"),
            ("Decay", "35%"),
            ("Damping", "50%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "Music Club",
        &[
            ("Space", "Studio"),
            ("Size", "50%"),
            ("Decay", "45%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "Stadium",
        &[
            ("Space", "Arena"),
            ("Size", "95%"),
            ("Decay", "85%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "RMX 16",
        &[
            ("Space", "Digital Plate"),
            ("Decay", "50%"),
            ("Mix", "100%"),
        ],
    ),
    (
        "Nonlin",
        &[("Space", "Non-Linear"), ("Decay", "40%"), ("Mix", "100%")],
    ),
    (
        "Brick Wall",
        &[("Space", "Plate"), ("Decay", "60%"), ("Mix", "100%")],
    ),
];

/// A preset table's settings for the track `name`.
fn settings_of(
    table: &'static [Preset],
    name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    table.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// A track's name and its FX's settings, in the plugin's own words.
type Preset = (&'static str, &'static [(&'static str, &'static str)]);

/// What [`add`] put on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chains {
    pub tone: usize,
    pub buses: usize,
    pub process: usize,
    /// Drum pieces gated from their trigger: name and threshold, dBFS.
    pub gates: Vec<(String, f64)>,
    /// Returns given their FX.
    pub returns: usize,
    /// Settings a plugin did not know (`fx param=text`), said, not fatal.
    pub unknown: Vec<String>,
}

/// Put the mix's chains on `project` (see the module docs).
///
/// # Errors
///
/// A backend call failed.
pub fn add(
    daw: &daw_standalone::sync::Standalone,
    project: &ProjectContext,
) -> eyre::Result<Chains> {
    let tracks = Tracks::all(daw, project.clone());
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let mut done = Chains::default();
    for track in &tracks {
        let chain = FxChainContext::Track(track.guid.clone());
        if Effects::count(daw, project.clone(), chain.clone()) > 0 {
            continue;
        }
        let path = crate::gain_stage::path_of(track, &by_guid);
        let lower: Vec<String> = path.iter().map(|p| p.trim().to_lowercase()).collect();
        let under = |what: &str| lower.iter().rev().skip(1).any(|p| p == what);
        let name = track.name.trim();
        // A drum piece keyed from its trigger: the gate, then its tone.
        let keys: Vec<String> = daw_proto::Routing::receives(
            daw,
            project.clone(),
            daw::service::TrackRef::Guid(track.guid.clone()),
        )
        .into_iter()
        .filter(|r| r.dest_channels.start_channel >= 2)
        // A receive's far end — the track feeding it — is its
        // `dest_track_guid`; its `source_track_guid` is this track.
        .filter_map(|r| r.dest_track_guid)
        .collect();
        if !keys.is_empty() {
            let threshold = gate_threshold(daw, project, &tracks, &keys);
            let gate = format!("{:.1} dB", threshold.unwrap_or(-40.0));
            let mut settings: Vec<(&str, &str)> = gate_settings(name).to_vec();
            settings.push(("Threshold", gate.as_str()));
            set_up(
                daw,
                project,
                &chain,
                "FTS Gate",
                &settings,
                &mut done.unknown,
            )?;
            bypassed(daw, project, &chain, TONE)?;
            done.gates
                .push((name.to_owned(), threshold.unwrap_or(-40.0)));
            continue;
        }
        let parent = lower.iter().rev().nth(1).map_or("", String::as_str);
        if let Some((_, _, fx, settings)) = RETURNS
            .iter()
            .find(|(p, n, _, _)| *p == parent && name.eq_ignore_ascii_case(n))
        {
            set_up(daw, project, &chain, fx, settings, &mut done.unknown)?;
            done.returns += 1;
            continue;
        }
        if under(&dynamic_template::apply::PROCESS_FOLDER.to_lowercase()) {
            let (fx, settings) = match (settings_of(COMPRESS, name), settings_of(ROOMS, name)) {
                (Some(s), _) => ("FTS Comp", s),
                (_, Some(s)) => ("FTS Reverb", s),
                _ => continue,
            };
            let Some(guid) = Effects::add(daw, project.clone(), chain.clone(), fx) else {
                continue;
            };
            let target = FxTarget {
                context: chain.clone(),
                fx: FxRef::Guid(guid),
            };
            for (param, text) in settings {
                match builtin_fx::setting(fx, param, text) {
                    Some((index, value)) => Effects::set_parameter(
                        daw,
                        project.clone(),
                        SetParameterRequest {
                            target: target.clone(),
                            index,
                            value,
                        },
                    )?,
                    None => done.unknown.push(format!("{fx} {param}={text}")),
                }
            }
            done.process += 1;
            continue;
        }
        let referenced = lower.iter().any(|p| {
            p == &dynamic_template::apply::REFERENCES_FOLDER.to_lowercase()
                || p == "guide"
                || p == "mix bus"
        });
        let bus = dynamic_template::apply::is_bus_track(name);
        let recorded = !track.is_folder
            && !daw
                .get_items(
                    project.clone(),
                    daw::service::TrackRef::Guid(track.guid.clone()),
                )
                .is_empty();
        // An instrument folder, or a folder directly in one: Drums, Bass,
        // Guitars' Clean and Rhythm, the vocals' Main — not the kit's own
        // wiring, the returns' folder or a trigger.
        let group = track.is_folder
            && !referenced
            && path.len() <= 2
            && !dynamic_template::apply::is_trigger(name)
            && !["drum kit", "process", "fx", "sum", "rehearsal"]
                .iter()
                .any(|w| name.eq_ignore_ascii_case(w));
        let names = if bus || group {
            BUS
        } else if recorded && !referenced && !dynamic_template::apply::is_trigger(name) {
            // A trigger is a key, not a channel: it has no tone.
            TONE
        } else {
            continue;
        };
        bypassed(daw, project, &chain, names)?;
        if bus || group {
            done.buses += 1;
        } else {
            done.tone += 1;
        }
    }
    Ok(done)
}

/// `names` on `chain`, each added bypassed: ready, costing nothing.
fn bypassed(
    daw: &daw_standalone::sync::Standalone,
    project: &ProjectContext,
    chain: &FxChainContext,
    names: &[&str],
) -> eyre::Result<()> {
    for fx in names {
        let Some(guid) = Effects::add(daw, project.clone(), chain.clone(), fx) else {
            continue;
        };
        Effects::set_enabled(
            daw,
            project.clone(),
            FxTarget {
                context: chain.clone(),
                fx: FxRef::Guid(guid),
            },
            false,
        )?;
    }
    Ok(())
}

/// `fx` added to `chain`, on, with `settings` in the plugin's own words;
/// a setting it does not know goes to `unknown`.
fn set_up(
    daw: &daw_standalone::sync::Standalone,
    project: &ProjectContext,
    chain: &FxChainContext,
    fx: &str,
    settings: &[(&str, &str)],
    unknown: &mut Vec<String>,
) -> eyre::Result<()> {
    let Some(guid) = Effects::add(daw, project.clone(), chain.clone(), fx) else {
        return Ok(());
    };
    let target = FxTarget {
        context: chain.clone(),
        fx: FxRef::Guid(guid),
    };
    for (param, text) in settings {
        match builtin_fx::setting(fx, param, text) {
            Some((index, value)) => Effects::set_parameter(
                daw,
                project.clone(),
                SetParameterRequest {
                    target: target.clone(),
                    index,
                    value,
                },
            )?,
            None => unknown.push(format!("{fx} {param}={text}")),
        }
    }
    Ok(())
}

/// How far under a trigger's strong hits its gate opens: far enough for
/// the soft ones — a ghost note, a flam's grace — to open it too.
const GATE_UNDER_HITS_DB: f64 = 24.0;

/// How far over the trigger's floor the gate stays shut.
const GATE_OVER_FLOOR_DB: f64 = 6.0;

/// The threshold for a gate keyed from `keys` (trigger tracks, or a
/// folder of them), read off the trigger's own recording — offline, the
/// whole song at once: [`GATE_UNDER_HITS_DB`] under the strong hits of
/// the quietest trigger played (a folder of tom triggers opens for every
/// tom; one that far under the loudest is a tom not played), and at
/// least [`GATE_OVER_FLOOR_DB`] over the noise between them.
/// `None` when no trigger has audio.
fn gate_threshold(
    daw: &daw_standalone::sync::Standalone,
    project: &ProjectContext,
    tracks: &[daw_proto::Track],
    keys: &[String],
) -> Option<f64> {
    use daw::service::{ItemRef, Peaks as _, TakeRef};
    let sources = tracks.iter().filter(|t| {
        !t.is_folder
            && (keys.contains(&t.guid) || t.parent_guid.as_ref().is_some_and(|p| keys.contains(p)))
    });
    let mut levels: Vec<(f64, f64)> = Vec::new();
    for track in sources {
        let mut points: Vec<f64> = Vec::new();
        let mut step = 0.0;
        for item in daw.get_items(
            project.clone(),
            daw::service::TrackRef::Guid(track.guid.clone()),
        ) {
            if !daw_ui::studio::project::on_playing_lane(track, &item) {
                continue;
            }
            let peaks = daw.take_peaks(
                project.clone(),
                ItemRef::Guid(item.guid.clone()),
                TakeRef::Active,
                crate::midi::WAVE_BLOCK,
            );
            if let Some(wave) = crate::midi::Wave::from_peaks(&peaks) {
                step = wave.step;
                points.extend(
                    wave.points
                        .iter()
                        .map(|(high, low)| f64::from(high.abs().max(low.abs())) * item.volume),
                );
            }
        }
        let Some(hits) = crate::gain_stage::level(&points, step) else {
            continue;
        };
        levels.push((hits, floor_db(&points)));
    }
    // A trigger far under the loudest of its set is a drum not played —
    // or a dead trigger — reading crosstalk: it would open the gate on
    // everything else's.
    let loudest = levels.iter().map(|(h, _)| *h).fold(f64::NEG_INFINITY, f64::max);
    let (hits, floor) = levels
        .iter()
        .copied()
        .filter(|(h, _)| *h >= loudest - GATE_UNDER_HITS_DB)
        .reduce(|(h, f), (h2, f2)| (h.min(h2), f.max(f2)))?;
    Some(
        (hits - GATE_UNDER_HITS_DB)
            .max(floor + GATE_OVER_FLOOR_DB)
            .min(hits - GATE_OVER_FLOOR_DB),
    )
}

/// A recording's floor, dBFS: the 20th percentile of its peaks — the
/// gaps between hits, which a drum trigger is mostly made of.
fn floor_db(points: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = points.iter().map(|p| p.abs()).collect();
    sorted.sort_by(f64::total_cmp);
    let at = sorted.len().saturating_sub(1) / 5;
    sorted
        .get(at)
        .filter(|p| **p > 0.0)
        .map_or(-120.0, |p| 20.0 * p.log10())
}
