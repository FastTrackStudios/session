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

/// The FX panel: the selected track's devices, filling where it is put.
#[component]
pub fn FxPanel() -> Element {
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
    #[cfg(feature = "native")]
    {
        let widget = use_hook(|| {
            dioxus_native_dom::CustomWidgetAttr::new(DeviceRow::new(
                Rc::clone(&closeups.tone),
                tracks,
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
            ))))
        });
        rsx! {
            crate::web_host::WidgetCanvas { widget, hidden: false }
        }
    }
    #[cfg(not(any(feature = "native", feature = "web")))]
    {
        let _ = (closeups, tracks);
        rsx! {}
    }
}

/// The selected track's processors, side by side.
struct DeviceRow {
    tone: Rc<Shared>,
    /// The session's tracks, kept up with the engine's changes — which is
    /// selected is what the row shows.
    tracks: Vec<daw_proto::Track>,
    watch: Option<crate::engine::Watch>,
    /// Changes taken off the watch to ask for a frame, applied in it.
    pending: RefCell<Vec<daw_proto::track::TrackEvent>>,
    palette: crate::arrangement::Palette,
    font: crate::text::Font,
    /// How far the row is scrolled, in its own units.
    scroll: f64,
    /// Each device's box where it was last drawn, and what it is.
    devices: RefCell<Vec<(Which, Panel)>>,
    hold: Option<(BlitzPointerId, Grip, Which, (f64, f64))>,
    hover: Option<(Which, Grip)>,
    dirty: Cell<bool>,
}

impl DeviceRow {
    fn new(tone: Rc<Shared>, tracks: Vec<daw_proto::Track>) -> Self {
        Self {
            tone,
            tracks,
            watch: crate::engine::Watch::start(),
            pending: RefCell::new(Vec::new()),
            palette: crate::skin::palette(&crate::arrangement::Palette::from_theme(
                &daw_ui::theming::Theme::dark(),
            )),
            font: crate::text::Font::embedded().expect("the embedded font"),
            scroll: 0.0,
            devices: RefCell::new(Vec::new()),
            hold: None,
            hover: None,
            dirty: Cell::new(true),
        }
    }

    /// The track the row is of: the selected one, or the first.
    fn track(&self) -> Option<&daw_proto::Track> {
        self.tracks
            .iter()
            .find(|t| t.selected)
            .or_else(|| self.tracks.first())
    }

    /// Take the engine's changes; true if anything did.
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

    fn grip_at(&self, x: f64, y: f64) -> Option<(Which, Grip)> {
        let guid = self.track()?.guid.clone();
        let store = self.tone.store.borrow();
        let tone = store.get(&guid)?;
        self.devices.borrow().iter().find_map(|(which, panel)| {
            panel
                .contains(x, y)
                .then(|| crate::tone::grip_at(&[*which], tone, *panel, Folded::zoomed(), x, y))
                .flatten()
                .map(|grip| (*which, grip))
        })
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
                // Sideways, or a plain wheel turned sideways: the row only
                // goes one way.
                let by = if dx.abs() > dy.abs() { dx } else { dy };
                self.scroll = (self.scroll - by).max(0.0);
                true
            }
            UiEvent::PointerDown(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                match self.grip_at(x, y) {
                    Some((which, grip)) if grip.is_switch() => {
                        if let Some(guid) = self.track().map(|t| t.guid.clone())
                            && let Some(tone) = self.tone.store.borrow_mut().edit(&guid)
                        {
                            crate::tone::toggle(tone, grip);
                        }
                        self.tone.edited();
                        let _ = which;
                        true
                    }
                    Some((which, grip)) => {
                        self.hold = Some((e.id, grip, which, (x, y)));
                        true
                    }
                    None => false,
                }
            }
            UiEvent::PointerMove(e) => {
                let (x, y) = at(e);
                if let Some((id, grip, which, last)) = self.hold
                    && id == e.id
                {
                    self.hold = Some((id, grip, which, (x, y)));
                    let panel = self
                        .devices
                        .borrow()
                        .iter()
                        .find(|(w, _)| *w == which)
                        .map(|(_, p)| *p);
                    if let (Some(panel), Some(guid)) = (panel, self.track().map(|t| t.guid.clone()))
                        && let Some(tone) = self.tone.store.borrow_mut().edit(&guid)
                    {
                        crate::tone::drag(
                            tone,
                            grip,
                            &[which],
                            panel,
                            Folded::zoomed(),
                            crate::closeup::mods_of(e),
                            x - last.0,
                            y - last.1,
                        );
                    }
                    self.tone.edited();
                    true
                } else if !e.is_finger() {
                    let over = self.grip_at(x, y);
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
        let mut drawn = Scene::new();
        let mut devices = Vec::new();
        if let Some(track) = self.track() {
            let store = self.tone.store.borrow();
            if let Some(tone) = store.get(&track.guid) {
                let panels =
                    tone.panels(crate::tone::panels_for(session::mix_phases::MixPhase::Tone));
                // Only what is on screen is drawn: the rest of the row is
                // where it would be, and nothing more.
                let most =
                    (crate::num::coord(panels.len()) * (DEVICE_W + GAP) - w + INSET * 2.0).max(0.0);
                self.scroll = self.scroll.min(most);
                for (i, which) in panels.iter().enumerate() {
                    let x = INSET + crate::num::coord(i) * (DEVICE_W + GAP) - self.scroll;
                    if x + DEVICE_W < 0.0 || x > w {
                        continue;
                    }
                    let panel = Panel {
                        x,
                        y: INSET,
                        width: DEVICE_W,
                        height: (h - INSET * 2.0).max(0.0),
                    };
                    let lit = self
                        .hold
                        .filter(|(_, _, held, _)| held == which)
                        .map(|(_, grip, ..)| grip)
                        .or(self.hover.filter(|(on, _)| on == which).map(|(_, g)| g));
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
                    devices.push((*which, panel));
                }
            }
        }
        *self.devices.borrow_mut() = devices;
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
