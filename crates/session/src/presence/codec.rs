//! Presence on the wire: each kind's Loro value, and the pointer's
//! rate-limited sender.

use loro::LoroValue;

#[allow(clippy::wildcard_imports)]
use super::*;

// ── codecs ──────────────────────────────────────────────────────────────

fn map(entries: Vec<(&str, LoroValue)>) -> LoroValue {
    let m: std::collections::HashMap<String, LoroValue> = entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    LoroValue::from(m)
}

fn opt_f64(v: Option<f64>) -> LoroValue {
    v.map_or(LoroValue::Null, LoroValue::Double)
}

fn strings(v: &[String]) -> LoroValue {
    LoroValue::from(
        v.iter()
            .map(|s| LoroValue::from(s.as_str()))
            .collect::<Vec<_>>(),
    )
}

struct Fields(std::collections::HashMap<String, LoroValue>);

impl Fields {
    fn of(v: &LoroValue) -> Option<Self> {
        match v {
            LoroValue::Map(m) => Some(Self(
                m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            )),
            _ => None,
        }
    }
    fn f64(&self, k: &str) -> Option<f64> {
        match self.0.get(k)? {
            LoroValue::Double(d) => Some(*d),
            LoroValue::I64(i) => i32::try_from(*i).ok().map(f64::from),
            _ => None,
        }
    }
    fn bool(&self, k: &str) -> Option<bool> {
        match self.0.get(k)? {
            LoroValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn string(&self, k: &str) -> Option<String> {
        match self.0.get(k)? {
            LoroValue::String(s) => Some(s.to_string()),
            _ => None,
        }
    }
    fn binary(&self, k: &str) -> Option<Vec<u8>> {
        match self.0.get(k)? {
            LoroValue::Binary(b) => Some(b.to_vec()),
            _ => None,
        }
    }
    fn strings(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(LoroValue::List(l)) => l
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl PeerState {
    #[must_use]
    pub fn encode(&self) -> LoroValue {
        map(vec![
            ("name", LoroValue::from(self.name.as_str())),
            ("color", LoroValue::I64(i64::from(self.color))),
            (
                "song",
                self.song
                    .as_deref()
                    .map_or(LoroValue::Null, LoroValue::from),
            ),
            ("view", LoroValue::from(self.view.as_str())),
            ("edit_cursor", opt_f64(self.edit_cursor)),
            ("sel_start", opt_f64(self.time_selection.map(|s| s.0))),
            ("sel_end", opt_f64(self.time_selection.map(|s| s.1))),
            ("tracks", strings(&self.selected_tracks)),
            ("items", strings(&self.selected_items)),
            (
                "caret_anchor",
                self.chart_caret
                    .as_ref()
                    .map_or(LoroValue::Null, |c| LoroValue::from(c.0.clone())),
            ),
            (
                "caret_head",
                self.chart_caret
                    .as_ref()
                    .map_or(LoroValue::Null, |c| LoroValue::from(c.1.clone())),
            ),
        ])
    }

    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<Self> {
        let f = Fields::of(v)?;
        let color = match f.0.get("color") {
            Some(LoroValue::I64(c)) => u32::try_from(*c).unwrap_or(0),
            _ => 0,
        };
        Some(Self {
            name: f.string("name").unwrap_or_default(),
            color,
            song: f.string("song"),
            view: f.string("view").unwrap_or_default(),
            edit_cursor: f.f64("edit_cursor"),
            time_selection: f.f64("sel_start").zip(f.f64("sel_end")),
            selected_tracks: f.strings("tracks"),
            selected_items: f.strings("items"),
            chart_caret: f.binary("caret_anchor").zip(f.binary("caret_head")),
        })
    }
}

impl Pointer {
    /// Encoded with the sender's clock, `t_ms`, for interpolation.
    #[must_use]
    pub fn encode(&self, t_ms: f64) -> LoroValue {
        match self {
            Self::Timeline { at, track, y } => map(vec![
                ("kind", LoroValue::from("timeline")),
                ("t", LoroValue::Double(t_ms)),
                ("at", LoroValue::Double(*at)),
                ("y", LoroValue::Double(*y)),
                (
                    "track",
                    track.as_deref().map_or(LoroValue::Null, LoroValue::from),
                ),
            ]),
            Self::Chart { x, y } => map(vec![
                ("kind", LoroValue::from("chart")),
                ("t", LoroValue::Double(t_ms)),
                ("x", LoroValue::Double(*x)),
                ("y", LoroValue::Double(*y)),
            ]),
            Self::Region { region, x, y } => map(vec![
                ("kind", LoroValue::from("region")),
                ("t", LoroValue::Double(t_ms)),
                ("region", LoroValue::from(region.as_str())),
                ("x", LoroValue::Double(*x)),
                ("y", LoroValue::Double(*y)),
            ]),
            Self::Anchor { panel, key, u, v } => map(vec![
                ("kind", LoroValue::from("anchor")),
                ("t", LoroValue::Double(t_ms)),
                ("panel", LoroValue::from(panel.as_str())),
                ("key", LoroValue::from(key.as_str())),
                ("u", LoroValue::Double(*u)),
                ("v", LoroValue::Double(*v)),
            ]),
        }
    }

    /// The pointer and the sender's time.
    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<(Self, f64)> {
        let f = Fields::of(v)?;
        let t = f.f64("t")?;
        let pointer = match f.string("kind")?.as_str() {
            "timeline" => Self::Timeline {
                at: f.f64("at")?,
                track: f.string("track"),
                y: f.f64("y").unwrap_or(0.5),
            },
            "chart" => Self::Chart {
                x: f.f64("x")?,
                y: f.f64("y")?,
            },
            "region" => Self::Region {
                region: f.string("region")?,
                x: f.f64("x")?,
                y: f.f64("y")?,
            },
            "anchor" => Self::Anchor {
                panel: f.string("panel")?,
                key: f.string("key")?,
                u: f.f64("u")?,
                v: f.f64("v")?,
            },
            _ => return None,
        };
        Some((pointer, t))
    }
}

impl PlayState {
    #[must_use]
    pub fn encode(&self) -> LoroValue {
        map(vec![
            ("playing", LoroValue::Bool(self.playing)),
            ("position", LoroValue::Double(self.position)),
            ("at", LoroValue::Double(self.at_ms)),
            ("rate", LoroValue::Double(self.rate)),
        ])
    }

    #[must_use]
    pub fn decode(v: &LoroValue) -> Option<Self> {
        let f = Fields::of(v)?;
        Some(Self {
            playing: f.bool("playing")?,
            position: f.f64("position")?,
            at_ms: f.f64("at")?,
            rate: f.f64("rate").unwrap_or(1.0),
        })
    }
}

// ── sending: throttle ───────────────────────────────────────────────────

/// Rate-limits a stream of values to at most one per `interval_ms`.
///
/// Never drops the last one: a pointer that stops moving must end where
/// it stopped, not at the last sample the throttle happened to let through.
#[derive(Debug, Clone)]
pub struct Throttle<T> {
    interval_ms: f64,
    last_sent_ms: Option<f64>,
    pending: Option<T>,
}

impl<T: PartialEq + Clone> Throttle<T> {
    #[must_use]
    pub const fn new(interval_ms: f64) -> Self {
        Self {
            interval_ms,
            last_sent_ms: None,
            pending: None,
        }
    }

    /// Offer a new value at `now_ms`. Returns it if it should go out now;
    /// otherwise it is held for [`Self::flush`].
    pub fn offer(&mut self, value: T, now_ms: f64) -> Option<T> {
        match self.last_sent_ms {
            Some(last) if now_ms - last < self.interval_ms => {
                self.pending = Some(value);
                None
            }
            _ => {
                self.last_sent_ms = Some(now_ms);
                self.pending = None;
                Some(value)
            }
        }
    }

    /// The held value, once its turn has come. Call on a timer.
    pub fn flush(&mut self, now_ms: f64) -> Option<T> {
        let due = self
            .last_sent_ms
            .is_none_or(|last| now_ms - last >= self.interval_ms);
        if due && let Some(value) = self.pending.take() {
            self.last_sent_ms = Some(now_ms);
            return Some(value);
        }
        None
    }
}

impl Roster {
    /// Apply one presence entry that arrived at `local_ms`. `me` is this
    /// peer's id, whose own entries are ignored. `None` removes the part
    /// (the peer left, or timed out).
    pub fn apply(&mut self, me: &str, key: &str, value: Option<&LoroValue>, local_ms: f64) {
        let Some((peer, part)) = split_key(key) else {
            return;
        };
        // Only a peer's own three parts make a peer: session-wide entries
        // (the shared transport, `session/transport`) are not somebody.
        if peer == me || ![STATE, POINTER, PLAY].contains(&part) {
            return;
        }
        let Some(value) = value else {
            if part == STATE {
                self.peers.remove(peer);
            } else if let Some(p) = self.peers.get_mut(peer) {
                match part {
                    POINTER => p.trail = PointerTrail::default(),
                    PLAY => p.play = None,
                    _ => {}
                }
            }
            return;
        };
        let entry = self.peers.entry(peer.to_string()).or_default();
        match part {
            STATE => entry.state = PeerState::decode(value),
            POINTER => {
                if let Some((pointer, sent)) = Pointer::decode(value) {
                    entry.trail.push(pointer, sent, local_ms);
                }
            }
            PLAY => entry.play = PlayState::decode(value),
            _ => {}
        }
    }
}
