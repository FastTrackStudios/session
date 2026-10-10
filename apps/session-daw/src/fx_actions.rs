//! A track's FX over the Action List.
//!
//! Every processor on it, every parameter of each, read and set — in the
//! plugin's own words where the plugin is one of the FTS processors
//! (`-18 dB`, `Plate`, `1/2`), or as a normalized 0..1 for any plugin.
//!
//! An FX is named by its name in the chain (`FTS Reverb`, `Pro-Q 4`, any
//! part of it, case aside) or its place (`#2`, from 1); a parameter by its
//! name (`Decay`; a name a plugin repeats — the reverb's six `Post Freq` —
//! with its count: `Post Freq 6`) or its index (`#12`, from 0).

use architect::action::{ActionList, ActionMeta, Effect};
use daw::service::{Effects, FxChainContext, FxRef, FxTarget, ProjectContext};
use facet::Facet;

use crate::actions::{meta, resolve, song};

static LIST: ActionMeta = meta(
    "FX_LIST",
    "fx_list",
    "A track's FX",
    "The processors on `track` (its GUID, path or name), in chain order: each one's place (`#1`…), name, whether it is on, and how many parameters it has. Read-only.",
    "FX",
    "",
);

static PARAMS: ActionMeta = meta(
    "FX_PARAMS",
    "fx_params",
    "An FX's parameters",
    "Every parameter of one FX on `track` — `fx` its name (any part) or place (`#2`) — each with its index, name, its value as the plugin writes it (`-18.0 dB`, `Plate`) and normalized (0..1), and for a choice its choices. `filter` keeps the names containing it. Read-only.",
    "FX",
    "",
);

static SET: ActionMeta = meta(
    "FX_SET",
    "fx_set",
    "Set FX parameters",
    "Set parameters of one FX on `track` (`fx` its name or `#place`). `set` maps each parameter (name — `Post Freq 6` for the sixth of a repeated one — or `#index`) to a value: the plugin's own words for an FTS processor or any choice (`-18 dB`, `4:1`, `Plate`, `1/2`, `On`), or `=0.42` for a normalized value on any plugin. Says what each now reads, or why not. One undo step.",
    "FX",
    "",
);

static ENABLE: ActionMeta = meta(
    "FX_ENABLE",
    "fx_enable",
    "Switch an FX on or off",
    "Bypass (`on: false`) or un-bypass (`on: true`) one FX on `track` (`fx` its name or `#place`).",
    "FX",
    "",
);

/// A track.
#[derive(Facet)]
struct TrackArg {
    track: String,
}

/// One FX on a track, its parameters narrowed by name.
#[derive(Facet)]
struct ParamsArg {
    track: String,
    fx: String,
    filter: Option<String>,
}

/// Parameters to set on one FX.
#[derive(Facet)]
struct SetArg {
    track: String,
    fx: String,
    set: std::collections::HashMap<String, String>,
}

/// One FX on or off.
#[derive(Facet)]
struct EnableArg {
    track: String,
    fx: String,
    on: bool,
}

/// An FX in a chain, as listed.
#[derive(Facet)]
struct FxRow {
    place: String,
    name: String,
    on: bool,
    parameters: u32,
}

/// One parameter, as listed.
#[derive(Facet)]
struct ParamRow {
    index: u32,
    name: String,
    value: String,
    normalized: f64,
    choices: Vec<String>,
}

/// `track`'s chain and the FX `which` names in it.
fn target(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: &str,
    which: &str,
) -> Result<(FxTarget, daw_proto::fx::Fx), String> {
    let guid = resolve(daw, project, track)?;
    let context = FxChainContext::Track(guid);
    let chain = Effects::list(daw, project.clone(), context.clone());
    let which = which.trim();
    let found = if let Some(place) = which.strip_prefix('#') {
        let place: usize = place
            .parse()
            .map_err(|_| format!("{which:?} is not a place"))?;
        chain
            .get(place.checked_sub(1).ok_or("places count from #1")?)
            .cloned()
    } else {
        let lower = which.to_lowercase();
        let named: Vec<&daw_proto::fx::Fx> = chain
            .iter()
            .filter(|fx| fx.name.to_lowercase().contains(&lower))
            .collect();
        match named.as_slice() {
            [] => None,
            [one] => Some((*one).clone()),
            many => {
                // An exact name wins over the others it is part of.
                many.iter()
                    .find(|fx| fx.name.eq_ignore_ascii_case(which))
                    .map(|fx| (*fx).clone())
                    .ok_or_else(|| {
                        format!(
                            "{which:?} could be any of {} — name one, or its #place",
                            many.iter()
                                .map(|fx| fx.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .map(Some)?
            }
        }
    };
    let fx = found.ok_or_else(|| {
        format!(
            "no {which:?} on the track — it has {}",
            chain
                .iter()
                .map(|fx| fx.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    Ok((
        FxTarget {
            context,
            fx: FxRef::Guid(fx.guid.clone()),
        },
        fx,
    ))
}

/// The parameter `which` names among `params`: `#index`, a name, or a
/// repeated name with its count (`Post Freq 6`).
fn param_of<'a>(
    params: &'a [daw_proto::fx::FxParameter],
    which: &str,
) -> Result<&'a daw_proto::fx::FxParameter, String> {
    let which = which.trim();
    if let Some(index) = which.strip_prefix('#') {
        let index: u32 = index
            .parse()
            .map_err(|_| format!("{which:?} is not an index"))?;
        return params
            .iter()
            .find(|p| p.index == index)
            .ok_or_else(|| format!("no parameter #{index}"));
    }
    let named: Vec<&daw_proto::fx::FxParameter> = params
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(which))
        .collect();
    if let [one] = named.as_slice() {
        return Ok(one);
    }
    if named.len() > 1 {
        return Err(format!(
            "{which:?} is {} parameters — say which: `{which} 1`…`{which} {}`",
            named.len(),
            named.len()
        ));
    }
    // A repeated name with its count.
    if let Some((base, n)) = which.rsplit_once(' ')
        && let Ok(n) = n.parse::<usize>()
    {
        let same: Vec<&daw_proto::fx::FxParameter> = params
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(base))
            .collect();
        if let Some(found) = n.checked_sub(1).and_then(|i| same.get(i)) {
            return Ok(found);
        }
    }
    Err(format!("no parameter {which:?} (FX_PARAMS lists them)"))
}

/// The normalized value for `text` on `param` of `fx`: `=0.42` as given;
/// a choice by its label; else the plugin's own reading of the words (an
/// FTS processor's, `builtin_fx::setting`).
fn value_of(fx: &str, param: &daw_proto::fx::FxParameter, text: &str) -> Result<f64, String> {
    let text = text.trim();
    if let Some(raw) = text.strip_prefix('=') {
        let v: f64 = raw
            .trim()
            .parse()
            .map_err(|_| format!("{text:?} is not a number"))?;
        return Ok(v.clamp(0.0, 1.0));
    }
    if let Some((v, _)) = param
        .step_labels
        .iter()
        .find(|(_, label)| label.eq_ignore_ascii_case(text))
    {
        return Ok(*v);
    }
    // The FTS processors read their own words; the index found by name is
    // the first of a repeated one, so the value is the same either way.
    builtin_fx::setting(fx, &param.name, text)
        .map(|(_, v)| v)
        .ok_or_else(|| {
            format!(
                "{} does not read {text:?} for {} — use its own words, or `=0..1`",
                fx, param.name
            )
        })
}

pub(crate) fn register(list: &ActionList) {
    list.register(&LIST, Effect::ReadOnly, |a: TrackArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        let context = FxChainContext::Track(guid);
        Ok(Effects::list(daw, project.clone(), context.clone())
            .into_iter()
            .enumerate()
            .map(|(i, fx)| FxRow {
                place: format!("#{}", i.saturating_add(1)),
                parameters: u32::try_from(
                    Effects::parameters(
                        daw,
                        project.clone(),
                        FxTarget {
                            context: context.clone(),
                            fx: FxRef::Guid(fx.guid.clone()),
                        },
                    )
                    .len(),
                )
                .unwrap_or(u32::MAX),
                name: fx.name,
                on: fx.enabled,
            })
            .collect::<Vec<_>>())
    });
    list.register(&PARAMS, Effect::ReadOnly, |a: ParamsArg| {
        let (daw, project) = song()?;
        let (target, _) = target(daw, &project, &a.track, &a.fx)?;
        let filter = a.filter.map(|f| f.to_lowercase());
        Ok(Effects::parameters(daw, project, target)
            .into_iter()
            .filter(|p| {
                filter
                    .as_ref()
                    .is_none_or(|f| p.name.to_lowercase().contains(f))
            })
            .map(|p| ParamRow {
                index: p.index,
                choices: p.step_labels.iter().map(|(_, l)| l.clone()).collect(),
                name: p.name,
                value: p.formatted,
                normalized: p.value,
            })
            .collect::<Vec<_>>())
    });
    list.register(&SET, Effect::Mutating, |a: SetArg| {
        let (daw, project) = song()?;
        let (target, fx) = target(daw, &project, &a.track, &a.fx)?;
        let params = Effects::parameters(daw, project.clone(), target.clone());
        let mut said = Vec::new();
        let mut settings: Vec<(&String, &String)> = a.set.iter().collect();
        settings.sort();
        for (which, text) in settings {
            let outcome = param_of(&params, which).and_then(|param| {
                let value = value_of(&fx.name, param, text)?;
                Effects::set_parameter(
                    daw,
                    project.clone(),
                    daw_proto::fx::SetParameterRequest {
                        target: target.clone(),
                        index: param.index,
                        value,
                    },
                )
                .map_err(|e| e.to_string())?;
                let now = Effects::parameter(daw, project.clone(), target.clone(), param.index)
                    .map_or_else(String::new, |p| p.formatted);
                // As asked: `Post Freq 6`, not the name all six share.
                Ok(format!("{} = {now}", which.trim()))
            });
            said.push(outcome.unwrap_or_else(|e| format!("{which}: {e}")));
        }
        crate::actions::list().changed("fx", &fx.guid);
        Ok(said)
    });
    list.register(&ENABLE, Effect::Mutating, |a: EnableArg| {
        let (daw, project) = song()?;
        let (target, fx) = target(daw, &project, &a.track, &a.fx)?;
        Effects::set_enabled(daw, project, target, a.on).map_err(|e| e.to_string())?;
        Ok(format!(
            "{} {}",
            fx.name,
            if a.on { "on" } else { "bypassed" }
        ))
    });
}
