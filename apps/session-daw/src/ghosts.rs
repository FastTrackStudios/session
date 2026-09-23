//! Everyone else in the session, drawn faintly over the arrangement.
//!
//! Each remote peer is a ghost in their own colour: their time selection
//! as a wash, their edit cursor as a thin line, their play cursor (when
//! transports are independent) as a faint line with no trail, the items
//! they have selected outlined, and their mouse — an arrow with their
//! name — on the beat and the track it is over. Faint on purpose: this is
//! what other people are doing, and it must never be mistaken for, or
//! draw over, what YOU are doing.
//!
//! The collaboration driver publishes the roster here ([`publish`]); the
//! arrangement paints it every frame ([`paint`]), interpolating pointers
//! and extrapolating play cursors to the moment being drawn.

use std::collections::HashMap;
use std::sync::Mutex;

use session::sync::presence::{PeerState, Pointer, Roster};
use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, Fill};

use crate::arrangement::{Arrangement, Palette, Viewport};
use crate::text::Font;

struct Published {
    roster: Roster,
    /// Local clock → shared clock, milliseconds.
    clock_offset_ms: f64,
    /// Whose play cursors to draw (independent transports only).
    show_play: bool,
}

static ROSTER: Mutex<Option<Published>> = Mutex::new(None);

/// Publish the roster the arrangement should draw. `None` clears it
/// (the session was left).
pub fn publish(roster: Option<Roster>, clock_offset_ms: f64, show_play: bool) {
    if let Ok(mut slot) = ROSTER.lock() {
        *slot = roster.map(|roster| Published {
            roster,
            clock_offset_ms,
            show_play,
        });
    }
}

/// Anyone to draw: the arrangement keeps repainting while there is, so
/// pointers glide and play cursors move.
#[must_use]
pub fn active() -> bool {
    ROSTER
        .lock()
        .ok()
        .is_some_and(|slot| slot.as_ref().is_some_and(|p| !p.roster.peers.is_empty()))
}

/// What this peer is doing, as the arrangement last saw it — read by the
/// collaboration driver, which decides what to tell the others and when.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Local {
    /// The mouse: over the lanes a time and a track, anywhere else a
    /// place in a named panel ([`local_window_pointer`]).
    pub pointer: Option<Pointer>,
    pub selected_items: Vec<String>,
    pub selected_tracks: Vec<String>,
}

static LOCAL: Mutex<Local> = Mutex::new(Local {
    pointer: None,
    selected_items: Vec::new(),
    selected_tracks: Vec::new(),
});

/// Whether the mouse is over the lanes, where the arrangement reports it
/// as a time and a track — finer than any panel position.
static OVER_LANES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The mouse moved over the arrangement: a time and a track over the
/// lanes, `None` off them (the window overlay reports it from there).
pub fn local_pointer(pointer: Option<(f64, Option<String>)>) {
    OVER_LANES.store(pointer.is_some(), std::sync::atomic::Ordering::Relaxed);
    if let Some((at, track)) = pointer
        && let Ok(mut local) = LOCAL.lock()
    {
        local.pointer = Some(Pointer::Timeline { at, track });
    }
}

// ── panels: where each named part of the window is ──────────────────────

/// A panel's rectangle in the window, logical pixels: x, y, width, height.
pub type PanelRect = (f64, f64, f64, f64);

thread_local! {
    static REGION_NODES: std::cell::RefCell<HashMap<String, std::rc::Rc<dioxus::prelude::MountedData>>> =
        std::cell::RefCell::new(HashMap::new());
    static REGION_RECTS: std::cell::RefCell<HashMap<String, PanelRect>> = std::cell::RefCell::new(HashMap::new());
}

/// A panel mounted: remember its node, measured by [`measure_regions`].
/// For `onmounted` on the panel's outermost element.
pub fn region_mounted(id: &str, node: std::rc::Rc<dioxus::prelude::MountedData>) {
    REGION_NODES.with(|n| n.borrow_mut().insert(id.to_owned(), node));
}

/// Re-measure every panel (they move with the window and the layout).
pub async fn measure_regions() {
    let nodes: Vec<(String, std::rc::Rc<dioxus::prelude::MountedData>)> =
        REGION_NODES.with(|n| n.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    for (id, node) in nodes {
        match node.get_client_rect().await {
            Ok(r) if r.size.width > 0.0 && r.size.height > 0.0 => {
                REGION_RECTS.with(|m| {
                    m.borrow_mut().insert(id, (r.origin.x, r.origin.y, r.size.width, r.size.height))
                });
            }
            // Unmounted, or laid out to nothing: not somewhere to point.
            _ => {
                REGION_RECTS.with(|m| m.borrow_mut().remove(&id));
            }
        }
    }
}

/// Where a named panel is in this window.
#[must_use]
pub fn region_rect(id: &str) -> Option<PanelRect> {
    REGION_RECTS.with(|m| m.borrow().get(id).copied())
}

/// The mouse moved in the window (logical pixels): which panel it is
/// over and where in it, or where in the window. Over the lanes the
/// arrangement's own report wins.
pub fn local_window_pointer(x: f64, y: f64, window: (f64, f64)) {
    if OVER_LANES.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let hit = REGION_RECTS.with(|m| {
        m.borrow()
            .iter()
            .filter(|(_, (rx, ry, rw, rh))| x >= *rx && y >= *ry && x < rx + rw && y < ry + rh)
            // The smallest panel under the point is the one it is in.
            .min_by(|a, b| (a.1.2 * a.1.3).total_cmp(&(b.1.2 * b.1.3)))
            .map(|(id, (rx, ry, rw, rh))| Pointer::Region {
                region: id.clone(),
                x: (x - rx) / rw,
                y: (y - ry) / rh,
            })
    });
    let pointer = hit.unwrap_or_else(|| Pointer::Region {
        region: "window".into(),
        x: x / window.0.max(1.0),
        y: y / window.1.max(1.0),
    });
    if let Ok(mut local) = LOCAL.lock() {
        local.pointer = Some(pointer);
    }
}

/// The mouse left the window.
pub fn local_window_left() {
    OVER_LANES.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut local) = LOCAL.lock() {
        local.pointer = None;
    }
}

/// Everyone else's pointer that is not over the lanes (those the
/// arrangement draws), placed in this window: (x, y, name, colour).
#[must_use]
pub fn window_pointers(window: (f64, f64)) -> Vec<(f64, f64, String, u32)> {
    let Ok(slot) = ROSTER.lock() else { return Vec::new() };
    let Some(published) = slot.as_ref() else { return Vec::new() };
    let now = now_ms();
    published
        .roster
        .peers
        .values()
        .filter_map(|peer| {
            let state = peer.state.as_ref()?;
            let Pointer::Region { region, x, y } = peer.trail.at(now)? else { return None };
            let (rx, ry, rw, rh) = if region == "window" {
                (0.0, 0.0, window.0, window.1)
            } else {
                // A panel this window is not showing: nowhere to put it.
                region_rect(&region)?
            };
            Some((rx + x * rw, ry + y * rh, state.name.clone(), state.color))
        })
        .collect()
}

/// The selection, as drawn this frame.
pub fn local_selection(items: &std::collections::HashSet<String>, rows: &[(daw_proto::Track, u32)]) {
    if let Ok(mut local) = LOCAL.lock() {
        let mut items: Vec<String> = items.iter().cloned().collect();
        items.sort();
        let tracks: Vec<String> =
            rows.iter().filter(|(t, _)| t.selected).map(|(t, _)| t.guid.clone()).collect();
        if local.selected_items != items {
            local.selected_items = items;
        }
        if local.selected_tracks != tracks {
            local.selected_tracks = tracks;
        }
    }
}

/// The arrangement's track guids and item guids as last drawn (the
/// collaboration puppet points at real ones).
static SHOWN: Mutex<(Vec<String>, Vec<String>, Vec<String>)> =
    Mutex::new((Vec::new(), Vec::new(), Vec::new()));

pub(crate) fn local_shown(rows: &[(daw_proto::Track, u32)], scene: &Arrangement) {
    if let Ok(mut shown) = SHOWN.lock()
        && shown.0.len() != rows.len()
    {
        shown.0 = rows.iter().map(|(t, _)| t.guid.clone()).collect();
        shown.1 = scene.item_boxes().iter().map(|i| i.guid.clone()).collect();
        shown.2 = rows.iter().map(|(t, _)| t.name.clone()).collect();
    }
}

#[must_use]
pub fn local_rows() -> Vec<String> {
    SHOWN.lock().map(|s| s.0.clone()).unwrap_or_default()
}

/// A shown track's name, by guid.
#[must_use]
pub fn local_track_name(guid: &str) -> Option<String> {
    let shown = SHOWN.lock().ok()?;
    let i = shown.0.iter().position(|g| g == guid)?;
    shown.2.get(i).cloned()
}

#[must_use]
pub fn local_item_guids() -> Vec<String> {
    SHOWN.lock().map(|s| s.1.clone()).unwrap_or_default()
}

/// This peer's presence as the arrangement last saw it.
#[must_use]
pub fn local() -> Local {
    LOCAL.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Milliseconds on this machine's clock, for presence timing.
#[must_use]
pub fn now_ms() -> f64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// How strongly a ghost is drawn, against the local cursor's 1.0.
const FAINT: f32 = 0.55;
const NAME_SIZE: f32 = 11.0;

fn color_of(state: &PeerState) -> Color {
    let [r, g, b] = [
        (state.color >> 16) & 0xff,
        (state.color >> 8) & 0xff,
        state.color & 0xff,
    ]
    .map(|c| u8::try_from(c).unwrap_or(0));
    Color::from_rgba8(r, g, b, 0xff)
}

/// Paint every remote peer. `rows` is the arrangement's row → track list,
/// `lanes_at` the lanes' origin (as the selection overlay takes it), and
/// `height` how far down lines reach.
#[allow(clippy::too_many_arguments)] // the arrangement's own paint context
pub fn paint(
    painter: &mut impl anyrender::PaintScene,
    palette: &Palette,
    font: &Font,
    scene: &Arrangement,
    rows: &[(daw_proto::Track, u32)],
    view: Viewport,
    lanes_at: (f64, f64),
    height: f64,
) {
    let Ok(slot) = ROSTER.lock() else { return };
    let Some(published) = slot.as_ref() else {
        return;
    };
    let local_now = now_ms();
    let shared_now = local_now + published.clock_offset_ms;

    let left = scene.tcp.width();
    let x_of = |seconds: f64| seconds.mul_add(view.pps, left - view.scroll_x);
    let row_of: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (t, _))| (t.guid.as_str(), i))
        .collect();
    let (_, oy) = lanes_at;
    let band = |row: usize| {
        scene
            .row_box(row)
            .map(|(top, h)| (top.mul_add(view.zoom_y, oy), h * view.zoom_y))
    };

    for peer in published.roster.peers.values() {
        let Some(state) = &peer.state else { continue };
        let color = color_of(state);

        // Their time selection, a wash fainter than yours.
        if let Some((start, end)) = state.time_selection {
            let (x0, x1) = (x_of(start).max(left), x_of(end).max(left));
            if x1 > x0 {
                painter.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color.with_alpha(0.10),
                    None,
                    &Rect::new(x0, 0.0, x1, height),
                );
            }
        }
        // Their selected items, outlined in their colour.
        for item in scene
            .item_boxes()
            .iter()
            .filter(|i| state.selected_items.contains(&i.guid))
        {
            let Some((top, h)) = band(item.row) else {
                continue;
            };
            let r = Rect::new(
                item.x0.mul_add(view.pps, lanes_at.0),
                top + 1.0,
                item.x1.mul_add(view.pps, lanes_at.0),
                top + h - 1.0,
            );
            painter.stroke(
                &Stroke::new(1.5),
                Affine::IDENTITY,
                color.with_alpha(FAINT),
                None,
                &r,
            );
        }
        // Their selected tracks: a stripe down the lanes' left edge.
        for guid in &state.selected_tracks {
            let Some((top, h)) = row_of.get(guid.as_str()).and_then(|r| band(*r)) else {
                continue;
            };
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color.with_alpha(FAINT),
                None,
                &Rect::new(left, top, left + 3.0, top + h),
            );
        }
        // Their edit cursor.
        if let Some(at) = state.edit_cursor {
            let x = x_of(at);
            if x >= left {
                painter.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color.with_alpha(FAINT),
                    None,
                    &Rect::new(x, 0.0, x + 1.0, height),
                );
            }
        }
        // Their play cursor, when everyone plays on their own.
        if published.show_play
            && let Some(play) = peer.play
        {
            let x = x_of(play.position_at(shared_now));
            if x >= left {
                painter.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color.with_alpha(if play.playing { 0.45 } else { 0.25 }),
                    None,
                    &Rect::new(x - 0.75, 0.0, x + 0.75, height),
                );
            }
        }
        // Their mouse, with their name.
        if let Some(Pointer::Timeline { at, track }) = peer.trail.at(local_now) {
            let x = x_of(at);
            let y = track
                .as_deref()
                .and_then(|g| row_of.get(g))
                .and_then(|r| band(*r))
                .map_or(8.0, |(top, h)| top + h / 2.0);
            if x >= left {
                paint_pointer(painter, palette, font, &state.name, color, Point::new(x, y));
            }
        }
    }
}

fn paint_pointer(
    painter: &mut impl anyrender::PaintScene,
    palette: &Palette,
    font: &Font,
    name: &str,
    color: Color,
    at: Point,
) {
    // An arrow pointing up-left, its tip on the spot.
    let mut arrow = BezPath::new();
    arrow.move_to(at);
    arrow.line_to((at.x, at.y + 14.0));
    arrow.line_to((at.x + 4.0, at.y + 10.5));
    arrow.line_to((at.x + 9.5, at.y + 11.0));
    arrow.close_path();
    painter.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color.with_alpha(0.9),
        None,
        &arrow,
    );
    painter.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        palette.text.with_alpha(0.6),
        None,
        &arrow,
    );

    if name.is_empty() {
        return;
    }
    let width = font.width(name, NAME_SIZE);
    let tag = Rect::new(at.x + 10.0, at.y + 12.0, at.x + 18.0 + width, at.y + 27.0);
    painter.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color.with_alpha(0.85),
        None,
        &RoundedRect::from_rect(tag, 3.0),
    );
    let mut text = anyrender::Scene::new();
    crate::tcp::glyphs(
        &mut text,
        font,
        Color::from_rgba8(0x10, 0x10, 0x14, 0xff),
        name,
        tag.x0 + 4.0,
        tag.y1 - 4.0,
        NAME_SIZE,
    );
    for command in &text.commands {
        crate::arrangement::submit_command(painter, command, Affine::IDENTITY);
    }
}
