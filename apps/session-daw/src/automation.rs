//! Automation lanes: a track's envelopes, each in a lane of its own under
//! the track — REAPER's envelope lanes — drawn as they play and edited in
//! place.
//!
//! A lane is shown when its envelope is **visible** in the engine, so the
//! choice is the song's (saved, and REAPER reads it the same way) rather
//! than the window's. Its height is the envelope's own lane height, or
//! [`LANE_H`].
//!
//! The lanes are read from the engine with the rest of the session (see
//! [`read`]) into one store the layout, the drawing and the hit test all
//! look in, so a lane cannot be drawn where it is not hit or hit where it
//! is not drawn. An edit changes the store at once — the picture follows
//! the pointer — and goes to the engine as an [`crate::engine::Edit`].
//!
//! The curve drawn is the one the engine plays, from the same definition
//! ([`daw_proto::automation::played`]): the points bent by their shapes,
//! automation items over their windows.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use daw_proto::automation::{
    AutomationItem, EnvelopePoint, EnvelopeRef, EnvelopeShape, EnvelopeType,
};
use daw_proto::primitives::{AutomationMode, PositionInSeconds};

/// A lane's height when its envelope has none of its own, in session
/// pixels.
pub const LANE_H: f64 = 64.0;

/// The least a lane can be dragged to.
pub const LANE_MIN_H: f64 = 24.0;

/// The vocal leveler's lane: its bottom and how many dB it spans to its
/// top — ±36: a gate's floor near the bottom, and a ride that carries the
/// makeup (a quiet verse brought up 20 dB and more) under the top.
const LEVELER_FLOOR_DB: f64 = -36.0;
const LEVELER_SPAN_DB: f64 = 72.0;

/// The vocal leveler's lane's name, and the reference it goes by.
pub const LEVELER: &str = "Vocal leveler";

/// One leveled item in the vocal leveler's lane: its four lanes and their
/// sum, in the recording's seconds (`stretch` turns them into the item's).
#[derive(Clone, Debug)]
pub struct Leveled {
    pub item: String,
    /// Where the item starts and how long it is, seconds.
    pub position: f64,
    pub length: f64,
    /// Item seconds per recording second.
    pub stretch: f64,
    pub lanes: level_dsp::envelope::Contributions,
    /// Their sum — what the item's volume plays.
    pub composite: Vec<level_dsp::envelope::EnvPoint>,
}

impl Leveled {
    /// Timeline seconds for a lane's time.
    #[must_use]
    pub fn at(&self, t_s: f64) -> f64 {
        t_s.mul_add(self.stretch, self.position)
    }

    /// A lane's time for timeline seconds.
    #[must_use]
    pub fn lane_time(&self, t: f64) -> f64 {
        (t - self.position) / self.stretch.max(1e-9)
    }

    /// Whether timeline `t` is in the item.
    #[must_use]
    pub fn holds(&self, t: f64) -> bool {
        t >= self.position && t <= self.position + self.length
    }

    /// The sum again, after its lanes changed.
    pub fn resum(&mut self) {
        self.composite = level_dsp::envelope::composite(&self.lanes);
    }
}

/// The dB a leveler curve (`hold` points step) gives at lane time `t`; 0
/// with none.
#[must_use]
pub fn db_at(points: &[level_dsp::envelope::EnvPoint], t: f64) -> f64 {
    let after = points.partition_point(|p| p.t_s <= t);
    match (
        after.checked_sub(1).and_then(|i| points.get(i)),
        points.get(after),
    ) {
        (None, Some(b)) => b.db,
        (Some(a), None) => a.db,
        (Some(a), Some(b)) if a.hold || b.t_s <= a.t_s => a.db,
        (Some(a), Some(b)) => (b.db - a.db).mul_add((t - a.t_s) / (b.t_s - a.t_s), a.db),
        (None, None) => 0.0,
    }
}

/// What a lane automates — which decides how its values are drawn and
/// read out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Linear gain, 1 at 0 dB (the fader trims it).
    Volume,
    /// 0 left, ½ centre, 1 right.
    Pan,
    /// Above ½ muted.
    Mute,
    /// 0 to 1.
    Width,
    /// A plugin parameter, normalized.
    Param,
    /// The vocal leveler's lanes, in dB ([`Leveled`]).
    Leveler,
}

impl Kind {
    /// The kind of a track envelope type.
    #[must_use]
    pub const fn of(ty: EnvelopeType) -> Self {
        match ty {
            EnvelopeType::Volume | EnvelopeType::VolumePrefx => Self::Volume,
            EnvelopeType::Pan | EnvelopeType::PanPrefx => Self::Pan,
            EnvelopeType::Mute => Self::Mute,
            EnvelopeType::Width | EnvelopeType::WidthPrefx => Self::Width,
            _ => Self::Param,
        }
    }

    /// Where `value` sits in the lane, `0` at the bottom and `1` at the
    /// top.
    ///
    /// Volume on a fader's law (`2·y⁴`: 0 dB about five sixths of the way
    /// up, −∞ at the bottom, +6 dB at the top), so the dBs a mix is made
    /// of get the room. Pan with left at the top and mute with sounding at
    /// the top, as REAPER draws them.
    #[must_use]
    pub fn y_of(self, value: f64) -> f64 {
        match self {
            Self::Volume => (value.max(0.0) / 2.0).sqrt().sqrt().min(1.0),
            Self::Pan | Self::Mute => 1.0 - value.clamp(0.0, 1.0),
            Self::Width | Self::Param => value.clamp(0.0, 1.0),
            Self::Leveler => ((value - LEVELER_FLOOR_DB) / LEVELER_SPAN_DB).clamp(0.0, 1.0),
        }
    }

    /// The value at `y` in the lane (the inverse of [`Self::y_of`]).
    #[must_use]
    pub fn value_of(self, y: f64) -> f64 {
        let y = y.clamp(0.0, 1.0);
        match self {
            Self::Volume => 2.0 * y.powi(4),
            Self::Pan => 1.0 - y,
            Self::Mute => {
                if y < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Width | Self::Param => y,
            Self::Leveler => y.mul_add(LEVELER_SPAN_DB, LEVELER_FLOOR_DB),
        }
    }

    /// `value` said the way a mixer reads it.
    #[must_use]
    pub fn text(self, value: f64) -> String {
        match self {
            Self::Volume => {
                if value <= 1e-6 {
                    "-inf dB".to_owned()
                } else {
                    format!("{:+.1} dB", 20.0 * value.log10())
                }
            }
            Self::Pan => {
                let pan = value.mul_add(2.0, -1.0);
                let percent = (pan.abs() * 100.0).round();
                if percent < 0.5 {
                    "C".to_owned()
                } else if pan < 0.0 {
                    format!("{percent:.0}%L")
                } else {
                    format!("{percent:.0}%R")
                }
            }
            Self::Mute => if value > 0.5 { "Muted" } else { "On" }.to_owned(),
            Self::Width | Self::Param => format!("{:.0}%", value * 100.0),
            Self::Leveler => format!("{value:+.1} dB"),
        }
    }

    /// The shape a point drawn in this lane gets: a mute switches, the
    /// rest move.
    #[must_use]
    pub const fn shape(self) -> EnvelopeShape {
        match self {
            Self::Mute => EnvelopeShape::Square,
            _ => EnvelopeShape::Linear,
        }
    }
}

/// One envelope in its lane.
#[derive(Clone, Debug)]
pub struct Lane {
    /// The track it is under.
    pub track: String,
    /// The envelope, as the engine names it.
    pub target: EnvelopeRef,
    pub name: String,
    pub kind: Kind,
    /// Session pixels.
    pub height: f64,
    pub mode: AutomationMode,
    /// Its own points, sorted by time.
    pub points: Vec<EnvelopePoint>,
    /// Its automation items, each with its pool's curve (seconds into the
    /// item's source).
    pub items: Vec<(AutomationItem, Vec<EnvelopePoint>)>,
    /// What plays: the points with the items over them.
    pub played: Vec<EnvelopePoint>,
    /// What it rests at with no points — the parameter's own value, or
    /// the envelope's neutral one.
    pub rest: f64,
    /// The plugin a parameter's lane is on, by name — whose words its
    /// values are said in.
    pub fx_name: Option<String>,
    /// The vocal leveler's lane's items (empty for every other lane).
    pub leveled: Vec<Leveled>,
}

impl Lane {
    /// `value` said in the lane's own terms: dB, a pan, or a plugin
    /// parameter as the plugin writes it.
    #[must_use]
    pub fn say(&self, value: f64) -> String {
        let param = match (&self.fx_name, &self.target) {
            (Some(fx), EnvelopeRef::FxParam { param_index, .. }) => {
                Some((fx.as_str(), *param_index))
            }
            _ => None,
        };
        say(self.kind, param, value)
    }

    /// The value playing at `t` seconds.
    #[must_use]
    pub fn value_at(&self, t: f64) -> f64 {
        if self.kind == Kind::Leveler {
            return self
                .leveled
                .iter()
                .find(|l| l.holds(t))
                .map_or(0.0, |l| db_at(&l.composite, l.lane_time(t)));
        }
        daw_proto::automation::value_at(&self.played, t).unwrap_or(self.rest)
    }

    /// Recompute what plays after the points or items changed.
    pub fn replay(&mut self) {
        self.played = daw_proto::automation::played(&self.points, &self.items);
    }

    /// Which automation item is on top at `t`, if any (the latest whose
    /// window holds it).
    #[must_use]
    pub fn item_at(&self, t: f64) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, (item, _))| {
                let start = item.position.as_seconds();
                t >= start && t <= start + item.length.as_seconds()
            })
            .map(|(i, _)| i)
            .next_back()
    }

    /// The same envelope as `other`.
    #[must_use]
    pub fn is(&self, track: &str, target: &EnvelopeRef) -> bool {
        self.track == track && same_target(&self.target, target)
    }
}

/// `value` of an envelope of `kind` said in its own terms; a built-in
/// plugin's parameter (`(fx, index)`) as the plugin writes it.
#[must_use]
pub fn say(kind: Kind, param: Option<(&str, u32)>, value: f64) -> String {
    #[cfg(feature = "native")]
    if let Some(text) = param.and_then(|(fx, index)| builtin_fx::reading(fx, index, value)) {
        return text;
    }
    #[cfg(not(feature = "native"))]
    let _ = param;
    kind.text(value)
}

/// Whether two references name one envelope.
#[must_use]
pub fn same_target(a: &EnvelopeRef, b: &EnvelopeRef) -> bool {
    a == b
}

/// A short, stable name for an envelope reference — what an edit is
/// coalesced by and an action names a lane with.
#[must_use]
pub fn target_key(target: &EnvelopeRef) -> String {
    match target {
        EnvelopeRef::Type(ty) => format!("{ty:?}"),
        EnvelopeRef::FxParam {
            fx_guid,
            param_index,
        } => format!("fx:{fx_guid}:{param_index}"),
        EnvelopeRef::Send { send_index, kind } => format!("send:{send_index}:{kind:?}"),
        EnvelopeRef::Take {
            item_guid, kind, ..
        } => format!("take:{item_guid}:{kind:?}"),
        EnvelopeRef::ByName(name) => format!("name:{name}"),
    }
}

/// The track ext key that shows a track's items' volume curves on them.
pub const TAKE_LINES_KEY: &str = "FTS:take-lines";

/// An item's take volume curve: `(seconds into the item, linear gain)`
/// points, drawn over the item and dragged there.
#[derive(Clone, Debug, Default)]
pub struct TakeCurve {
    /// The item's track.
    pub track: String,
    pub points: Vec<EnvelopePoint>,
}

/// Where a take volume (linear gain) sits in its item, `0` at the bottom
/// and `1` at the top: −48 dB to +24 dB, so a slice taken down and a
/// verse brought up 20 dB both have room.
#[must_use]
pub fn take_y(gain: f64) -> f64 {
    let db = 20.0 * gain.max(1e-6).log10();
    ((db + 48.0) / 72.0).clamp(0.0, 1.0)
}

/// The gain at `y` in an item (the inverse of [`take_y`]).
#[must_use]
pub fn take_gain(y: f64) -> f64 {
    10f64.powf(y.clamp(0.0, 1.0).mul_add(72.0, -48.0) / 20.0)
}

/// The take volume curves shown, by item guid.
fn take_store() -> &'static Mutex<HashMap<String, TakeCurve>> {
    static TAKES: LazyLock<Mutex<HashMap<String, TakeCurve>>> = LazyLock::new(Default::default);
    &TAKES
}

/// `item`'s take volume curve, if its track shows them and it has one.
#[must_use]
pub fn take_curve(item: &str) -> Option<TakeCurve> {
    take_store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(item)
        .cloned()
}

/// Change `item`'s take volume curve in place — a drag's points, at once.
pub fn set_take_curve(item: &str, points: Vec<EnvelopePoint>) {
    if let Some(curve) = take_store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_mut(item)
    {
        curve.points = points;
    }
}

/// Show `curve` on `item` — what a read puts there, for a caller that
/// already has it.
pub fn put_take_curve(item: &str, curve: TakeCurve) {
    take_store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(item.to_owned(), curve);
}

/// Whether any take curve is shown (so a frame need not look for them).
#[must_use]
pub fn any_take_curves() -> bool {
    !take_store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty()
}

/// The take volume curves of every item of `tracks` whose track shows
/// them ([`TAKE_LINES_KEY`]) and whose active take has one.
#[cfg(feature = "native")]
#[must_use]
pub fn read_takes(
    daw: &daw::standalone::Standalone,
    project: &daw_proto::ProjectContext,
    tracks: &[daw_proto::Track],
) -> HashMap<String, TakeCurve> {
    use daw_proto::automation::{Automation, EnvelopeLocation, TakeEnvelopeKind};
    use daw_proto::{ItemRef, Items as _, Takes as _, TrackRef, Tracks};
    let mut out = HashMap::new();
    for track in tracks {
        let shown = Tracks::ext_state(daw, project.clone(), TrackRef::Guid(track.guid.clone()))
            .iter()
            .any(|e| e.key == TAKE_LINES_KEY && e.value == "1");
        if !shown {
            continue;
        }
        for item in daw.get_items(project.clone(), TrackRef::Guid(track.guid.clone())) {
            let Some(take) = daw.get_active_take(project.clone(), ItemRef::Guid(item.guid.clone()))
            else {
                continue;
            };
            let at = EnvelopeLocation::new(
                TrackRef::Index(0),
                EnvelopeRef::Take {
                    item_guid: item.guid.clone(),
                    take_guid: take.guid,
                    kind: TakeEnvelopeKind::Volume,
                },
            );
            let points = Automation::points(daw, project.clone(), at);
            if !points.is_empty() {
                out.insert(
                    item.guid,
                    TakeCurve {
                        track: track.guid.clone(),
                        points,
                    },
                );
            }
        }
    }
    out
}

/// Every shown lane, by track guid, in the order they stack.
fn store() -> &'static Mutex<HashMap<String, Vec<Lane>>> {
    static LANES: LazyLock<Mutex<HashMap<String, Vec<Lane>>>> = LazyLock::new(Default::default);
    &LANES
}

/// The lanes under `track`, top to bottom.
#[must_use]
pub fn lanes_of(track: &str) -> Vec<Lane> {
    store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(track)
        .cloned()
        .unwrap_or_default()
}

/// How tall `track`'s lanes are together, in session pixels.
#[must_use]
pub fn lanes_height(track: &str) -> f64 {
    store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(track)
        .map_or(0.0, |lanes| lanes.iter().map(|l| l.height).sum())
}

/// Change one lane in place — a drag's points, at once, before the engine
/// has them. `false` when there is no such lane.
pub fn update(track: &str, target: &EnvelopeRef, f: impl FnOnce(&mut Lane)) -> bool {
    let mut store = store().lock().unwrap_or_else(PoisonError::into_inner);
    let Some(lane) = store
        .get_mut(track)
        .and_then(|lanes| lanes.iter_mut().find(|l| same_target(&l.target, target)))
    else {
        return false;
    };
    let height = lane.height;
    f(lane);
    lane.replay();
    let moved = (lane.height - height).abs() > f64::EPSILON;
    drop(store);
    if moved {
        // A lane taller or shorter is every row under it moved.
        crate::folds::touch();
    }
    true
}

/// Put `lanes` in place of those tracks' lanes, every other track's kept
/// (a track given none has none).
pub fn replace(lanes: impl IntoIterator<Item = (String, Vec<Lane>)>) {
    let mut all = store()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for (track, mine) in lanes {
        if mine.is_empty() {
            all.remove(&track);
        } else {
            all.insert(track, mine);
        }
    }
    adopt(all);
}

/// Put a set of lanes in place of the store's, the rows planned again if
/// any track's lanes came or went or changed height.
pub fn adopt(lanes: impl IntoIterator<Item = (String, Vec<Lane>)>) {
    let lanes: HashMap<String, Vec<Lane>> = lanes.into_iter().collect();
    let shape = |all: &HashMap<String, Vec<Lane>>| {
        let mut shape: Vec<(String, Vec<(String, u64)>)> = all
            .iter()
            .map(|(track, lanes)| {
                let heights = lanes
                    .iter()
                    .map(|l| (target_key(&l.target), l.height.to_bits()))
                    .collect();
                (track.clone(), heights)
            })
            .filter(|(_, lanes): &(String, Vec<(String, u64)>)| !lanes.is_empty())
            .collect();
        shape.sort();
        shape
    };
    let mut store = store().lock().unwrap_or_else(PoisonError::into_inner);
    let moved = shape(&store) != shape(&lanes);
    *store = lanes;
    drop(store);
    if moved {
        crate::folds::touch();
    }
}

/// The lanes the engine shows for `tracks` — each track's visible
/// envelopes (not a take's, which ride on their items), their points,
/// items and what plays — read from the engine in this process.
#[cfg(feature = "native")]
#[must_use]
pub fn read(
    daw: &daw::standalone::Standalone,
    project: &daw_proto::ProjectContext,
    tracks: &[daw_proto::Track],
) -> HashMap<String, Vec<Lane>> {
    use daw_proto::TrackRef;
    use daw_proto::automation::{Automation, EnvelopeLocation};
    let mut out = HashMap::new();
    for track in tracks {
        let mut lanes: Vec<Lane> =
            Automation::envelopes(daw, project.clone(), TrackRef::Guid(track.guid.clone()))
                .into_iter()
                .filter(|e| e.visible)
                .filter_map(|e| {
                    let target = match (&e.fx_guid, e.param_index) {
                        (Some(fx_guid), Some(param_index)) => EnvelopeRef::FxParam {
                            fx_guid: fx_guid.clone(),
                            param_index,
                        },
                        _ if e.name.starts_with("Send ") => return None,
                        _ => EnvelopeRef::Type(e.envelope_type),
                    };
                    let at =
                        EnvelopeLocation::new(TrackRef::Guid(track.guid.clone()), target.clone());
                    let points = Automation::points(daw, project.clone(), at.clone());
                    let items = Automation::automation_items(daw, project.clone(), at.clone())
                        .into_iter()
                        .map(|item| {
                            let curve = Automation::automation_item_points(
                                daw,
                                project.clone(),
                                at.clone(),
                                item.index,
                            );
                            (item, curve)
                        })
                        .collect();
                    let kind = Kind::of(e.envelope_type);
                    let (name, rest, fx_name) = match &target {
                        EnvelopeRef::FxParam {
                            fx_guid,
                            param_index,
                        } => param(daw, project, &track.guid, fx_guid, *param_index),
                        _ => (e.name.clone(), neutral(kind), None),
                    };
                    let height = f64::from(e.lane_height);
                    let mut lane = Lane {
                        track: track.guid.clone(),
                        target,
                        name,
                        kind,
                        height: if height >= LANE_MIN_H { height } else { LANE_H },
                        mode: e.automation_mode,
                        points,
                        items,
                        played: Vec::new(),
                        rest,
                        fx_name,
                        leveled: Vec::new(),
                    };
                    lane.replay();
                    Some(lane)
                })
                .collect();
        if crate::vocal_level::lane_shown(daw, project, &track.guid) {
            let leveled = crate::vocal_level::leveled(daw, project, track);
            if !leveled.is_empty() {
                lanes.push(Lane {
                    track: track.guid.clone(),
                    target: EnvelopeRef::ByName(LEVELER.to_owned()),
                    name: LEVELER.to_owned(),
                    kind: Kind::Leveler,
                    height: 2.0 * LANE_H,
                    mode: AutomationMode::Read,
                    points: Vec::new(),
                    items: Vec::new(),
                    played: Vec::new(),
                    rest: 0.0,
                    fx_name: None,
                    leveled,
                });
            }
        }
        if lanes.is_empty() {
            continue;
        }
        lanes.sort_by_key(order);
        out.insert(track.guid.clone(), lanes);
    }
    out
}

/// Read the lanes of `tracks` in song `project` from this process's engine
/// into the store. Nothing without one (a remote window shows no lanes
/// yet).
#[cfg(feature = "native")]
pub fn refresh_in(project: &str, tracks: &[daw_proto::Track]) {
    let Some(daw) = crate::open::local_engine() else {
        return;
    };
    let project = daw_proto::ProjectContext::Project(project.to_owned());
    adopt(read(daw, &project, tracks));
    *take_store().lock().unwrap_or_else(PoisonError::into_inner) =
        read_takes(daw, &project, tracks);
}

/// [`refresh_in`] the song open now.
#[cfg(feature = "native")]
pub fn refresh(tracks: &[daw_proto::Track]) {
    if let Some(song) = crate::open::current_song() {
        refresh_in(&song, tracks);
    }
}

/// Where a lane stacks: volume, pan, width, mute, then the plugins' in
/// chain order.
fn order(lane: &Lane) -> (u8, String) {
    let rank = match lane.kind {
        Kind::Volume => 0,
        Kind::Pan => 1,
        Kind::Width => 2,
        Kind::Mute => 3,
        Kind::Param => 4,
        Kind::Leveler => 5,
    };
    (rank, lane.name.clone())
}

/// What a lane with no points rests at: 0 dB, the middle, sounding, full.
#[must_use]
pub const fn neutral(kind: Kind) -> f64 {
    match kind {
        Kind::Volume | Kind::Width => 1.0,
        Kind::Pan | Kind::Param => 0.5,
        Kind::Mute | Kind::Leveler => 0.0,
    }
}

/// A plugin parameter's lane name (`EQ: Band 3 Gain`), its value now, and
/// its plugin's name.
#[cfg(feature = "native")]
fn param(
    daw: &daw::standalone::Standalone,
    project: &daw_proto::ProjectContext,
    track: &str,
    fx_guid: &str,
    index: u32,
) -> (String, f64, Option<String>) {
    use daw_proto::fx::{Effects, FxChainContext, FxRef, FxTarget};
    let chain = FxChainContext::Track(track.to_owned());
    let fx = Effects::list(daw, project.clone(), chain.clone())
        .into_iter()
        .find(|f| f.guid == fx_guid);
    let target = FxTarget::new(chain, FxRef::Guid(fx_guid.to_owned()));
    let found = Effects::parameters(daw, project.clone(), target)
        .into_iter()
        .find(|p| p.index == index);
    let fx_name = fx.map_or_else(|| "FX".to_owned(), |f| f.name);
    let short = fx_name.strip_prefix("FTS ").unwrap_or(&fx_name).to_owned();
    match found {
        Some(p) => (format!("{short}: {}", p.name), p.value, Some(fx_name)),
        None => (format!("{short}: #{index}"), 0.5, Some(fx_name)),
    }
}

/// `points` with the one nearest `at` (within `reach` seconds) moved to
/// `(t, value)`, or a new one there — its index in the result, the list
/// still sorted.
#[must_use]
pub fn with_point(
    points: &[EnvelopePoint],
    moving: Option<usize>,
    t: f64,
    value: f64,
    shape: EnvelopeShape,
) -> (Vec<EnvelopePoint>, usize) {
    let mut out: Vec<EnvelopePoint> = points.to_vec();
    let point = EnvelopePoint {
        index: 0,
        time: PositionInSeconds::from_seconds(t.max(0.0)),
        value,
        shape,
        tension: 0.0,
        selected: true,
    };
    let kept = moving.and_then(|i| out.get(i).cloned());
    if let Some(i) = moving.filter(|&i| i < out.len()) {
        out.remove(i);
    }
    let point = kept.map_or_else(
        || point.clone(),
        |k| EnvelopePoint {
            shape: k.shape,
            tension: k.tension,
            ..point
        },
    );
    let at = out.partition_point(|p| p.time.as_seconds() <= point.time.as_seconds());
    out.insert(at, point);
    for (i, p) in out.iter_mut().enumerate() {
        p.index = u32::try_from(i).unwrap_or(u32::MAX);
    }
    (out, at)
}

/// `points` without the one at `index`.
#[must_use]
pub fn without_point(points: &[EnvelopePoint], index: usize) -> Vec<EnvelopePoint> {
    points
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, p)| p.clone())
        .enumerate()
        .map(|(i, p)| EnvelopePoint {
            index: u32::try_from(i).unwrap_or(u32::MAX),
            ..p
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_reads_back_from_where_it_is_drawn() {
        for value in [0.0, 0.001, 0.25, 1.0, 2.0] {
            let y = Kind::Volume.y_of(value);
            assert!((Kind::Volume.value_of(y) - value).abs() < 1e-9, "{value}");
        }
        // 0 dB well up the lane, and the top +6 dB.
        assert!(Kind::Volume.y_of(1.0) > 0.8);
        assert_eq!(Kind::Volume.text(2.0), "+6.0 dB");
        assert_eq!(Kind::Pan.text(0.25), "50%L");
        assert_eq!(Kind::Pan.text(0.5), "C");
    }

    #[test]
    fn a_point_moved_keeps_the_points_in_order() {
        let p = |t: f64| EnvelopePoint {
            time: PositionInSeconds::from_seconds(t),
            value: 0.5,
            ..EnvelopePoint::default()
        };
        let points = [p(1.0), p(2.0), p(3.0)];
        let (moved, at) = with_point(&points, Some(0), 2.5, 0.9, EnvelopeShape::Linear);
        let times: Vec<f64> = moved.iter().map(|p| p.time.as_seconds()).collect();
        assert_eq!(times, [2.0, 2.5, 3.0]);
        assert_eq!(at, 1);
        let (added, at) = with_point(&points, None, 0.5, 0.1, EnvelopeShape::Square);
        assert_eq!(added.len(), 4);
        assert_eq!(at, 0);
        assert_eq!(without_point(&added, 0).len(), 3);
    }
}
