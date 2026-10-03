//! A guitar track's signal rig, drawn the way signal draws it.
//!
//! Some guitars are not a channel strip's chain at all: they go through
//! a signal rig — pedals, two amps in stereo, a modulation and a Time
//! module — and their FX lane is that rig. The rig's blocks are what
//! signal composes for a profile's patch (`signal-guitar`'s `flatten` and
//! `build_profile`, dumped by its `rig_dump` example into `rigs/`), and
//! each block wears the face frame designed for it
//! (`rig-faces/faces.json`): matched the way signal's rig view matches
//! them, fed the same values under the same `<ns>/<param>` addresses.
//!
//! The sections are signal's: Input (pitch, wah, volume), Pre (the pre
//! effects), Core (drives, amps, gate, the patch's trim), Mod (the
//! modulation and motion effects) and Time (delays, reverbs).

use std::collections::HashMap;

use reverb_dsp::algorithm::AlgorithmType;

/// One block of a rig, as the rig presents it to its view.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub name: String,
    /// Its kind, as signal names block types (`amp`, `drive`, `reverb`…).
    pub kind: String,
    pub bypassed: bool,
    /// What is loaded: a pedal, an amp's composed name, a block preset.
    pub preset: String,
    /// The capture file, for an amp or a pedal.
    pub asset: String,
    /// A drive slot with no pedal in it.
    pub empty: bool,
    pub option: Option<f64>,
    pub output_level_db: Option<f64>,
    pub params: Vec<(String, f64)>,
}

impl Block {
    /// Param `name`, or `default`.
    #[must_use]
    pub fn param(&self, name: &str, default: f64) -> f64 {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map_or(default, |(_, v)| *v)
    }

    fn set(&mut self, name: &str, value: f64) {
        if let Some((_, v)) = self.params.iter_mut().find(|(n, _)| n == name) {
            *v = value;
        } else {
            self.params.push((name.to_owned(), value));
        }
    }
}

/// A composed rig: its profile and patch, its blocks in chain order, and
/// which preset each block's nameplate shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    pub profile: String,
    pub patch: String,
    pub blocks: Vec<Block>,
    pub active: HashMap<String, String>,
}

impl Rig {
    /// A rig from `rig_dump`'s JSON.
    #[must_use]
    pub fn parse(json: &str) -> Option<Self> {
        let doc: serde_json::Value = serde_json::from_str(json).ok()?;
        let text = |v: &serde_json::Value, k: &str| {
            v.get(k)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let blocks = doc
            .get("blocks")?
            .as_array()?
            .iter()
            .map(|b| Block {
                name: text(b, "name"),
                kind: text(b, "block_type"),
                bypassed: b
                    .get("bypassed")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                preset: text(b, "preset"),
                asset: text(b, "asset"),
                empty: b
                    .get("empty")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                option: b.get("option").and_then(serde_json::Value::as_f64),
                output_level_db: b.get("output_level_db").and_then(serde_json::Value::as_f64),
                params: b
                    .get("params")
                    .and_then(serde_json::Value::as_array)
                    .map(|ps| {
                        ps.iter()
                            .filter_map(|p| {
                                Some((
                                    p.get("name")?.as_str()?.to_owned(),
                                    p.get("value")?.as_f64()?,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        let active = doc
            .get("active_blocks")
            .and_then(serde_json::Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            profile: text(&doc, "profile"),
            patch: text(&doc, "patch"),
            blocks,
            active,
        })
    }

    /// Write an edit a face made, `<ns>/<param>` = `value`, to block `at`.
    pub fn edit(&mut self, at: usize, ns: &str, addr: &str, value: f64, face: &FaceEntry) {
        let Some(block) = self.blocks.get_mut(at) else {
            return;
        };
        let Some(name) = addr.strip_prefix(ns).and_then(|s| s.strip_prefix('/')) else {
            return;
        };
        if let Some(map) = face.maps.iter().find(|m| m.face == name) {
            block.set(&map.param.clone(), map.to_block(value));
            return;
        }
        match name {
            "on" => block.bypassed = value < 0.5,
            "level" => block.output_level_db = Some(value),
            "option" => block.option = Some(value),
            "time_knob" => turn(block, value),
            _ => block.set(name, value),
        }
    }
}

/// The rigs this session's guitars play: the Blues profile's default
/// patch (two amps, stereo) on the Rhythm part, Worship's Clean on the
/// Lead — until a track records which rig it is.
#[must_use]
pub fn for_track(name: &str, parent: Option<&str>) -> Option<Rig> {
    if parent != Some("Electric") {
        return None;
    }
    let json = match name {
        "Rhythm" => include_str!("../rigs/blues-vibroverb-dumble.json"),
        "Lead" => include_str!("../rigs/worship-clean.json"),
        _ => return None,
    };
    Rig::parse(json)
}

/// Where a block sits in the rig, as signal's rig view groups it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Input,
    Pre,
    Core,
    Mod,
    Time,
}

impl Section {
    pub const ALL: [Self; 5] = [Self::Input, Self::Pre, Self::Core, Self::Mod, Self::Time];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Pre => "Pre",
            Self::Core => "Core",
            Self::Mod => "Mod",
            Self::Time => "Time",
        }
    }

    /// Its tint, as the rig's own colours run: input grey, pre amber,
    /// core red, mod cyan, time violet.
    #[must_use]
    pub const fn tint(self) -> vello::peniko::Color {
        let [r, g, b] = match self {
            Self::Input => [148, 163, 184],
            Self::Pre => [245, 158, 11],
            Self::Core => [239, 68, 68],
            Self::Mod => [34, 211, 238],
            Self::Time => [167, 139, 250],
        };
        vello::peniko::Color::from_rgb8(r, g, b)
    }
}

const MOD_KINDS: [&str; 6] = ["chorus", "phaser", "flanger", "trem", "vibrato", "rotary"];

/// Which section `block` is in — `None` for what the view does not draw
/// (the master EQ, the limiter, the post-amp volume).
#[must_use]
pub fn section_of(block: &Block) -> Option<Section> {
    let name = block.name.as_str();
    let kind = block.kind.as_str();
    let pre = name.starts_with("Pre ") && name != "Pre Comp";
    if matches!(
        name,
        "Transpose" | "Dive Bomb" | "Pitch" | "Harmonizer" | "Volume Pedal"
    ) || matches!(kind, "wah" | "filter")
    {
        return Some(Section::Input);
    }
    if pre {
        return Some(Section::Pre);
    }
    // The board's boost is a `boost`; the post-amp "Boost" is a volume,
    // which the rig view does not draw.
    if matches!(kind, "drive" | "amp" | "gate" | "boost")
        || matches!(name, "Pre Comp" | "Amp EQ" | "Post Comp" | "Patch Trim")
    {
        return Some(Section::Core);
    }
    if MOD_KINDS.contains(&kind) {
        return Some(Section::Mod);
    }
    if matches!(kind, "delay" | "reverb") {
        return Some(Section::Time);
    }
    None
}

/// A face param standing for a block param.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamMap {
    pub face: String,
    pub param: String,
    pub kind: MapKind,
}

/// How a face param's value becomes its block param's.
#[derive(Clone, Debug, PartialEq)]
pub enum MapKind {
    /// block = face × a + b.
    Linear(f64, f64),
    /// block = table[face].
    Table(Vec<f64>),
}

impl ParamMap {
    fn to_block(&self, face: f64) -> f64 {
        match &self.kind {
            MapKind::Linear(a, b) => face.mul_add(*a, *b),
            MapKind::Table(t) => t
                .get(crate::num::index(face.round()))
                .or_else(|| t.last())
                .copied()
                .unwrap_or(face),
        }
    }

    fn to_face(&self, block: f64) -> f64 {
        match &self.kind {
            MapKind::Linear(a, b) if a.abs() > 1e-12 => (block - b) / a,
            MapKind::Linear(..) => 0.0,
            MapKind::Table(t) => t
                .iter()
                .enumerate()
                .min_by(|(_, x), (_, y)| (*x - block).abs().total_cmp(&(*y - block).abs()))
                .map_or(0.0, |(i, _)| crate::num::coord(i)),
        }
    }
}

/// One face in frame's manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceEntry {
    /// Its design, under the faces directory.
    pub face: String,
    /// The namespace its params are under.
    pub ns: String,
    /// Words that pick it, lower case (`a+b` needs both).
    pub words: Vec<String>,
    /// Its size, for its box's proportion.
    pub size: (f64, f64),
    pub maps: Vec<ParamMap>,
    /// Its dark finish: a Time lane under another draws this.
    pub dark: Option<String>,
    /// It prints its preset on a nameplate.
    pub nameplate: bool,
    /// It carries the Time knob.
    pub time_knob: bool,
}

const SET: &str = "rig-faces";

fn entry(v: &serde_json::Value) -> Option<FaceEntry> {
    let face = v.get("face")?.as_str()?;
    let ns = v.get("ns")?.as_str()?.to_owned();
    let words = v
        .get("match")
        .and_then(serde_json::Value::as_array)
        .map(|m| {
            m.iter()
                .filter_map(|w| w.as_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    let size = v
        .get("size")
        .and_then(serde_json::Value::as_array)
        .and_then(|s| Some((s.first()?.as_f64()?, s.get(1)?.as_f64()?)))
        .unwrap_or((1.0, 1.0));
    let maps = v
        .get("maps")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let face = m.get("face")?.as_str()?.to_owned();
                    let param = m.get("param")?.as_str()?.to_owned();
                    let kind =
                        if let Some(l) = m.get("linear").and_then(serde_json::Value::as_array) {
                            MapKind::Linear(l.first()?.as_f64()?, l.get(1)?.as_f64()?)
                        } else {
                            MapKind::Table(
                                m.get("table")?
                                    .as_array()?
                                    .iter()
                                    .filter_map(serde_json::Value::as_f64)
                                    .collect(),
                            )
                        };
                    Some(ParamMap { face, param, kind })
                })
                .collect()
        })
        .unwrap_or_default();
    let flag = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    Some(FaceEntry {
        face: format!("{SET}/{face}"),
        ns,
        words,
        size,
        maps,
        dark: v
            .get("dark")
            .and_then(serde_json::Value::as_str)
            .map(|d| format!("{SET}/{d}")),
        nameplate: flag("nameplate"),
        time_knob: flag("time_knob"),
    })
}

/// frame's rig faces manifest, read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faces {
    drives: Vec<FaceEntry>,
    amps: Vec<FaceEntry>,
    gate: Option<FaceEntry>,
    eq: Option<FaceEntry>,
    pre: Vec<FaceEntry>,
    input: Vec<FaceEntry>,
    post_comp: Option<FaceEntry>,
    time_delay: Vec<FaceEntry>,
    time_reverb: Vec<FaceEntry>,
    modulation: Vec<FaceEntry>,
    trim: Option<FaceEntry>,
}

impl Faces {
    /// The manifest from frame's faces directory; none (or a bad one) is
    /// an empty set — every block a card.
    #[must_use]
    pub fn load() -> Self {
        let path = crate::faces::design_dir().join(SET).join("faces.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!(target: "frame", manifest = %path.display(), "faces.json does not parse");
            return Self::default();
        };
        let list = |v: Option<&serde_json::Value>| {
            v.and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(entry).collect())
                .unwrap_or_default()
        };
        let time = doc.get("time");
        Self {
            drives: list(doc.get("drives")),
            amps: list(doc.get("amps")),
            gate: doc.get("gate").and_then(entry),
            eq: doc.get("eq").and_then(entry),
            pre: list(doc.get("pre")),
            input: list(doc.get("input")),
            post_comp: doc.get("post_comp").and_then(entry),
            time_delay: list(time.and_then(|t| t.get("delay"))),
            time_reverb: list(time.and_then(|t| t.get("reverb"))),
            modulation: list(doc.get("modulation")),
            trim: doc.get("trim").and_then(entry),
        }
    }

    /// The face `block` wears in `rig`, if frame has one for it.
    #[must_use]
    pub fn of(&self, rig: &Rig, block: &Block) -> Option<&FaceEntry> {
        let kind = block.kind.as_str();
        let name = block.name.as_str();
        match kind {
            "drive" | "boost" if !block.empty => pick(&self.drives, &block.preset),
            "amp" => {
                let file = block
                    .asset
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&block.asset);
                pick(&self.amps, file).or_else(|| pick(&self.amps, &block.preset))
            }
            "gate" => self.gate.as_ref(),
            "delay" if !name.starts_with("Pre ") => exact(
                &self.time_delay,
                DELAY_ALGOS
                    .get(crate::num::index(block.param("style", 1.0).round()))
                    .copied()
                    .unwrap_or(""),
            ),
            "reverb" if !name.starts_with("Pre ") => exact(
                &self.time_reverb,
                AlgorithmType::from_index(crate::num::index(block.param("algorithm", 1.0).round()))
                    .name(),
            ),
            k if MOD_KINDS.contains(&k) && !name.starts_with("Pre ") => exact(&self.modulation, k),
            _ if name.starts_with("Pre ") && name != "Pre Comp" => {
                pick(&self.pre, &block.preset).or_else(|| pick(&self.pre, name))
            }
            _ if name == "Post Comp" => self.post_comp.as_ref(),
            _ if name == "Patch Trim" => self.trim.as_ref(),
            // The amp's EQ wears a face beside a single amp only.
            _ if name == "Amp EQ"
                && rig
                    .blocks
                    .iter()
                    .filter(|b| b.kind == "amp" && !b.asset.is_empty())
                    .count()
                    < 2 =>
            {
                self.eq.as_ref()
            }
            _ if section_of(block) == Some(Section::Input) => {
                pick(&self.input, &name.to_lowercase())
                    .or_else(|| pick(&self.input, &kind.to_lowercase()))
            }
            _ => None,
        }
    }
}

/// `delay::Style` order, as signal's rig view names the machines.
const DELAY_ALGOS: [&str; 13] = [
    "Tape", "Digital", "Analog", "Lo-Fi", "Shimmer", "Reverse", "Ice", "Rhythm", "Drum", "Oil Can",
    "MultiTap", "Spectral", "Filter",
];

/// The first face whose words are in `name` (`a+b`: both).
fn pick<'a>(list: &'a [FaceEntry], name: &str) -> Option<&'a FaceEntry> {
    let name = name.to_lowercase();
    if name.is_empty() {
        return None;
    }
    list.iter().find(|e| {
        e.words
            .iter()
            .any(|w| w.split('+').all(|part| name.contains(part.trim())))
    })
}

/// The face one of whose words is exactly `name`.
fn exact<'a>(list: &'a [FaceEntry], name: &str) -> Option<&'a FaceEntry> {
    let name = name.to_lowercase();
    list.iter().find(|e| e.words.contains(&name))
}

/// A delay's time range the Time knob sweeps (ms), logarithmically.
const DELAY_MS: (f64, f64) = (20.0, 2500.0);

/// A modulation's rate range (Hz), logarithmically.
const MOD_HZ: (f64, f64) = (0.05, 12.0);

/// Seconds as the Time knob reads them.
fn seconds_text(s: f64) -> String {
    if s < 1.0 {
        format!("{:.0} ms", s * 1000.0)
    } else if s < 10.0 {
        format!("{s:.1} s")
    } else {
        format!("{s:.0} s")
    }
}

/// The Time knob for `block`, running free: where it stands, and what it
/// reads.
fn time_view(block: &Block) -> Option<(f64, String)> {
    match block.kind.as_str() {
        "reverb" => {
            let decay = block.param("decay", 0.4);
            let alg =
                AlgorithmType::from_index(crate::num::index(block.param("algorithm", 1.0).round()));
            let variant = crate::num::index(block.param("variant", 0.0).round());
            let secs =
                reverb_dsp::algorithm::decay_seconds(alg, variant, decay).unwrap_or(decay * 8.0);
            Some((decay, seconds_text(secs)))
        }
        "delay" => {
            let ms = block.param("time", 350.0);
            Some((
                (ms / DELAY_MS.0)
                    .log(DELAY_MS.1 / DELAY_MS.0)
                    .clamp(0.0, 1.0),
                format!("{ms:.0} ms"),
            ))
        }
        k if MOD_KINDS.contains(&k) => {
            let hz = block.param("rate", 1.0);
            Some((
                (hz / MOD_HZ.0).log(MOD_HZ.1 / MOD_HZ.0).clamp(0.0, 1.0),
                format!("{hz:.2} Hz"),
            ))
        }
        _ => None,
    }
}

/// A turn of the Time knob to `pos`, running free.
fn turn(block: &mut Block, pos: f64) {
    let pos = pos.clamp(0.0, 1.0);
    match block.kind.as_str() {
        "reverb" => block.set("decay", pos),
        "delay" => block.set("time", DELAY_MS.0 * (DELAY_MS.1 / DELAY_MS.0).powf(pos)),
        k if MOD_KINDS.contains(&k) => {
            block.set("rate", MOD_HZ.0 * (MOD_HZ.1 / MOD_HZ.0).powf(pos));
        }
        _ => {}
    }
}

/// What `face` shows for `block` of `rig`, `dark` for a lane under
/// another of its kind: the face's design and its values and text, by
/// address, in the block's own units — as signal's rig view feeds it.
#[must_use]
pub fn face_of(rig: &Rig, block: &Block, face: &FaceEntry, dark: bool) -> crate::faces::FrameFace {
    let ns = &face.ns;
    let mut values: Vec<(String, f64)> = block
        .params
        .iter()
        .map(|(n, v)| (format!("{ns}/{n}"), *v))
        .collect();
    if let Some(db) = block.output_level_db {
        values.push((format!("{ns}/level"), db));
    }
    values.push((format!("{ns}/on"), if block.bypassed { 0.0 } else { 1.0 }));
    if let Some(option) = block.option {
        values.push((format!("{ns}/option"), option));
    }
    for map in &face.maps {
        values.push((
            format!("{ns}/{}", map.face),
            map.to_face(block.param(&map.param, 0.0)),
        ));
    }
    let mut texts = Vec::new();
    if face.time_knob
        && let Some((pos, text)) = time_view(block)
    {
        values.push((format!("{ns}/time_knob"), pos));
        values.push((format!("{ns}/sync"), 0.0));
        texts.push((format!("{ns}/time_text"), text));
    }
    if face.nameplate {
        texts.push((
            format!("{ns}/preset"),
            rig.active
                .get(&block.name)
                .cloned()
                .unwrap_or_else(|| "—".to_owned()),
        ));
    }
    let design = if dark {
        face.dark.clone().unwrap_or_else(|| face.face.clone())
    } else {
        face.face.clone()
    };
    crate::faces::FrameFace {
        design,
        values,
        texts,
    }
}
