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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chains {
    pub tone: usize,
    pub buses: usize,
    pub process: usize,
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
        let names = if bus {
            BUS
        } else if recorded && !referenced {
            TONE
        } else {
            continue;
        };
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
        if bus {
            done.buses += 1;
        } else {
            done.tone += 1;
        }
    }
    Ok(done)
}
