//! The FX view: the selected track's effects as a row of devices.
//!
//! The mixer's racks show a track's processing stacked over its strip, at
//! a strip's width; this lays the same processors out the way Ableton and
//! Bitwig do — side by side, each a device at an editor's size, the row
//! scrolling sideways. Docked under the arrangement or the mixer, or a
//! view of its own (`shell::FxToggle`). See `docs/fx-view.md`.
//!
//! The devices are the rack's own panels, drawn by the rack's painters
//! and edited through its grips — the settings every rack view shares
//! (`tone::Shared`), so an edit here is an edit on the strip. Still to
//! come, from the design: the selected track's lineage as strips on the
//! left, collapsing a device, each device's face as a frame of its own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyrender::Scene;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, MouseEventButton, UiEvent,
};
use dioxus::prelude::*;
use vello::kurbo::Affine;

use crate::tone::{Folded, Grip, Panel, Shared, Which};

/// How wide one device is, laid out.
const DEVICE_W: f64 = 300.0;

/// The gap between two devices.
const GAP: f64 = 6.0;

/// How far the row stands in from the pane's edges.
const INSET: f64 = 8.0;

/// The FX panel: the selected track's devices, filling where it is put —
/// or, with `lanes` above one (the FX view of its own), that many tracks'
/// chains stacked, from the selected one down, the stack scrolling through
/// the rest.
#[component]
pub fn FxPanel(#[props(default = 1)] lanes: usize) -> Element {
    // Over a song, with the racks' settings: a host without them (a page
    // with no song yet) shows an empty row rather than failing.
    let (Some(closeups), Some(session)) = (
        try_use_context::<crate::closeup::Closeups>(),
        try_use_context::<crate::studio::StudioSession>(),
    ) else {
        return rsx! {};
    };
    // The session's rows: every track a rack has settings for, seeded
    // here as the mixer seeds them, so the FX is whole without it.
    let rows = session.rows.as_slice().to_vec();
    closeups.tone.store.borrow_mut().seed(&rows);
    let tracks: Vec<daw_proto::Track> = rows.into_iter().map(|(t, _)| t).collect();
    let lanes = lanes.max(1);
    #[cfg(feature = "native")]
    {
        let widget = use_hook(|| {
            dioxus_native_dom::CustomWidgetAttr::new(DeviceRow::new(
                Rc::clone(&closeups.tone),
                tracks,
                lanes,
            ))
        });
        rsx! {
            object {
                style: "position:absolute; left:0; top:0; width:100%; height:100%; background:#0b0c0e;",
                data: widget,
            }
        }
    }
    #[cfg(all(feature = "web", not(feature = "native")))]
    {
        let widget = use_hook(|| {
            crate::web_host::HostedRef(Rc::new(RefCell::new(DeviceRow::new(
                Rc::clone(&closeups.tone),
                tracks,
                lanes,
            ))))
        });
        rsx! {
            crate::web_host::WidgetCanvas { widget, hidden: false }
        }
    }
    #[cfg(not(any(feature = "native", feature = "web")))]
    {
        let _ = (closeups, tracks, lanes);
        rsx! {}
    }
}

/// How wide a lane's name is, when there is more than one lane.
const HEAD_W: f64 = 140.0;

/// One device on screen: whose, which, where.
#[derive(Clone)]
struct Placed {
    guid: String,
    which: Which,
    panel: Panel,
}

/// Tracks' processors, side by side: one lane, or several stacked.
struct DeviceRow {
    tone: Rc<Shared>,
    /// The session's tracks, kept up with the engine's changes — which is
    /// selected is where the lanes start.
    tracks: Vec<daw_proto::Track>,
    watch: Option<crate::engine::Watch>,
    /// Changes taken off the watch to ask for a frame, applied in it.
    pending: RefCell<Vec<daw_proto::track::TrackEvent>>,
    palette: crate::arrangement::Palette,
    font: crate::text::Font,
    /// How many lanes show at once.
    lanes: usize,
    /// How many tracks past the selected one the first lane is.
    down: usize,
    /// How far each lane is scrolled sideways, by track.
    scroll: std::collections::HashMap<String, f64>,
    /// Each lane where it was last drawn: whose, its top and height.
    shown: RefCell<Vec<(String, f64, f64)>>,
    /// Each device where it was last drawn.
    devices: RefCell<Vec<Placed>>,
    hold: Option<(BlitzPointerId, Grip, Placed, (f64, f64))>,
    hover: Option<(String, Which, Grip)>,
    dirty: Cell<bool>,
}

impl DeviceRow {
    fn new(tone: Rc<Shared>, tracks: Vec<daw_proto::Track>, lanes: usize) -> Self {
        Self {
            tone,
            tracks,
            watch: crate::engine::Watch::start(),
            pending: RefCell::new(Vec::new()),
            palette: crate::skin::palette(&crate::arrangement::Palette::from_theme(
                &daw_ui::theming::Theme::dark(),
            )),
            font: crate::text::Font::embedded().expect("the embedded font"),
            lanes,
            down: 0,
            scroll: std::collections::HashMap::new(),
            shown: RefCell::new(Vec::new()),
            devices: RefCell::new(Vec::new()),
            hold: None,
            hover: None,
            dirty: Cell::new(true),
        }
    }

    /// The tracks the lanes are of: from the selected one (or the first)
    /// down, past `down` of them, as many as there are lanes.
    fn lane_tracks(&self) -> Vec<&daw_proto::Track> {
        let from = self.tracks.iter().position(|t| t.selected).unwrap_or(0);
        self.tracks
            .iter()
            .skip(from + self.down)
            .take(self.lanes)
            .collect()
    }

    fn follow(&mut self) {
        let mut events = std::mem::take(&mut *self.pending.borrow_mut());
        if let Some(watch) = &self.watch {
            events.extend(watch.drain());
        }
        for event in &events {
            crate::engine::apply_event(&mut self.tracks, event);
        }
    }

    /// Whether the engine has said anything since the last frame.
    fn heard(&self) -> bool {
        let Some(watch) = &self.watch else {
            return false;
        };
        let mut pending = self.pending.borrow_mut();
        pending.extend(watch.drain());
        !pending.is_empty()
    }

    fn device_at(&self, x: f64, y: f64) -> Option<(Placed, Grip)> {
        let store = self.tone.store.borrow();
        self.devices.borrow().iter().find_map(|placed| {
            if !placed.panel.contains(x, y) {
                return None;
            }
            let tone = store.get(&placed.guid)?;
            crate::tone::grip_at(&[placed.which], tone, placed.panel, Folded::zoomed(), x, y)
                .map(|grip| (placed.clone(), grip))
        })
    }

    /// The lane under `y`, by its track.
    fn lane_at(&self, y: f64) -> Option<String> {
        self.shown
            .borrow()
            .iter()
            .find(|(_, top, h)| y >= *top && y < top + h)
            .map(|(guid, ..)| guid.clone())
    }

    fn event(&mut self, event: &UiEvent) {
        let at =
            |e: &BlitzPointerEvent| (f64::from(e.coords.client_x), f64::from(e.coords.client_y));
        let changed = match event {
            UiEvent::Wheel(e) => {
                let (dx, dy) = match e.delta {
                    BlitzWheelDelta::Lines(x, y) => (x * 40.0, y * 40.0),
                    BlitzWheelDelta::Pixels(x, y) => (x, y),
                };
                let sideways = dx.abs() > dy.abs() || e.mods.shift() || self.lanes == 1;
                if sideways {
                    // The lane under the pointer, along its chain.
                    let by = if dx.abs() > dy.abs() { dx } else { dy };
                    if let Some(guid) = self.lane_at(f64::from(e.coords.client_y)) {
                        let scroll = self.scroll.entry(guid).or_default();
                        *scroll = (*scroll - by).max(0.0);
                    }
                } else if dy < 0.0 {
                    // Down through the tracks, a lane at a time.
                    self.down += 1;
                } else {
                    self.down = self.down.saturating_sub(1);
                }
                true
            }
            UiEvent::PointerDown(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                match self.device_at(x, y) {
                    Some((placed, grip)) if grip.is_switch() => {
                        if let Some(tone) = self.tone.store.borrow_mut().edit(&placed.guid) {
                            crate::tone::toggle(tone, grip);
                        }
                        self.tone.edited();
                        true
                    }
                    Some((placed, grip)) => {
                        self.hold = Some((e.id, grip, placed, (x, y)));
                        true
                    }
                    None => false,
                }
            }
            UiEvent::PointerMove(e) => {
                let (x, y) = at(e);
                if let Some((id, grip, placed, last)) = self.hold.clone()
                    && id == e.id
                {
                    if let Some(tone) = self.tone.store.borrow_mut().edit(&placed.guid) {
                        crate::tone::drag(
                            tone,
                            grip,
                            &[placed.which],
                            placed.panel,
                            Folded::zoomed(),
                            crate::closeup::mods_of(e),
                            x - last.0,
                            y - last.1,
                        );
                    }
                    self.hold = Some((id, grip, placed, (x, y)));
                    self.tone.edited();
                    true
                } else if !e.is_finger() {
                    let over = self
                        .device_at(x, y)
                        .map(|(placed, grip)| (placed.guid, placed.which, grip));
                    let lit = over != self.hover;
                    self.hover = over;
                    lit
                } else {
                    false
                }
            }
            UiEvent::PointerUp(e) | UiEvent::PointerCancel(e) => {
                self.hold.take_if(|(id, ..)| *id == e.id).is_some()
            }
            _ => false,
        };
        if changed {
            self.dirty.set(true);
        }
    }

    fn paint_scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.follow();
        self.dirty.set(false);
        let scale = scale.max(f64::EPSILON);
        let (w, h) = (f64::from(width) / scale, f64::from(height) / scale);
        // Never past the last track: the stack stops at the end.
        let from = self.tracks.iter().position(|t| t.selected).unwrap_or(0);
        let left = self.tracks.len().saturating_sub(from);
        self.down = self.down.min(left.saturating_sub(self.lanes.min(left)));
        let named = self.lanes > 1;
        let lane_h = h / crate::num::coord(self.lanes);
        let start = if named { HEAD_W } else { INSET };
        let mut drawn = Scene::new();
        let mut devices = Vec::new();
        let mut shown = Vec::new();
        let lanes: Vec<daw_proto::Track> = self.lane_tracks().into_iter().cloned().collect();
        for (n, track) in lanes.iter().enumerate() {
            let top = crate::num::coord(n) * lane_h;
            shown.push((track.guid.clone(), top, lane_h));
            if named {
                // The lane's name, in the track's colour, down its left.
                anyrender::PaintScene::fill(
                    &mut drawn,
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    crate::tcp::track_color(&self.palette, track),
                    None,
                    &vello::kurbo::Rect::new(0.0, top + INSET, 3.0, top + lane_h - INSET),
                );
                crate::tcp::glyphs(
                    &mut drawn,
                    &self.font,
                    if track.selected {
                        self.palette.text
                    } else {
                        self.palette.text_dim
                    },
                    &self.font.elide(&track.name, 13.0, HEAD_W - 18.0),
                    10.0,
                    top + INSET + 16.0,
                    13.0,
                );
            }
            let store = self.tone.store.borrow();
            let Some(tone) = store.get(&track.guid) else {
                continue;
            };
            let panels = tone.panels(crate::tone::panels_for(session::mix_phases::MixPhase::Tone));
            let most =
                (crate::num::coord(panels.len()) * (DEVICE_W + GAP) - (w - start) + INSET).max(0.0);
            let scroll = self
                .scroll
                .get(&track.guid)
                .copied()
                .unwrap_or(0.0)
                .min(most);
            self.scroll.insert(track.guid.clone(), scroll);
            for (i, which) in panels.iter().enumerate() {
                let x = start + crate::num::coord(i) * (DEVICE_W + GAP) - scroll;
                // Only what is on screen is drawn.
                if x + DEVICE_W < start || x > w {
                    continue;
                }
                let panel = Panel {
                    x,
                    y: top + INSET,
                    width: DEVICE_W,
                    height: (lane_h - INSET * 2.0).max(0.0),
                };
                let lit = self
                    .hold
                    .as_ref()
                    .filter(|(_, _, held, _)| held.guid == track.guid && held.which == *which)
                    .map(|(_, grip, ..)| *grip)
                    .or(self
                        .hover
                        .as_ref()
                        .filter(|(g, on, _)| *g == track.guid && on == which)
                        .map(|(.., grip)| *grip));
                crate::tone::draw(
                    &mut drawn,
                    &self.palette,
                    &self.font,
                    tone,
                    &crate::live::Meters::default(),
                    &[*which],
                    panel,
                    Folded::zoomed(),
                    lit,
                    None,
                );
                devices.push(Placed {
                    guid: track.guid.clone(),
                    which: *which,
                    panel,
                });
            }
        }
        *self.devices.borrow_mut() = devices;
        *self.shown.borrow_mut() = shown;
        let mut out = Scene::new();
        anyrender::PaintScene::append_scene(&mut out, drawn, Affine::scale(scale));
        out
    }
}

#[cfg(feature = "native")]
impl blitz_dom::node::Widget for DeviceRow {
    fn handle_event(&mut self, event: &UiEvent) {
        self.event(event);
    }

    fn needs_redraw(&self) -> bool {
        self.dirty.get() || self.heard()
    }

    fn paint(
        &mut self,
        _render_ctx: &mut dyn anyrender::RenderContext,
        _styles: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        self.paint_scene(width, height, scale)
    }
}

#[cfg(feature = "web")]
impl crate::web_host::Hosted for DeviceRow {
    fn paint(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.paint_scene(width, height, scale)
    }
    fn event(&mut self, event: &UiEvent) {
        DeviceRow::event(self, event);
    }
}
