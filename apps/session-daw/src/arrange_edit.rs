//! Editing in the arrangement, as a state machine the window drives.
//!
//! The window gets winit events; this gets what they meant — a hit,
//! a time under the pointer, the keys held, a bound action — and
//! answers with what changed here and what has to happen elsewhere
//! ([`Effect`]: an edit for the engine, a re-record, a transport
//! move). Nothing in here knows a window, which is what makes a
//! gesture testable: a test builds a project, records it, presses on
//! an item, moves, releases, and reads the effects — the same code the
//! window runs, without a display.
//!
//! The window's copy of the project is edited here too (`&mut
//! Project`), so a prediction and the edit it predicts are one place.

use std::collections::HashSet;

use daw_proto::Track;
use daw_ui::studio::project::Project;

use crate::arrangement::{Arrangement, Fades, ItemZone, Viewport};
use crate::cursor;
use crate::engine::{Edit, Move};
use crate::hit::{Hit, Target};
use crate::keys::Action;
use crate::mousemap::{self, Gesture, Mods};

/// The zoom tool's drag: the expression editor's gesture, on the
/// arrangement.
///
/// Hold `z`, press, and drag — sideways zooms time, up zooms the
/// rows, both exponential and both anchored so what was under the
/// press stays under it. Alt sweeps a box to frame instead.
///
/// The gains are the roll's: two hundred pixels of travel is one
/// e-fold, eight hundred with Shift for the fine control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomTool {
    /// Where the press was, in window pixels.
    origin: (f64, f64),
    current: (f64, f64),
    /// The second under the press, and the content row under it at a
    /// vertical zoom of one.
    anchor_t: f64,
    anchor_y: f64,
    base_pps: f64,
    base_zoom_y: f64,
    /// Alt: a sweep to frame rather than a continuous zoom.
    marquee: bool,
}

/// Where the lanes start in the window: the rails' corner plus the
/// panel's width and the ruler's height.
pub type LanesOrigin = (f64, f64);

/// The least and most the arrangement zooms to, in pixels a second
/// and in row scale.
pub const PPS_RANGE: (f64, f64) = (2.0, 2000.0);
pub const ZOOM_Y_RANGE: (f64, f64) = (0.1, 8.0);

/// What an interaction asks the window to do beyond the picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// To the engine.
    Send(Edit),
    /// The recording is stale: what it holds changed.
    ReRecord,
    /// The transport, with a time where one is meant.
    Transport(Move, f64),
    /// The window's playhead, told where the transport went.
    Playhead(f64),
}

/// An item taken hold of by its body or an edge.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemPress {
    pub index: usize,
    pub guid: String,
    pub zone: ItemZone,
    /// The item's span when it was taken, in seconds.
    pub x0: f64,
    pub x1: f64,
    /// The time under the pointer when it was taken.
    pub from: f64,
    /// Where it would land, once the press has become a drag.
    pub ghost: Option<(f64, f64)>,
}

/// A fade taken hold of by its handle.
#[derive(Clone, Debug, PartialEq)]
pub struct FadeDrag {
    pub index: usize,
    pub guid: String,
    pub zone: ItemZone,
    pub x0: f64,
    pub x1: f64,
    pub fades: Fades,
}

/// The arrangement's editing state.
#[derive(Debug, Default)]
pub struct Editor {
    /// The zoom tool's drag, while `z` is held and the button is down.
    pub zoom: Option<ZoomTool>,
    /// The selected items, by guid.
    pub selected: HashSet<String>,
    /// The edit cursor and the time selection.
    pub cursor: cursor::Edit,
    /// A time selection being dragged out, from this time.
    time_drag: Option<f64>,
    item_press: Option<ItemPress>,
    fade_drag: Option<FadeDrag>,
}

impl Editor {
    /// The pointer went down on `hit`, with `keys` held. `true` if the
    /// press was taken here.
    pub fn press(&mut self, hit: Option<Hit>, keys: Mods, scene: &Arrangement, effects: &mut Vec<Effect>) -> bool {
        let Some(hit) = hit else { return false };
        match hit.target {
            Target::Item { index, zone, seconds, .. } => {
                let action = mousemap::resolve(hit.context, Gesture::Drag, keys);
                let Some(item) = scene.item(index) else { return false };
                match action {
                    mousemap::Action::FadeIn | mousemap::Action::FadeShape => {
                        self.fade_drag = Some(FadeDrag {
                            index,
                            guid: item.guid.clone(),
                            zone,
                            x0: item.x0,
                            x1: item.x1,
                            fades: item.fades,
                        });
                        true
                    }
                    mousemap::Action::MoveItem | mousemap::Action::TrimLeft | mousemap::Action::TrimRight => {
                        self.item_press = Some(ItemPress {
                            index,
                            guid: item.guid.clone(),
                            zone,
                            x0: item.x0,
                            x1: item.x1,
                            from: seconds,
                            ghost: None,
                        });
                        true
                    }
                    _ => false,
                }
            }
            Target::Lane { seconds, .. } | Target::Ruler { seconds } => {
                if mousemap::resolve(hit.context, Gesture::Click, keys) != mousemap::Action::SetEditCursor {
                    return false;
                }
                // A press moves the cursor at once — the click is what
                // you meant — and the transport goes there, because the
                // edit cursor and the play position are one thing until
                // a time selection separates them.
                self.time_drag = Some(seconds);
                self.cursor.click(seconds);
                effects.push(Effect::Transport(Move::Seek, seconds));
                true
            }
            _ => false,
        }
    }

    /// The pointer moved to the time `at` (if it is over the timeline).
    /// `true` when the picture changed.
    pub fn moved(&mut self, at: Option<f64>, pps: f64, bpm: f64, keys: Mods) -> bool {
        let Some(at) = at else { return false };
        if let Some(press) = self.item_press.as_mut() {
            let dx = at - press.from;
            let moved = press.ghost.is_some() || (dx * pps).abs() > crate::gesture::SLOP;
            if !moved {
                return false;
            }
            let beat = 60.0 / bpm.max(1.0);
            let snap = |t: f64| if keys.shift { t } else { (t / beat).round() * beat };
            press.ghost = Some(match press.zone {
                ItemZone::LeftEdge => (snap(press.x0 + dx).clamp(0.0, press.x1 - 0.01), press.x1),
                ItemZone::RightEdge => (press.x0, snap(press.x1 + dx).max(press.x0 + 0.01)),
                _ => {
                    let x0 = snap(press.x0 + dx).max(0.0);
                    (x0, x0 + (press.x1 - press.x0))
                }
            });
            return true;
        }
        if let Some(drag) = self.fade_drag.as_mut() {
            let span = (drag.x1 - drag.x0).max(0.0);
            match drag.zone {
                ItemZone::FadeIn => drag.fades.fade_in = (at - drag.x0).clamp(0.0, span),
                ItemZone::FadeOut => drag.fades.fade_out = (drag.x1 - at).clamp(0.0, span),
                _ => {}
            }
            return true;
        }
        if let Some(from) = self.time_drag {
            self.cursor.drag(from, at);
            return true;
        }
        false
    }

    /// The pointer came up at the time `at`.
    pub fn release(&mut self, at: Option<f64>, keys: Mods, project: &mut Project, effects: &mut Vec<Effect>) -> bool {
        if let Some(press) = self.item_press.take() {
            match press.ghost {
                None => self.select(&press.guid, !keys.ctrl, project, effects),
                Some((x0, x1)) => {
                    let edit = match press.zone {
                        ItemZone::LeftEdge | ItemZone::RightEdge => Edit::TrimItem(press.guid.clone(), x0, x1 - x0),
                        _ => Edit::MoveItem(press.guid.clone(), x0),
                    };
                    edit_item(project, &press.guid, |item| {
                        item.position = daw_proto::primitives::PositionInSeconds::from_seconds(x0);
                        item.length = daw_proto::primitives::Duration::from_seconds(x1 - x0);
                    });
                    effects.push(Effect::Send(edit));
                    effects.push(Effect::ReRecord);
                }
            }
            return true;
        }
        if let Some(drag) = self.fade_drag.take() {
            let edit = match drag.zone {
                ItemZone::FadeIn => Edit::SetFadeIn(drag.guid.clone(), drag.fades.fade_in, drag.fades.in_shape),
                ItemZone::FadeOut => Edit::SetFadeOut(drag.guid.clone(), drag.fades.fade_out, drag.fades.out_shape),
                _ => return true,
            };
            edit_item(project, &drag.guid, |item| {
                item.fade_in_length = daw_proto::primitives::Duration::from_seconds(drag.fades.fade_in);
                item.fade_out_length = daw_proto::primitives::Duration::from_seconds(drag.fades.fade_out);
            });
            effects.push(Effect::Send(edit));
            effects.push(Effect::ReRecord);
            return true;
        }
        if let Some(from) = self.time_drag.take() {
            // A drag across the timeline is a time selection; a click
            // is just the cursor. `Edit::drag` decides which, so a
            // twitch does not leave a four-millisecond selection.
            if let Some(to) = at {
                self.cursor.drag(from, to);
            }
            return true;
        }
        false
    }

    /// The zoom tool takes hold at a window point. `true` when it did
    /// — it only takes a press on the lanes.
    pub fn zoom_press(&mut self, at: (f64, f64), view: &Viewport, lanes: LanesOrigin, keys: Mods) -> bool {
        let (x, y) = at;
        if x < lanes.0 || y < lanes.1 {
            return false;
        }
        let zoom_y = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        self.zoom = Some(ZoomTool {
            origin: at,
            current: at,
            anchor_t: (x - lanes.0 + view.scroll_x) / view.pps.max(f64::EPSILON),
            anchor_y: (y - lanes.1 + view.scroll_y) / zoom_y,
            base_pps: view.pps,
            base_zoom_y: zoom_y,
            marquee: keys.alt,
        });
        true
    }

    /// The zoom tool's drag: the view the arrangement should now show,
    /// or `None` when no zoom is in flight or a sweep is still being
    /// drawn. The scroll comes back unclamped; the window clamps it to
    /// its spans as it does every scroll.
    pub fn zoom_move(&mut self, at: (f64, f64), view: &Viewport, lanes: LanesOrigin, keys: Mods) -> Option<Viewport> {
        let z = self.zoom.as_mut()?;
        z.current = at;
        if z.marquee {
            return None;
        }
        let gain = if keys.shift { 800.0 } else { 200.0 };
        let dx = at.0 - z.origin.0;
        let dy = z.origin.1 - at.1;
        // Right and up zoom in, which is the direction the content
        // grows in both cases.
        let pps = (z.base_pps * (dx / gain).exp()).clamp(PPS_RANGE.0, PPS_RANGE.1);
        let zoom_y = (z.base_zoom_y * (dy / gain).exp()).clamp(ZOOM_Y_RANGE.0, ZOOM_Y_RANGE.1);
        // Put what was under the press back under it, on both axes.
        let scroll_x = z.anchor_t.mul_add(pps, lanes.0 - z.origin.0);
        let scroll_y = z.anchor_y.mul_add(zoom_y, lanes.1 - z.origin.1);
        Some(Viewport {
            scroll_x,
            scroll_y,
            pps,
            zoom_y,
            width: view.width,
            height: view.height,
        })
    }

    /// The zoom tool lets go. An Alt sweep frames its box now; the
    /// continuous drag already zoomed on the way. Returns the view to
    /// show, if the release changes it.
    pub fn zoom_release(&mut self, view: &Viewport, lanes: LanesOrigin) -> Option<Viewport> {
        let z = self.zoom.take()?;
        if !z.marquee {
            return None;
        }
        let moved = (z.current.0 - z.origin.0).abs() + (z.current.1 - z.origin.1).abs();
        // A sweep that never moved is a click, and framing a click
        // would zoom to the maximum for what looked like a misclick.
        if moved <= 3.0 {
            return None;
        }
        let zoom_y = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        let (x0, x1) = (z.origin.0.min(z.current.0), z.origin.0.max(z.current.0));
        let (y0, y1) = (z.origin.1.min(z.current.1), z.origin.1.max(z.current.1));
        let t0 = (x0 - lanes.0 + view.scroll_x) / view.pps.max(f64::EPSILON);
        let t1 = (x1 - lanes.0 + view.scroll_x) / view.pps.max(f64::EPSILON);
        let r0 = (y0 - lanes.1 + view.scroll_y) / zoom_y;
        let r1 = (y1 - lanes.1 + view.scroll_y) / zoom_y;
        let lanes_w = (view.width - (lanes.0 - crate::rails::SIDE)).max(1.0);
        let lanes_h = (view.height - (lanes.1 - crate::rails::TOP)).max(1.0);
        let pps = (lanes_w / (t1 - t0).max(1e-6)).clamp(PPS_RANGE.0, PPS_RANGE.1);
        let zoom = (lanes_h / (r1 - r0).max(1e-6)).clamp(ZOOM_Y_RANGE.0, ZOOM_Y_RANGE.1);
        Some(Viewport {
            scroll_x: t0 * pps,
            scroll_y: r0 * zoom,
            pps,
            zoom_y: zoom,
            width: view.width,
            height: view.height,
        })
    }

    /// The Alt sweep's box, in window pixels, while one is being drawn.
    #[must_use]
    pub fn zoom_marquee(&self) -> Option<((f64, f64), (f64, f64))> {
        self.zoom.filter(|z| z.marquee).map(|z| (z.origin, z.current))
    }

    /// Whether a drag is in flight — what the edge autoscroll asks.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.item_press.as_ref().is_some_and(|p| p.ghost.is_some())
            || self.fade_drag.is_some()
            || self.time_drag.is_some()
            || self.zoom.is_some()
    }

    /// The ghost of an item being moved or trimmed: its index and span.
    #[must_use]
    pub fn ghost(&self) -> Option<(usize, f64, f64)> {
        self.item_press.as_ref().and_then(|p| p.ghost.map(|(x0, x1)| (p.index, x0, x1)))
    }

    /// The fade in flight: its item and where the fades are now.
    #[must_use]
    pub fn fade_in_flight(&self) -> Option<(usize, Fades)> {
        self.fade_drag.as_ref().map(|d| (d.index, d.fades))
    }

    /// Select an item — alone, or added — here and in the engine.
    pub fn select(&mut self, guid: &str, exclusive: bool, project: &mut Project, effects: &mut Vec<Effect>) {
        if exclusive {
            self.selected.clear();
        }
        self.selected.insert(guid.to_owned());
        let selected = &self.selected;
        for item in project.items.values_mut().flatten() {
            item.selected = selected.contains(&item.guid);
        }
        effects.push(Effect::Send(Edit::SelectItem(guid.to_owned(), exclusive)));
    }

    /// A bound key. `false` when this cannot do it, so the key falls
    /// through to what the window binds itself.
    #[expect(clippy::too_many_arguments, reason = "everything a key can touch, passed rather than owned")]
    pub fn key(
        &mut self,
        action: Action,
        project: &mut Project,
        scene: &Arrangement,
        rows: &[(Track, u32)],
        tracks: &mut [Track],
        row_to_track: &crate::plan::Rows,
        bpm: f64,
        effects: &mut Vec<Effect>,
    ) -> bool {
        match action {
            Action::PlayStop | Action::PlayPause => effects.push(Effect::Transport(Move::PlayStop, 0.0)),
            Action::GoToStart => {
                effects.push(Effect::Transport(Move::Home, 0.0));
                effects.push(Effect::Playhead(0.0));
                self.cursor.click(0.0);
            }
            Action::SplitAtCursor => self.split_at(self.cursor.at, project, effects),
            Action::DeleteSelectedItems => {
                let doomed: Vec<String> = self.selected.drain().collect();
                if doomed.is_empty() {
                    return true;
                }
                for lane in project.items.values_mut() {
                    lane.retain(|i| !doomed.contains(&i.guid));
                }
                project.item_count = project.items.values().map(Vec::len).sum();
                for guid in doomed {
                    effects.push(Effect::Send(Edit::DeleteItem(guid)));
                }
                effects.push(Effect::ReRecord);
            }
            Action::SelectAllItems => {
                self.selected.clear();
                for item in project.items.values_mut().flatten() {
                    item.selected = true;
                    self.selected.insert(item.guid.clone());
                }
                effects.push(Effect::Send(Edit::SelectAllItems(String::new())));
            }
            Action::ClearSelection => {
                self.cursor.selection = None;
                self.selected.clear();
                for item in project.items.values_mut().flatten() {
                    item.selected = false;
                }
                effects.push(Effect::Send(Edit::DeselectAllItems(String::new())));
            }
            Action::CursorBar(by) | Action::CursorBeat(by) => {
                let step = if matches!(action, Action::CursorBar(_)) { 240.0 } else { 60.0 } / bpm.max(1.0);
                // To the grid line in that direction — from a cursor
                // between lines, the next line, not a step past it.
                let at = self.cursor.at / step;
                let to = if by > 0 { (at + 1e-6).floor() + 1.0 } else { (at - 1e-6).ceil() - 1.0 };
                self.cursor.at = (to * step).max(0.0);
            }
            Action::TrackStep { by, extend } => {
                let current = rows.iter().position(|(t, _)| t.selected);
                let next = current.map_or(0, |i| i.saturating_add_signed(isize::try_from(by).unwrap_or(0)));
                let next = next.min(rows.len().saturating_sub(1));
                let Some((track, _)) = rows.get(next) else { return true };
                let guid = track.guid.clone();
                if let Some(index) = row_to_track.index(next) {
                    if !extend {
                        for t in tracks.iter_mut() {
                            t.selected = false;
                        }
                    }
                    if let Some(t) = tracks.get_mut(index) {
                        t.selected = true;
                    }
                }
                effects.push(Effect::Send(if extend { Edit::AddToSelection(guid) } else { Edit::Select(guid) }));
                effects.push(Effect::ReRecord);
            }
            Action::Marker(by) => {
                let at = self.cursor.at;
                let mut times: Vec<f64> = scene.markers().iter().map(|m| m.at).collect();
                times.extend(scene.sections().iter().map(|s| s.start));
                times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let to = if by > 0 {
                    times.into_iter().find(|t| *t > at + 1e-3)
                } else {
                    times.into_iter().rev().find(|t| *t < at - 1e-3)
                };
                if let Some(to) = to {
                    self.cursor.click(to);
                    effects.push(Effect::Transport(Move::Seek, to));
                }
            }
            Action::ToggleRecord => tracing::info!("record is not wired to the transport yet"),
            Action::Unbound(id) => {
                tracing::info!(ui.action = %id, "bound in the profile, not built here yet");
                return false;
            }
        }
        true
    }

    /// Split at a time: the selected items that contain it, or every
    /// item that does when nothing is selected — REAPER's rule for the
    /// split key. Each becomes two here, and the engine is told.
    pub fn split_at(&mut self, at: f64, project: &mut Project, effects: &mut Vec<Effect>) {
        let mut splits = Vec::new();
        for lane in project.items.values_mut() {
            let mut halves = Vec::new();
            for item in lane.iter_mut() {
                let start = item.position.as_seconds();
                let end = start + item.length.as_seconds();
                let mine = self.selected.is_empty() || self.selected.contains(&item.guid);
                if !mine || at <= start + 1e-3 || at >= end - 1e-3 {
                    continue;
                }
                // Named after what it came from and where — stable
                // whatever order the lanes come out of the map in.
                let mut right = item.clone();
                right.guid = format!("{}-split@{at:.3}", item.guid);
                right.position = daw_proto::primitives::PositionInSeconds::from_seconds(at);
                right.length = daw_proto::primitives::Duration::from_seconds(end - at);
                right.fade_in_length = daw_proto::primitives::Duration::from_seconds(0.0);
                item.length = daw_proto::primitives::Duration::from_seconds(at - start);
                item.fade_out_length = daw_proto::primitives::Duration::from_seconds(0.0);
                splits.push(Edit::SplitItem(item.guid.clone(), at, right.guid.clone()));
                halves.push(right);
            }
            lane.extend(halves);
            lane.sort_by(|a, b| {
                a.position
                    .as_seconds()
                    .partial_cmp(&b.position.as_seconds())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        if splits.is_empty() {
            return;
        }
        project.item_count = project.items.values().map(Vec::len).sum();
        for edit in splits {
            effects.push(Effect::Send(edit));
        }
        effects.push(Effect::ReRecord);
    }
}

/// Change one item in the window's copy of the project.
fn edit_item(project: &mut Project, guid: &str, change: impl FnOnce(&mut daw_proto::Item)) {
    if let Some(item) = project.items.values_mut().flatten().find(|i| i.guid == guid) {
        change(item);
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;

    fn view() -> Viewport {
        Viewport {
            scroll_x: 100.0,
            scroll_y: 40.0,
            pps: 40.0,
            zoom_y: 1.0,
            width: 2000.0,
            height: 1000.0,
        }
    }
    const LANES: LanesOrigin = (387.0, 103.0);

    #[test]
    fn a_sideways_drag_zooms_time_about_the_press() {
        let mut ed = Editor::default();
        let v = view();
        let at = (887.0, 400.0);
        assert!(ed.zoom_press(at, &v, LANES, Mods::default()));
        let t_under = (at.0 - LANES.0 + v.scroll_x) / v.pps;
        let next = ed.zoom_move((1087.0, 400.0), &v, LANES, Mods::default()).expect("a zoom");
        assert!((next.pps / v.pps - std::f64::consts::E).abs() < 1e-9, "200px is one e-fold");
        assert!((next.zoom_y - 1.0).abs() < 1e-12, "no vertical travel, no vertical zoom");
        // The second under the press is still under it.
        let t_after = (at.0 - LANES.0 + next.scroll_x) / next.pps;
        assert!((t_after - t_under).abs() < 1e-9);
    }

    #[test]
    fn an_upward_drag_zooms_the_rows() {
        let mut ed = Editor::default();
        let v = view();
        let at = (887.0, 400.0);
        ed.zoom_press(at, &v, LANES, Mods::default());
        let row_under = (at.1 - LANES.1 + v.scroll_y) / v.zoom_y;
        let next = ed.zoom_move((887.0, 200.0), &v, LANES, Mods::default()).expect("a zoom");
        assert!((next.zoom_y - std::f64::consts::E).abs() < 1e-9);
        assert!((next.pps - v.pps).abs() < 1e-12);
        let row_after = (at.1 - LANES.1 + next.scroll_y) / next.zoom_y;
        assert!((row_after - row_under).abs() < 1e-9);
    }

    #[test]
    fn shift_is_the_fine_control_and_the_press_must_be_on_the_lanes() {
        let mut ed = Editor::default();
        let v = view();
        assert!(!ed.zoom_press((10.0, 400.0), &v, LANES, Mods::default()), "the panel is not the lanes");
        ed.zoom_press((887.0, 400.0), &v, LANES, Mods::default());
        let shift = Mods { shift: true, ..Mods::default() };
        let next = ed.zoom_move((1087.0, 400.0), &v, LANES, shift).expect("a zoom");
        assert!((next.pps / v.pps - (0.25f64).exp()).abs() < 1e-9);
        assert!(ed.zoom_release(&v, LANES).is_none(), "a drag zoomed on the way; the release adds nothing");
        assert!(!ed.dragging());
    }

    #[test]
    fn an_alt_sweep_frames_its_box_on_release() {
        let mut ed = Editor::default();
        let v = view();
        let alt = Mods { alt: true, ..Mods::default() };
        ed.zoom_press((587.0, 203.0), &v, LANES, alt);
        assert!(ed.zoom_move((987.0, 403.0), &v, LANES, alt).is_none(), "a sweep moves nothing until release");
        assert_eq!(ed.zoom_marquee(), Some(((587.0, 203.0), (987.0, 403.0))));
        let next = ed.zoom_release(&v, LANES).expect("the box framed");
        // Four hundred pixels of sweep at 40 px/s is ten seconds; the
        // lanes are 2000 - 343 wide, so the frame is ~165 px/s.
        let lanes_w = v.width - crate::arrangement::TCP_WIDTH;
        assert!((next.pps - lanes_w / 10.0).abs() < 1e-6);
        // And the sweep's left edge is the new left edge.
        let t0 = (587.0 - LANES.0 + v.scroll_x) / v.pps;
        assert!((next.scroll_x / next.pps - t0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrangement::{Palette, Viewport, TCP_WIDTH};
    use crate::ruler::RULER_H;
    use daw_proto::primitives::{Duration, PositionInSeconds};
    use daw_ui::studio::{ProjectRef, RowsRef};

    const PPS: f64 = 100.0;

    /// Two tracks, three items: a project small enough to reason about
    /// and big enough to split, move and select across.
    fn project() -> Project {
        let track = |guid: &str, index: u32| Track {
            guid: guid.to_owned(),
            name: guid.to_uppercase(),
            index,
            ..Track::default()
        };
        let item = |guid: &str, track: &str, at: f64, len: f64| daw_proto::Item {
            guid: guid.to_owned(),
            track_guid: track.to_owned(),
            position: PositionInSeconds::from_seconds(at),
            length: Duration::from_seconds(len),
            fade_in_length: Duration::from_seconds(0.5),
            fade_out_length: Duration::from_seconds(1.0),
            ..daw_proto::Item::default()
        };
        let mut items = std::collections::HashMap::new();
        items.insert("kick".to_owned(), vec![item("k1", "kick", 2.0, 4.0), item("k2", "kick", 10.0, 4.0)]);
        items.insert("snare".to_owned(), vec![item("s1", "snare", 4.0, 8.0)]);
        Project {
            tracks: vec![track("kick", 0), track("snare", 1)],
            items,
            bpm: 120.0,
            length_secs: 60.0,
            item_count: 3,
            ..Project::default()
        }
    }

    struct Stage {
        editor: Editor,
        project: Project,
        scene: Arrangement,
        rows: Vec<(Track, u32)>,
    }

    impl Stage {
        fn new() -> Self {
            let project = project();
            let rows: Vec<(Track, u32)> = project.tracks.iter().cloned().map(|t| (t, 0)).collect();
            Self {
                scene: record(&project, &rows),
                editor: Editor::default(),
                project,
                rows,
            }
        }

        fn view(&self) -> Viewport {
            Viewport {
                scroll_x: 0.0,
                scroll_y: 0.0,
                pps: PPS,
                zoom_y: 1.0,
                width: 2000.0,
                height: 1000.0,
            }
        }

        /// A window point over `seconds` on `row`, `dy` into the row.
        fn point(&self, row: usize, seconds: f64, dy: f64) -> (f64, f64) {
            let (top, _) = self.scene.row_box(row).expect("a row");
            (
                crate::rails::SIDE + TCP_WIDTH + seconds * PPS,
                crate::rails::TOP + RULER_H + top + dy,
            )
        }

        fn hit(&self, x: f64, y: f64) -> Hit {
            crate::hit::arrangement(&self.scene, self.view(), 0, x, y)
        }

        fn at(&self, x: f64) -> f64 {
            (x - crate::rails::SIDE - TCP_WIDTH) / PPS
        }

        fn press(&mut self, x: f64, y: f64, keys: Mods) -> (bool, Vec<Effect>) {
            let mut effects = Vec::new();
            let taken = self.editor.press(Some(self.hit(x, y)), keys, &self.scene, &mut effects);
            (taken, effects)
        }

        fn drag_to(&mut self, x: f64, keys: Mods) -> bool {
            self.editor.moved(Some(self.at(x)), PPS, self.project.bpm, keys)
        }

        fn release(&mut self, x: f64, keys: Mods) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.editor.release(Some(self.at(x)), keys, &mut self.project, &mut effects);
            effects
        }

        fn key(&mut self, action: Action) -> (bool, Vec<Effect>) {
            let mut effects = Vec::new();
            let map = crate::plan::Rows::of(&self.rows, &self.project.tracks);
            let mut tracks = self.project.tracks.clone();
            let bpm = self.project.bpm;
            let handled = self.editor.key(
                action,
                &mut self.project,
                &self.scene,
                &self.rows,
                &mut tracks,
                &map,
                bpm,
                &mut effects,
            );
            self.project.tracks = tracks;
            (handled, effects)
        }

        fn item(&self, guid: &str) -> &daw_proto::Item {
            self.project.items.values().flatten().find(|i| i.guid == guid).expect(guid)
        }

        fn rerecord(&mut self) {
            self.scene = record(&self.project, &self.rows);
        }
    }

    fn record(project: &Project, rows: &[(Track, u32)]) -> Arrangement {
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = crate::text::Font::embedded().expect("the embedded font");
        Arrangement::build(
            &palette,
            &font,
            &ProjectRef(std::sync::Arc::new(project.clone())),
            &RowsRef(std::sync::Arc::new(rows.to_vec())),
            crate::layout::Layout::default(),
        )
    }

    fn sends(effects: &[Effect]) -> Vec<&Edit> {
        effects.iter().filter_map(|e| match e {
            Effect::Send(edit) => Some(edit),
            _ => None,
        }).collect()
    }

    /// A click on an item's body selects it, alone; with Ctrl it is
    /// added. The project's own flags follow.
    #[test]
    fn a_click_selects_and_ctrl_adds() {
        let mut s = Stage::new();
        let (x, y) = s.point(0, 4.0, 15.0);
        assert!(s.press(x, y, Mods::default()).0, "the body is taken");
        let effects = s.release(x, Mods::default());
        assert_eq!(sends(&effects), vec![&Edit::SelectItem("k1".into(), true)]);
        assert!(s.item("k1").selected && !s.item("s1").selected);
        let (x2, y2) = s.point(1, 8.0, 15.0);
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        s.press(x2, y2, ctrl);
        s.release(x2, ctrl);
        assert!(s.item("k1").selected && s.item("s1").selected);
        assert_eq!(s.editor.selected.len(), 2);
    }

    /// A drag on the body moves the item, snapped to the beat, with a
    /// ghost on the way and one edit and one re-record at the end.
    #[test]
    fn a_body_drag_moves_the_item_to_the_beat() {
        let mut s = Stage::new();
        let (x, y) = s.point(0, 3.0, 15.0);
        s.press(x, y, Mods::default());
        // 1.3 s to the right: the item at 2.0 lands on 3.5 (a beat is
        // 0.5 s at 120 bpm), not 3.3.
        assert!(s.drag_to(x + 1.3 * PPS, Mods::default()));
        assert_eq!(s.editor.ghost(), Some((0, 3.5, 7.5)));
        assert!(s.editor.dragging());
        let effects = s.release(x + 1.3 * PPS, Mods::default());
        assert_eq!(sends(&effects), vec![&Edit::MoveItem("k1".into(), 3.5)]);
        assert!(effects.contains(&Effect::ReRecord));
        assert!((s.item("k1").position.as_seconds() - 3.5).abs() < 1e-9);
        assert!(!s.editor.dragging());
        // Shift: no snap.
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        s.rerecord();
        let (x, y) = s.point(0, 5.0, 15.0);
        s.press(x, y, shift);
        s.drag_to(x + 0.13 * PPS, shift);
        let effects = s.release(x + 0.13 * PPS, shift);
        let Some(Edit::MoveItem(_, to)) = sends(&effects).first().copied() else { panic!("a move") };
        assert!((to - 3.63).abs() < 1e-6);
    }

    /// The edges trim: the left edge moves the start and keeps the end,
    /// the right edge the other way round.
    #[test]
    fn the_edges_trim() {
        let mut s = Stage::new();
        let (x, y) = s.point(1, 4.0, 15.0);
        let hit = s.hit(x + 1.0, y);
        assert!(matches!(hit.target, Target::Item { zone: ItemZone::LeftEdge, .. }), "{hit:?}");
        s.press(x + 1.0, y, Mods::default());
        s.drag_to(x + 1.0 + 2.0 * PPS, Mods::default());
        let effects = s.release(x + 1.0 + 2.0 * PPS, Mods::default());
        assert_eq!(sends(&effects), vec![&Edit::TrimItem("s1".into(), 6.0, 6.0)]);
        s.rerecord();
        let (x, y) = s.point(1, 12.0, 15.0);
        s.press(x - 1.0, y, Mods::default());
        s.drag_to(x - 1.0 - 1.0 * PPS, Mods::default());
        let effects = s.release(x - 1.0 - 1.0 * PPS, Mods::default());
        assert_eq!(sends(&effects), vec![&Edit::TrimItem("s1".into(), 6.0, 5.0)]);
    }

    /// A fade handle drags the fade's length, clamped to the item.
    #[test]
    fn a_fade_handle_drags_the_fade() {
        let mut s = Stage::new();
        // k1 starts at 2.0 with a 0.5 s fade-in: its handle is at 2.5,
        // in the band along the top.
        let (x, y) = s.point(0, 2.5, 4.0);
        let hit = s.hit(x, y);
        assert!(matches!(hit.target, Target::Item { zone: ItemZone::FadeIn, .. }), "{hit:?}");
        assert!(s.press(x, y, Mods::default()).0);
        s.drag_to(x + 1.0 * PPS, Mods::default());
        assert!((s.editor.fade_in_flight().expect("in flight").1.fade_in - 1.5).abs() < 1e-9);
        // Past the end of the item it stops at the item.
        s.drag_to(x + 20.0 * PPS, Mods::default());
        let effects = s.release(x + 20.0 * PPS, Mods::default());
        let Some(Edit::SetFadeIn(guid, len, _)) = sends(&effects).first().copied() else { panic!("a fade") };
        assert_eq!(guid, "k1");
        assert!((len - 4.0).abs() < 1e-9);
        assert!((s.item("k1").fade_in_length.as_seconds() - 4.0).abs() < 1e-9);
    }

    /// The split key: nothing selected splits every item under the
    /// cursor; a selection splits only itself. The halves meet at the
    /// cursor and the engine hears about each.
    #[test]
    fn split_at_cursor_follows_the_selection() {
        let mut s = Stage::new();
        s.editor.cursor.click(5.0);
        let (handled, effects) = s.key(Action::SplitAtCursor);
        assert!(handled);
        assert_eq!(sends(&effects).len(), 2, "k1 and s1 both contain 5.0");
        assert_eq!(s.project.item_count, 5);
        let k1 = s.item("k1");
        assert!((k1.position.as_seconds() + k1.length.as_seconds() - 5.0).abs() < 1e-9);
        let right = s.item("k1-split@5.000");
        assert!((right.position.as_seconds() - 5.0).abs() < 1e-9);
        assert!((right.length.as_seconds() - 1.0).abs() < 1e-9);
        // With a selection, only it.
        let mut s = Stage::new();
        let mut effects = Vec::new();
        s.editor.select("s1", true, &mut s.project, &mut effects);
        s.editor.cursor.click(5.0);
        let (_, effects) = s.key(Action::SplitAtCursor);
        assert_eq!(sends(&effects).len(), 1);
        assert_eq!(s.item("k1").length.as_seconds(), 4.0);
    }

    /// Delete takes the selection out of the project and tells the
    /// engine per item; select-all and escape are their opposites.
    #[test]
    fn delete_select_all_and_escape() {
        let mut s = Stage::new();
        let (handled, effects) = s.key(Action::SelectAllItems);
        assert!(handled);
        assert_eq!(s.editor.selected.len(), 3);
        assert!(effects.contains(&Effect::Send(Edit::SelectAllItems(String::new()))));
        let (_, effects) = s.key(Action::ClearSelection);
        assert!(s.editor.selected.is_empty());
        assert!(effects.contains(&Effect::Send(Edit::DeselectAllItems(String::new()))));
        let mut effects = Vec::new();
        s.editor.select("k2", true, &mut s.project, &mut effects);
        let (_, effects) = s.key(Action::DeleteSelectedItems);
        assert_eq!(sends(&effects), vec![&Edit::DeleteItem("k2".into())]);
        assert_eq!(s.project.item_count, 2);
        assert!(s.project.items["kick"].iter().all(|i| i.guid != "k2"));
    }

    /// The cursor keys step to grid lines, and a click on the lanes
    /// or the ruler places the cursor and seeks; a drag selects time.
    #[test]
    fn the_cursor_moves_by_the_grid_and_the_pointer() {
        let mut s = Stage::new();
        s.editor.cursor.click(1.3);
        s.key(Action::CursorBar(1));
        assert!((s.editor.cursor.at - 2.0).abs() < 1e-9, "next bar line at 120 bpm");
        s.key(Action::CursorBeat(-1));
        assert!((s.editor.cursor.at - 1.5).abs() < 1e-9);
        s.key(Action::GoToStart);
        assert_eq!(s.editor.cursor.at, 0.0);
        // An empty lane: press places and seeks, a drag selects.
        let (x, y) = s.point(0, 20.0, 15.0);
        let (taken, effects) = s.press(x, y, Mods::default());
        assert!(taken);
        assert_eq!(effects, vec![Effect::Transport(Move::Seek, 20.0)]);
        assert!((s.editor.cursor.at - 20.0).abs() < 1e-9);
        s.drag_to(x + 3.0 * PPS, Mods::default());
        s.release(x + 3.0 * PPS, Mods::default());
        let span = s.editor.cursor.selection.expect("a time selection");
        assert!((span.start - 20.0).abs() < 1e-9 && (span.end - 23.0).abs() < 1e-9);
    }

    /// Track selection steps down the rows and extends with Shift.
    #[test]
    fn track_steps_walk_the_rows() {
        let mut s = Stage::new();
        let (_, effects) = s.key(Action::TrackStep { by: 1, extend: false });
        assert_eq!(sends(&effects), vec![&Edit::Select("kick".into())], "from nothing, the first row");
        s.rows[0].0.selected = true;
        let (_, effects) = s.key(Action::TrackStep { by: 1, extend: true });
        assert_eq!(sends(&effects), vec![&Edit::AddToSelection("snare".into())]);
    }

    /// A key the window cannot do is not handled, so it falls through.
    #[test]
    fn an_unbound_action_falls_through() {
        let mut s = Stage::new();
        let (handled, effects) = s.key(Action::Unbound("40059".into()));
        assert!(!handled && effects.is_empty());
    }
}
