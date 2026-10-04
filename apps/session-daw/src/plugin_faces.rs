//! The processors' own studio faces, over the rack's settings.
//!
//! frame's plugin sets (`comp/`, `eq/`, `saturate/`, `modulation/`) are
//! the effects as their plugins look: a blackface FET limiter, an opto
//! leveler, a four-band console EQ, a tube, a tape machine, a chorus.
//! Each speaks its own controls (`fet/input`, `consoleeq/lf_gain`…), and
//! the rack's settings are generic — a threshold, a list of bands — so
//! each face here is an adapter: the settings as the face's values, and
//! the face's edits back into the settings.

use crate::macros::{frac, span};
use crate::tone::{Tone, Which};

/// Which compressor a track wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CompFace {
    /// The blackface FET limiter (an 1176): fast, for what transients.
    Fet,
    /// The opto leveler (an LA-2A): slow and smooth, for a voice.
    Opto,
    /// A VCA compressor: the plain one.
    #[default]
    Vca,
}

impl CompFace {
    /// The face a track's Tone compressor wears, by what the track is:
    /// a drum an 1176, a voice an LA-2A.
    #[must_use]
    pub fn for_track(name: &str, ancestors: &[String]) -> Self {
        let within = |word: &str| is_named(name, ancestors, word);
        if is_drum(name, ancestors) {
            Self::Fet
        } else if within("vocal") || within("vox") || within("bgv") {
            Self::Opto
        } else {
            Self::Vca
        }
    }

    /// What its header says it is.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Fet => "FET 1176",
            Self::Opto => "Opto LA-2A",
            Self::Vca => "VCA",
        }
    }

    const fn design(self) -> &'static str {
        match self {
            Self::Fet => "comp/01-fet-black",
            Self::Opto => "comp/04-opto-leveler",
            Self::Vca => "comp/09-vca-comp",
        }
    }
}

/// Whether `word` is in the track's name or any folder over it.
#[must_use]
pub fn is_named(name: &str, ancestors: &[String], word: &str) -> bool {
    ancestors
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(name))
        .any(|a| a.to_lowercase().contains(word))
}

/// Whether the track is a drum: a kick, a snare, a tom.
#[must_use]
pub fn is_drum(name: &str, ancestors: &[String]) -> bool {
    ["kick", "snare", "tom"]
        .iter()
        .any(|word| is_named(name, ancestors, word))
}

/// `design` in the shape `suffix` names (`-lane`, `-tall`) where that
/// has been designed, else `design` itself.
fn shaped(design: &str, suffix: &str) -> String {
    let wanted = format!("{design}{suffix}");
    if size_of(&wanted).is_some() {
        wanted
    } else {
        design.to_owned()
    }
}

/// Whether `design` is one of the lane-sized shapes.
#[must_use]
pub fn is_shaped(design: &str) -> bool {
    design.ends_with("-lane") || design.ends_with("-tall")
}

/// Designs' sizes, by design, as read (`None`: unreadable).
type Sizes = std::sync::Mutex<std::collections::HashMap<String, Option<(f64, f64)>>>;

/// A design's size, read from its markup's first frame once.
fn size_of(design: &str) -> Option<(f64, f64)> {
    static SIZES: std::sync::OnceLock<Sizes> = std::sync::OnceLock::new();
    let sizes = SIZES.get_or_init(Default::default);
    if let Some(known) = sizes.lock().ok().and_then(|s| s.get(design).copied()) {
        return known;
    }
    let path = crate::faces::design_dir().join(format!("{design}.frame"));
    let found = std::fs::read_to_string(path).ok().and_then(|text| {
        let frame = text.find("<Frame ")?;
        let line = text.get(frame..)?.lines().next()?;
        let number = |key: &str| -> Option<f64> {
            let rest = line.get(line.find(key)?..)?.strip_prefix(key)?;
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            rest.get(..end)?.parse().ok()
        };
        Some((number(" w=")?, number(" h=")?))
    });
    if let Ok(mut s) = sizes.lock() {
        s.insert(design.to_owned(), found);
    }
    found
}

/// The design `which` wears over `tone`, if a plugin face serves it.
fn design(which: Which, tone: &Tone) -> Option<&'static str> {
    match which {
        Which::Comp => Some(tone.comp_face.design()),
        Which::RescueComp if tone.comp_face == CompFace::Opto => Some(CompFace::Fet.design()),
        Which::Sat => Some(match sat_circuit(tone) {
            saturate_ui::viz::Circuit::Valve => "saturate/01-tube",
            saturate_ui::viz::Circuit::Tape => "saturate/02-tape",
            saturate_ui::viz::Circuit::Core => "saturate/03-transformer",
            saturate_ui::viz::Circuit::Solid => "saturate/04-transistor",
            saturate_ui::viz::Circuit::Steps => "saturate/05-digital",
        }),
        Which::Mod => match tone.modulation.engine {
            modulation_ui::viz::Engine::Chorus => Some("modulation/01-chorus"),
            modulation_ui::viz::Engine::Flanger => Some("modulation/02-flanger"),
            modulation_ui::viz::Engine::Vibrato => Some("modulation/03-vibrato"),
            modulation_ui::viz::Engine::Tremolo => Some("modulation/04-tremolo"),
            _ => None,
        },
        _ => None,
    }
}

/// The circuit the saturator's profile is.
fn sat_circuit(tone: &Tone) -> saturate_ui::viz::Circuit {
    saturate_profiles::PROFILES.get(tone.sat_profile).map_or(
        saturate_ui::viz::Circuit::Valve,
        |profile| {
            saturate_ui::viz::SatView::of(*profile, saturate_profiles::Controls::default())
                .circuit()
        },
    )
}

/// The namespace a design's controls are under.
fn ns_of(design: &str) -> &'static str {
    match design {
        "comp/01-fet-black" => "fet",
        "comp/04-opto-leveler" => "opto",
        "comp/09-vca-comp" => "vcacomp",
        "eq/02-console-eq" => "consoleeq",
        "saturate/01-tube" => "tube",
        "saturate/02-tape" => "tape_sat",
        "saturate/03-transformer" => "transformer",
        "saturate/04-transistor" => "transistor",
        "saturate/05-digital" => "digital_sat",
        "modulation/01-chorus" => "chorus",
        "modulation/02-flanger" => "flanger",
        "modulation/03-vibrato" => "vibrato",
        "modulation/04-tremolo" => "tremolo",
        _ => "",
    }
}

/// An 1176's attack and release dials run 1 (slow) to 7 (fast): `ms` on
/// a log scale over `lo..hi` ms, the dial the other way round.
fn dial(ms: f32, lo: f32, hi: f32) -> f64 {
    let f = (f64::from(ms.max(lo)) / f64::from(lo)).log(f64::from(hi) / f64::from(lo));
    6.0f64.mul_add(-f.clamp(0.0, 1.0), 7.0)
}

/// …and back.
fn undial(value: f64, lo: f32, hi: f32) -> f32 {
    let f = ((7.0 - value) / 6.0).clamp(0.0, 1.0);
    crate::num::narrow(f64::from(lo) * (f64::from(hi) / f64::from(lo)).powf(f))
}

/// The 1176's ratio buttons.
const FET_RATIOS: [f32; 4] = [4.0, 8.0, 12.0, 20.0];

/// The comp settings `which` stands for.
fn comp_of(tone: &Tone, which: Which) -> crate::tone::Comp {
    if which == Which::RescueComp {
        tone.rescue_comp
    } else {
        tone.comp
    }
}

fn comp_mut(tone: &mut Tone, which: Which) -> &mut crate::tone::Comp {
    if which == Which::RescueComp {
        &mut tone.rescue_comp
    } else {
        &mut tone.comp
    }
}

/// The used EQ bands, in order: a console EQ's four sections are the
/// first four.
fn band(tone: &Tone, n: usize) -> Option<&eq_ui::eq_graph_model::EqBand> {
    tone.eq.iter().filter(|b| b.used).nth(n)
}

fn band_mut(tone: &mut Tone, n: usize) -> Option<&mut eq_ui::eq_graph_model::EqBand> {
    tone.eq.iter_mut().filter(|b| b.used).nth(n)
}

/// What a compressor face shows for `c`, by address.
fn comp_values(ns: &str, c: crate::tone::Comp) -> Vec<(String, f64)> {
    let at = |param: &str| format!("{ns}/{param}");
    match ns {
        "fet" => {
            let ratio = FET_RATIOS
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (*a - c.ratio).abs().total_cmp(&(*b - c.ratio).abs()))
                .map_or(0, |(i, _)| i);
            vec![
                // The input drives into a fixed threshold: the lower the
                // threshold, the further the input is turned up.
                (
                    at("input"),
                    f64::from(c.threshold.mul_add(-1.2, -48.0)).clamp(-48.0, 0.0),
                ),
                (at("output"), -24.0),
                (at("attack"), dial(c.attack, 0.1, 100.0)),
                (at("release"), dial(c.release, 50.0, 1100.0)),
                (at("ratio"), crate::num::coord(ratio) / 3.0),
                (at("on"), 1.0),
            ]
        }
        "opto" => vec![
            (at("peak_reduction"), frac(-c.threshold, 0.0, 40.0) * 100.0),
            (at("gain"), 50.0),
            (at("limit"), if c.ratio > 6.0 { 1.0 } else { 0.0 }),
            (at("power"), 1.0),
        ],
        _ => vec![
            (at("threshold"), f64::from(c.threshold)),
            (at("ratio"), f64::from(c.ratio)),
            (at("output"), 0.0),
        ],
    }
}

/// A compressor face's edit `param` = `value`, onto `c`; whether it was
/// one of its controls.
fn comp_edit(ns: &str, param: &str, c: &mut crate::tone::Comp, value: f64) -> bool {
    let v = crate::num::narrow(value);
    match (ns, param) {
        ("fet", "input") => c.threshold = ((-48.0 - v) / 1.2).clamp(-60.0, 0.0),
        ("fet", "attack") => c.attack = undial(value, 0.1, 100.0),
        ("fet", "release") => c.release = undial(value, 50.0, 1100.0),
        ("fet", "ratio") => {
            let at = crate::num::index((value.clamp(0.0, 1.0) * 3.0).round());
            c.ratio = FET_RATIOS.get(at).copied().unwrap_or(4.0);
        }
        ("opto", "peak_reduction") => c.threshold = -span(value / 100.0, 0.0, 40.0),
        ("opto", "limit") => c.ratio = if value >= 0.5 { 10.0 } else { 3.0 },
        ("vcacomp", "threshold") => c.threshold = v.clamp(-60.0, 0.0),
        ("vcacomp", "ratio") => c.ratio = v.clamp(1.0, 60.0),
        _ => return false,
    }
    true
}

/// The second compressor's face, after the first (`Tone::comp_after`),
/// and its size.
#[must_use]
pub fn face_after(tone: &Tone) -> Option<(crate::faces::FrameFace, (f64, f64))> {
    let (comp, face) = tone.comp_after?;
    let design = face.design();
    let shaped = shaped(design, "-lane");
    let size = size_of(&shaped)?;
    Some((
        crate::faces::FrameFace {
            responsive: false,
            design: shaped,
            values: comp_values(ns_of(design), comp),
            texts: Vec::new(),
        },
        size,
    ))
}

/// An edit the second compressor's face made.
pub fn edit_after(tone: &mut Tone, addr: &str, value: f64) -> bool {
    let Some((ns, param)) = addr.split_once('/') else {
        return false;
    };
    tone.comp_after
        .as_mut()
        .is_some_and(|(comp, _)| comp_edit(ns, param, comp, value))
}

/// What a device's header calls it: the face it wears, where it wears
/// one.
#[must_use]
pub fn title(which: Which, tone: &Tone) -> String {
    match which {
        Which::Comp if tone.comp_after.is_some() => {
            let after = tone.comp_after.map_or("", |(_, f)| f.title());
            format!("{} → {after}", tone.comp_face.title())
        }
        Which::Comp => tone.comp_face.title().to_owned(),
        Which::Sat => saturate_profiles::PROFILES
            .get(tone.sat_profile)
            .map_or_else(|| "Saturation".to_owned(), |p| p.name.to_owned()),
        Which::Mod => format!("{:?}", tone.modulation.engine),
        Which::Reverb => tone.reverb.algorithm.name().to_owned(),
        _ => which.name().to_owned(),
    }
}

/// The values `design` shows for `which` over `tone`, by address.
fn values(design: &str, which: Which, tone: &Tone) -> Vec<(String, f64)> {
    let ns = ns_of(design);
    let at = |param: &str| format!("{ns}/{param}");
    let pct = |v: f32| f64::from(v.clamp(0.0, 1.0)) * 100.0;
    match ns {
        "fet" | "opto" | "vcacomp" => comp_values(ns, comp_of(tone, which)),
        "consoleeq" => {
            let mut out = vec![
                (at("eq_in"), 1.0),
                (at("filters_in"), 0.0),
                (at("hpf"), 16.0),
                (at("lpf"), 22_000.0),
            ];
            for (n, (gain, freq, q)) in [
                ("lf_gain", "lf_freq", None),
                ("lmf_gain", "lmf_freq", Some("lmf_q")),
                ("hmf_gain", "hmf_freq", Some("hmf_q")),
                ("hf_gain", "hf_freq", None),
            ]
            .iter()
            .enumerate()
            {
                if let Some(b) = band(tone, n) {
                    out.push((at(gain), f64::from(b.gain)));
                    out.push((at(freq), f64::from(b.frequency)));
                    if let Some(q) = q {
                        out.push((at(q), f64::from(b.q)));
                    }
                }
            }
            out
        }
        "tube" | "tape_sat" | "transformer" | "transistor" | "digital_sat" => {
            let scale = saturate_profiles::PROFILES
                .get(tone.sat_profile)
                .map_or(1.0, |p| p.voicing.drive_scale)
                .max(f32::EPSILON);
            let drive = ((tone.sat.drive - 1.0) / (15.0 * scale)).clamp(0.0, 1.0);
            vec![
                (at("drive"), pct(drive)),
                (at("mix"), pct(tone.sat.mix)),
                (at("output"), 0.0),
                (
                    at("bias"),
                    f64::from(tone.sat.q_point.clamp(-1.0, 1.0)) * 50.0,
                ),
                (at("power"), 1.0),
                (at("on"), 1.0),
            ]
        }
        "chorus" | "flanger" | "vibrato" | "tremolo" => {
            let w = tone.modulation;
            vec![
                (at("rate"), f64::from(w.rate)),
                (at("depth"), pct(w.depth)),
                (at("mix"), pct(w.mix)),
                (at("on"), 1.0),
            ]
        }
        _ => Vec::new(),
    }
}

/// The plugin face `which` wears over `tone`, and its size.
#[must_use]
pub fn face(which: Which, tone: &Tone) -> Option<(crate::faces::FrameFace, (f64, f64))> {
    let base = design(which, tone)?;
    // A compressor wears the shape designed for where it stands: one of
    // a stacked pair (`-lane`), or alone filling the column (`-tall`); a
    // saturator its `-tall` — the plugin's own face where those are not
    // designed yet.
    let shaped = match which {
        Which::Comp | Which::RescueComp => {
            let paired = which == Which::Comp && tone.comp_after.is_some();
            shaped(base, if paired { "-lane" } else { "-tall" })
        }
        // A saturator stands alone in its column, the compressors' size:
        // its 960-wide face fitted to the lane's height ran half as wide
        // again as they do.
        Which::Sat => shaped(base, "-tall"),
        _ => base.to_owned(),
    };
    let size = size_of(&shaped)?;
    Some((
        crate::faces::FrameFace {
            responsive: false,
            design: shaped,
            values: values(base, which, tone),
            texts: Vec::new(),
        },
        size,
    ))
}

/// Apply an edit a plugin face made on `which` to `tone`; whether it
/// was one this adapter knows.
pub fn edit(which: Which, tone: &mut Tone, addr: &str, value: f64) -> bool {
    let Some((ns, param)) = addr.split_once('/') else {
        return false;
    };
    let v = crate::num::narrow(value);
    match (ns, param) {
        ("fet" | "opto" | "vcacomp", _) => {
            // The second compressor's face speaks its own controls: an
            // edit in its namespace (and not the first's) is its.
            let after = tone.comp_after.map(|(_, f)| ns_of(f.design()));
            if which == Which::Comp && after == Some(ns) && ns_of(tone.comp_face.design()) != ns {
                return edit_after(tone, addr, value);
            }
            return comp_edit(ns, param, comp_mut(tone, which), value);
        }
        ("consoleeq", p) => {
            let (n, field) = match p {
                "lf_gain" => (0, 'g'),
                "lf_freq" => (0, 'f'),
                "lmf_gain" => (1, 'g'),
                "lmf_freq" => (1, 'f'),
                "lmf_q" => (1, 'q'),
                "hmf_gain" => (2, 'g'),
                "hmf_freq" => (2, 'f'),
                "hmf_q" => (2, 'q'),
                "hf_gain" => (3, 'g'),
                "hf_freq" => (3, 'f'),
                _ => return false,
            };
            let Some(b) = band_mut(tone, n) else {
                return false;
            };
            match field {
                'g' => b.gain = v,
                'f' => b.frequency = v,
                _ => b.q = v,
            }
        }
        ("tube" | "tape_sat" | "transformer" | "transistor" | "digital_sat", "drive") => {
            let scale = saturate_profiles::PROFILES
                .get(tone.sat_profile)
                .map_or(1.0, |p| p.voicing.drive_scale);
            tone.sat.drive = (15.0 * scale).mul_add(span(value / 100.0, 0.0, 1.0), 1.0);
        }
        ("tube" | "tape_sat" | "transformer" | "transistor" | "digital_sat", "mix") => {
            tone.sat.mix = span(value / 100.0, 0.0, 1.0);
        }
        ("tube" | "tape_sat" | "transformer" | "transistor" | "digital_sat", "bias") => {
            tone.sat.q_point = (v / 50.0).clamp(-1.0, 1.0);
        }
        ("chorus" | "flanger" | "vibrato" | "tremolo", "rate") => {
            tone.modulation.rate = v.clamp(0.05, 20.0);
        }
        ("chorus" | "flanger" | "vibrato" | "tremolo", "depth") => {
            tone.modulation.depth = span(value / 100.0, 0.0, 1.0);
        }
        ("chorus" | "flanger" | "vibrato" | "tremolo", "mix") => {
            tone.modulation.mix = span(value / 100.0, 0.0, 1.0);
        }
        _ => return false,
    }
    true
}
