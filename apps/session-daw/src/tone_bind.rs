//! The strip's Tone rack bound to the FTS processors on the track.
//!
//! The rack edits settings the window owns ([`crate::tone::Store`]); this
//! is what turns them into the track's chain — FTS EQ, Comp and Saturate,
//! put on bypassed by [`crate::mix_chains`] — as the plugins' own
//! parameters, each written in the plugin's own words (`-18.0 dB`, `3.0:1`,
//! `Low Shelf`, `Triode`) and resolved by the plugin
//! ([`builtin_fx::setting`]), so no plugin's ranges are assumed here.
//!
//! - EQ: every band the plugin has — the rack's bands, on or off as they
//!   are, and the rest off (the plugin's own defaults have bands on).
//! - Comp: threshold, ratio, knee, attack, release.
//! - Saturate: the profile (`Space`), and the knobs the rack's stage was
//!   made from by the plugin's own `apply` — drive and bias, inverted from
//!   it — and the mix.

use crate::tone::Tone;

/// One FX's wanted parameters: its name, and `(index, normalized value)`.
pub type Stage = (&'static str, Vec<(u32, f64)>);

/// The plugin's name for a band type.
const fn shape(shape: eq_ui::eq_graph_model::EqBandShape) -> &'static str {
    use eq_ui::eq_graph_model::EqBandShape as S;
    match shape {
        S::Bell => "Bell",
        S::LowShelf => "Low Shelf",
        S::HighShelf => "High Shelf",
        S::LowCut => "Low Cut",
        S::HighCut => "High Cut",
        S::Notch => "Notch",
        S::BandPass => "Bandpass",
        S::TiltShelf => "Tilt Shelf",
        S::FlatTilt => "Flat Tilt",
        S::AllPass => "All Pass",
    }
}

/// What `tone` asks of the track's three processors.
#[must_use]
pub fn stages(tone: &Tone) -> Vec<Stage> {
    vec![
        ("FTS EQ", eq(tone)),
        ("FTS Comp", comp(tone)),
        ("FTS Saturate", sat(tone)),
    ]
}

/// `(index, value)` for `fx`'s `param` at `text`, if the plugin knows it.
fn at(fx: &str, param: &str, text: &str) -> Option<(u32, f64)> {
    builtin_fx::setting(fx, param, text)
}

fn eq(tone: &Tone) -> Vec<(u32, f64)> {
    const FX: &str = "FTS EQ";
    let mut out = Vec::new();
    // Every band the plugin has: `B1 On` … until there is no such band.
    let mut bands = tone.eq.iter().filter(|b| b.used);
    for n in 1_u32.. {
        let on = format!("B{n} On");
        let band = bands.next();
        let Some(switch) = at(
            FX,
            &on,
            if band.is_some_and(|b| b.enabled) {
                "On"
            } else {
                "Off"
            },
        ) else {
            break;
        };
        out.push(switch);
        let Some(band) = band else { continue };
        for (param, text) in [
            ("Type", shape(band.shape).to_owned()),
            ("Freq", format!("{:.1} Hz", band.frequency)),
            ("Gain", format!("{:.1} dB", band.gain)),
            ("Q", format!("{:.2}", band.q)),
        ] {
            out.extend(at(FX, &format!("B{n} {param}"), &text));
        }
    }
    out
}

fn comp(tone: &Tone) -> Vec<(u32, f64)> {
    const FX: &str = "FTS Comp";
    let c = tone.comp;
    [
        ("Threshold", format!("{:.1} dB", c.threshold)),
        ("Ratio", format!("{:.1}:1", c.ratio)),
        ("Knee", format!("{:.1} dB", c.knee)),
        ("Attack", format!("{:.1} ms", c.attack)),
        ("Release", format!("{:.0} ms", c.release)),
    ]
    .iter()
    .filter_map(|(param, text)| at(FX, param, text))
    .collect()
}

fn sat(tone: &Tone) -> Vec<(u32, f64)> {
    const FX: &str = "FTS Saturate";
    let Some(profile) = saturate_profiles::PROFILES.get(tone.sat_profile) else {
        return Vec::new();
    };
    let v = profile.voicing;
    // `apply` made the stage from the knobs; these are the knobs again.
    let drive = ((tone.sat.drive - 1.0) / (15.0 * v.drive_scale.max(f32::EPSILON))).clamp(0.0, 1.0);
    let bias = ((tone.sat.q_point - v.q_point) / 1.2 + 0.5).clamp(0.0, 1.0);
    [
        ("Space", profile.name.to_owned()),
        ("Drive", format!("{:.0}%", drive * 100.0)),
        ("Bias", format!("{:.0}%", bias * 100.0)),
        (
            "Mix",
            format!("{:.0}%", tone.sat.mix.clamp(0.0, 1.0) * 100.0),
        ),
    ]
    .iter()
    .filter_map(|(param, text)| at(FX, param, text))
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::engine::Edit;
    use crate::tone::Store;

    fn store() -> Store {
        let track = daw_proto::Track {
            guid: "t".into(),
            name: "Kick In".into(),
            ..daw_proto::Track::default()
        };
        let mut store = Store::default();
        store.seed(&[(track, 0)]);
        store
    }

    /// A stage turned for the first time is switched on and sent whole —
    /// in the plugin's own terms: the threshold set reads back as set —
    /// and after that only what moved goes; an untouched stage stays off.
    #[test]
    fn the_rack_turned_sets_the_tracks_processor() {
        let mut store = store();
        store.edit("t").expect("t").comp.threshold = -30.0;
        let first = store.flush();
        let comp: Vec<&Edit> = first.iter().filter(|e| matches!(e, Edit::SetFxParam(_, fx, ..) | Edit::SetFxEnabled(_, fx, _) if fx == "FTS Comp")).collect();
        assert!(first.contains(&Edit::SetFxEnabled("t".into(), "FTS Comp".into(), true)));
        assert_eq!(comp.len(), 6, "five parameters and the switch: {comp:?}");
        assert!(first.iter().all(|e| !matches!(e, Edit::SetFxParam(_, fx, ..) | Edit::SetFxEnabled(_, fx, _) if fx != "FTS Comp")), "EQ and Saturate untouched: {first:?}");
        let (index, value) =
            builtin_fx::setting("FTS Comp", "Threshold", "-30.0 dB").expect("a threshold");
        assert!(first.contains(&Edit::SetFxParam(
            "t".into(),
            "FTS Comp".into(),
            index,
            value
        )));

        store.edit("t").expect("t").comp.release = 300.0;
        let second = store.flush();
        assert_eq!(second.len(), 1, "only the release: {second:?}");
        assert!(matches!(&second[0], Edit::SetFxParam(_, fx, ..) if fx == "FTS Comp"));
        assert!(store.flush().is_empty(), "nothing moved since");
    }

    /// The EQ's first change sends every band the plugin has: the rack's,
    /// and the rest off — the plugin's own defaults have bands on.
    #[test]
    fn the_eq_sends_every_band() {
        let mut store = store();
        if let Some(tone) = store.edit("t") {
            tone.eq.clear();
        }
        let edits = store.flush();
        let off = builtin_fx::setting("FTS EQ", "B2 On", "Off").expect("B2 On");
        assert!(
            edits.contains(&Edit::SetFxParam("t".into(), "FTS EQ".into(), off.0, off.1)),
            "{edits:?}"
        );
        assert!(edits.contains(&Edit::SetFxEnabled("t".into(), "FTS EQ".into(), true)));
    }
}
