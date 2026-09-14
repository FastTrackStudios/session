//! The expression editor, mounted as a view of this window.
//!
//! The editor's picture is an `anyrender::Scene` and its gestures are
//! functions over an `Editor` — both from `expression-editor-paint`,
//! which has no window of its own. This module is the seam: it gives
//! the editor a box inside the window, turns the window's pointer and
//! keys into the editor's, and replays the editor's scene into the
//! frame the rest of the window is drawing.
//!
//! What it deliberately is not: a copy of the Dioxus surface's
//! component tree. The Dioxus crate rasterizes every frame to a bitmap
//! and hands it to a web view as an image, which is what capped the
//! drum editor at the web view's rate. Here the scene goes straight to
//! Vello on the window's own surface, at whatever rate the GPU allows —
//! the same path the arrangement takes.
//!
//! # Layout
//!
//! The view is a box in window space: the roll on top, with its key
//! gutter and ruler, and the velocity strip under it. The editor's
//! viewport is the roll's *note area* — the box minus the chrome — which
//! is the convention every `interaction` handler assumes, so the
//! pointer is translated into roll space here and nowhere else.

use anyrender::PaintScene;
use expression_editor_core::mouse::{Action, Context, Gesture};
use expression_editor_core::rows::DrumMap;
use expression_editor_core::tools::{self, Hit};
use expression_editor_core::{Edit, Editor, Mode, RowSpace, StripLane, Viewport};
use expression_editor_paint::interaction::{self, Drag};
use expression_editor_paint::paint::{self, Overlay};
use expression_editor_paint::text::Labeller;
use expression_editor_paint::{canvas, demo};
use vello::kurbo::Affine;

use crate::mousemap::Mods;

/// The editor and the box it is drawn in.
pub struct Expression {
    pub editor: Editor,
    /// The gesture in flight over the roll.
    drag: Drag,
    labels: Labeller,
    overlay: Overlay,
    /// The item this edits, when it came from one. `None` for the demo
    /// groove, which is what opens when nothing is selected.
    pub item: Option<String>,
    /// Where the roll was pressed and the note under it, so a release
    /// that never moved can be a click.
    pressed: Option<((f64, f64), Option<expression_editor_core::doc::NoteId>)>,
    /// A value drag over the velocity strip.
    strip_drag: bool,
    /// A middle-drag pan over the strip: the last pointer x.
    strip_pan: Option<f64>,
    /// Top-left of the view, in window pixels.
    origin: (f64, f64),
    /// The view's box.
    size: (f64, f64),
}

/// Which part of the view a point is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Roll,
    Strip,
    Outside,
}

impl Expression {
    /// The demo drum groove, sized to `size` at `origin`.
    #[must_use]
    pub fn demo(origin: (f64, f64), size: (f64, f64)) -> Self {
        let vp = viewport_in(size, demo::default_viewport());
        let editor = demo::editor(demo::Scene::Drums, vp);
        let mut this = Self::hold(editor, None, origin, size);
        this.editor.set_mode(Mode::Drums);
        this
    }

    /// An item's MIDI take, as the editor sees it.
    ///
    /// `drums` puts the notes on the kit's lanes rather than a piano
    /// roll: each note's row becomes the index of the lane whose pitch
    /// it plays. A note the kit has no lane for is dropped rather than
    /// drawn on a row that means something else — the count is logged,
    /// so a kit that does not match the take is a visible problem, not
    /// a silent one.
    #[must_use]
    pub fn from_take(
        snapshot: &daw::service::midi::MidiTakeSnapshot,
        item: String,
        drums: bool,
        origin: (f64, f64),
        size: (f64, f64),
    ) -> Self {
        let vp = viewport_in(size, demo::default_viewport());
        let mut doc = expression_editor_daw::to_doc(snapshot, 48.0);
        if drums {
            let map = kit_for(&doc);
            let before = doc.notes.len();
            doc.notes.retain_mut(|n| match map.row_of_pitch(n.row) {
                Some(row) => {
                    n.row = i32::try_from(row).unwrap_or(0);
                    true
                }
                None => false,
            });
            let dropped = before.saturating_sub(doc.notes.len());
            if dropped > 0 {
                tracing::warn!(
                    expression.item = item,
                    expression.dropped = dropped,
                    "notes with no lane in the kit"
                );
            }
            doc.row_space = RowSpace::Drums(map);
        }
        let mut editor = Editor::new(doc, vp);
        if drums {
            editor.set_mode(Mode::Drums);
        }
        editor.reset_view();
        Self::hold(editor, Some(item), origin, size)
    }

    fn hold(editor: Editor, item: Option<String>, origin: (f64, f64), size: (f64, f64)) -> Self {
        Self {
            editor,
            drag: Drag::None,
            labels: Labeller::new(),
            overlay: Overlay::default(),
            item,
            pressed: None,
            strip_drag: false,
            strip_pan: None,
            origin,
            size,
        }
    }

    /// Give the view its box. Cheap when nothing changed.
    pub fn layout(&mut self, origin: (f64, f64), size: (f64, f64)) {
        self.origin = origin;
        if self.size != size {
            self.size = size;
            let vp = viewport_in(size, self.editor.viewport);
            self.editor.resize(vp);
        }
    }

    /// The roll's height — the box less the strip under it.
    fn roll_h(&self) -> f64 {
        (self.size.1 - self.editor.lane_strip_h).max(canvas::RULER_H + 1.0)
    }

    /// Draw the roll and the strip into the frame.
    pub fn paint(&mut self, painter: &mut impl PaintScene) {
        let (w, h) = self.size;
        let roll_h = self.roll_h();
        let roll = paint::roll_scene(&self.editor, w, roll_h, &self.overlay, &mut self.labels);
        painter.append_scene(roll, Affine::translate(self.origin));
        let strip = paint::strip_scene(&self.editor, w, h - roll_h, &mut self.labels);
        painter.append_scene(
            strip,
            Affine::translate((self.origin.0, self.origin.1 + roll_h)),
        );
    }

    /// A window point in the view's own space.
    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.origin.0, y - self.origin.1)
    }

    fn zone(&self, x: f64, y: f64) -> Zone {
        let (lx, ly) = self.local(x, y);
        if lx < 0.0 || ly < 0.0 || lx >= self.size.0 || ly >= self.size.1 {
            Zone::Outside
        } else if ly < self.roll_h() {
            Zone::Roll
        } else {
            Zone::Strip
        }
    }

    /// A window point in roll space — past the gutter and the ruler.
    fn roll_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        (lx - canvas::GUTTER_W, ly - canvas::RULER_H)
    }

    /// A window point in strip space.
    fn strip_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        (lx, ly - self.roll_h())
    }

    /// Whether a gesture is in flight, so the window keeps sending
    /// moves here even after the pointer leaves the box.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.drag.is_active() || self.strip_drag || self.strip_pan.is_some()
    }

    /// A button press. `button` is 0 left, 1 middle, 2 right. `true`
    /// when the view took it.
    pub fn press(&mut self, x: f64, y: f64, mods: Mods, button: u16) -> bool {
        match self.zone(x, y) {
            Zone::Outside => false,
            Zone::Roll => {
                let (rx, ry) = self.roll_point(x, y);
                let under = match self.editor.hit_test(rx, ry) {
                    Hit::Note { id, .. } | Hit::NoteEdge { id, .. } => Some(id),
                    _ => None,
                };
                self.pressed = (button == 0).then_some(((rx, ry), under));
                let drag = interaction::pointer_down(&mut self.editor, rx, ry, mods_of(mods), button);
                // A right-click asks for a menu this window does not
                // draw yet; it is not a drag.
                self.drag = match drag {
                    Drag::ContextMenu { .. } => Drag::None,
                    other => other,
                };
                true
            }
            Zone::Strip => {
                let (sx, sy) = self.strip_point(x, y);
                if button == 1 {
                    self.strip_pan = Some(sx);
                    return true;
                }
                if !self.editor.strip_lane.is_per_note() {
                    return true;
                }
                self.editor.begin_gesture();
                self.strip_drag = true;
                self.strip_write(sx, sy);
                true
            }
        }
    }

    /// The pointer moved. `true` when something changed.
    pub fn moved(&mut self, x: f64, y: f64, mods: Mods) -> bool {
        if let Some(last) = self.strip_pan {
            let (sx, _) = self.strip_point(x, y);
            // Time only: the strip's vertical is a value, not a scroll.
            self.editor.pan_px(sx - last, 0.0);
            self.strip_pan = Some(sx);
            return true;
        }
        if self.strip_drag {
            let (sx, sy) = self.strip_point(x, y);
            self.strip_write(sx, sy);
            return true;
        }
        if !self.drag.is_active() {
            return false;
        }
        let (rx, ry) = self.roll_point(x, y);
        interaction::pointer_move(&mut self.editor, &mut self.drag, rx, ry, mods_of(mods));
        true
    }

    /// The button came up. `true` when a gesture ended.
    pub fn release(&mut self, x: f64, y: f64, mods: Mods) -> bool {
        if self.strip_pan.take().is_some() {
            return true;
        }
        if std::mem::take(&mut self.strip_drag) {
            return true;
        }
        let (rx, ry) = self.roll_point(x, y);
        let pressed = self.pressed.take();
        let ended = if self.drag.is_active() {
            let drag = std::mem::replace(&mut self.drag, Drag::None);
            self.drag = interaction::pointer_up(&mut self.editor, drag, rx, ry, mods_of(mods));
            true
        } else {
            false
        };
        // A press that never travelled is a click, and the map has its
        // own row for those — in the drum map a drag on a hit is its
        // velocity, and only a click selects it. `pointer_down` opens
        // every gesture as a drag, so the click is resolved here, once
        // the release says which it was.
        if let Some(((px, py), Some(under))) = pressed
            && (rx - px).abs() + (ry - py).abs() <= CLICK_SLOP
        {
            self.click(under, mods);
            return true;
        }
        ended
    }

    /// The map's answer to a click on a note.
    fn click(&mut self, under: expression_editor_core::doc::NoteId, mods: Mods) {
        let ed = &mut self.editor;
        let action = ed
            .mouse
            .resolve_for(Context::Note, Gesture::Click, mods_of(mods), ed.tool);
        match action {
            Action::SelectNote => ed.selection.set_single(under),
            Action::AddNoteToSelection => ed.selection.add(under),
            Action::ToggleNoteSelection => ed.selection.toggle(under),
            _ => {}
        }
    }

    /// The wheel, in notches — one line of a mouse wheel is one.
    pub fn wheel(&mut self, x: f64, y: f64, dx: f64, dy: f64, mods: Mods) -> bool {
        if self.zone(x, y) == Zone::Outside {
            return false;
        }
        let (rx, ry) = self.roll_point(x, y);
        interaction::wheel(&mut self.editor, rx, ry, dx, dy, mods_of(mods));
        true
    }

    /// A key, by its browser-style name (`"Delete"`, `"ArrowLeft"`,
    /// `"a"`). `true` when the editor took it.
    pub fn key(&mut self, key: &str, mods: Mods) -> bool {
        interaction::key_down(&mut self.editor, &self.drag, key, mods_of(mods))
    }

    /// Set the velocity of the notes under a strip point to its height.
    ///
    /// A generous grab around the onset: a stem is a few pixels wide and
    /// this is a value edit, not a precision selection.
    fn strip_write(&mut self, at_x: f64, at_y: f64) {
        let strip_h = (self.size.1 - self.roll_h()).max(1.0);
        let velocity = (1.0 - at_y / strip_h).clamp(0.0, 1.0);
        let roll_x = at_x - canvas::GUTTER_W;
        let time = self.editor.camera.t_at(roll_x);
        let hit: Vec<_> = self
            .editor
            .doc
            .notes
            .iter()
            .filter(|note| {
                let off = (self.editor.camera.x(note.start) - roll_x).abs();
                off <= 8.0 || (note.start <= time && note.end > time && off <= 40.0)
            })
            .map(|note| note.id)
            .collect();
        if hit.is_empty() {
            return;
        }
        let edit = match self.editor.strip_lane {
            StripLane::OffVelocity => Edit::SetOffVelocity {
                notes: hit,
                velocity,
            },
            _ => Edit::SetVelocity {
                notes: hit,
                velocity,
            },
        };
        self.editor.apply(&edit);
    }
}

/// An item's active take, read through the facade.
///
/// Blocks the caller for the round trip — a few milliseconds against
/// the in-process backend, and it happens once, on the key that opens
/// the editor. `None` when there is no facade, no such item, or no
/// notes in it: an audio item has nothing for this editor, and an empty
/// roll would look like a broken load rather than an empty take.
///
/// The take's length is derived from the item's, since the facade
/// reports notes in ticks and the item in seconds.
#[must_use]
pub fn load_take(
    item_guid: &str,
    bpm: f64,
    length_seconds: f64,
) -> Option<daw::service::midi::MidiTakeSnapshot> {
    const PPQ: f64 = 960.0;
    let runtime = crate::open::runtime()?;
    runtime.block_on(async {
        let daw = daw::rpc::Daw::try_get()?;
        let project = daw.current_project().await.ok()?;
        let item = project.items().by_guid(item_guid).await.ok()??;
        let midi = item.active_take().midi();
        let notes = midi.notes().await.ok()?;
        if notes.is_empty() {
            return None;
        }
        let ccs = midi.ccs(None).await.unwrap_or_default();
        let played = notes
            .iter()
            .map(|n| n.start_ppq + n.length_ppq)
            .fold(0.0_f64, f64::max);
        Some(daw::service::midi::MidiTakeSnapshot {
            notes,
            ccs,
            pitch_bends: Vec::new(),
            channel_pressures: Vec::new(),
            poly_pressures: Vec::new(),
            note_expressions: Vec::new(),
            ppq: PPQ,
            length_ppq: (length_seconds * bpm / 60.0 * PPQ).max(played),
        })
    })
}

/// How far a press may travel and still be a click, in pixels.
const CLICK_SLOP: f64 = 3.0;

/// The roll's viewport inside a box: the note area, less the gutter,
/// the ruler and the strip under it.
fn viewport_in(size: (f64, f64), _fallback: Viewport) -> Viewport {
    let strip = 96.0;
    Viewport::new(
        (size.0 - canvas::GUTTER_W).max(1.0),
        (size.1 - strip - canvas::RULER_H).max(1.0),
    )
}

/// The kit whose lanes cover the most of a take's pitches.
///
/// The FTS map first, since that is what the templates play; General
/// MIDI when the take clearly is not one of ours.
fn kit_for(doc: &expression_editor_core::ExpressionDoc) -> DrumMap {
    let fts = DrumMap::fts();
    let gm = DrumMap::general_midi();
    let covered = |map: &DrumMap| {
        doc.notes
            .iter()
            .filter(|n| map.row_of_pitch(n.row).is_some())
            .count()
    };
    if covered(&gm) > covered(&fts) { gm } else { fts }
}

/// Whether a track's name says it carries drums.
#[must_use]
pub fn is_drum_track(name: &str) -> bool {
    const WORDS: [&str; 22] = [
        "drum", "drums", "kit", "kick", "bd", "snare", "sd", "tom", "toms", "rack", "floor",
        "hat", "hats", "hihat", "cymbal", "cymbals", "ride", "crash", "oh", "overhead",
        "overheads", "room",
    ];
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| WORDS.contains(&word))
}

const fn mods_of(m: Mods) -> tools::Mods {
    tools::Mods {
        ctrl: m.ctrl,
        shift: m.shift,
        alt: m.alt,
    }
}

#[cfg(test)]
mod tests {
    //! The editor driven the way the window drives it: window points,
    //! window modifiers, and the editor's own document read back.

    use super::*;

    const ORIGIN: (f64, f64) = (40.0, 30.0);
    const SIZE: (f64, f64) = (900.0, 500.0);

    fn plain() -> Mods {
        Mods::default()
    }

    fn view() -> Expression {
        Expression::demo(ORIGIN, SIZE)
    }

    /// Window coordinates of a note's left edge, mid-row.
    fn at_note(v: &Expression, id: u64) -> (f64, f64) {
        let ed = &v.editor;
        let n = ed
            .doc
            .notes
            .iter()
            .find(|n| n.id.0 == id)
            .expect("the demo has this note");
        let x = ed.camera.x(n.start) + 3.0;
        let y = ed.camera.y(f64::from(n.row), ed.viewport);
        (
            x + canvas::GUTTER_W + ORIGIN.0,
            y + canvas::RULER_H + ORIGIN.1,
        )
    }

    #[test]
    fn the_demo_is_a_kit_on_lanes() {
        let v = view();
        assert_eq!(v.editor.mode, Mode::Drums);
        assert!(matches!(v.editor.row_space, RowSpace::Drums(_)));
        assert!(!v.editor.doc.notes.is_empty());
    }

    #[test]
    fn the_box_less_the_chrome_is_the_viewport() {
        let v = view();
        assert!((v.editor.viewport.w - (SIZE.0 - canvas::GUTTER_W)).abs() < 1e-9);
        assert!((v.editor.viewport.h - (SIZE.1 - 96.0 - canvas::RULER_H)).abs() < 1e-9);
    }

    #[test]
    fn a_press_outside_the_box_is_not_taken() {
        let mut v = view();
        assert!(!v.press(ORIGIN.0 - 1.0, ORIGIN.1 + 100.0, plain(), 0));
        assert!(!v.press(ORIGIN.0 + 100.0, ORIGIN.1 + SIZE.1 + 1.0, plain(), 0));
    }

    #[test]
    fn a_click_on_a_hit_selects_it() {
        let mut v = view();
        let (x, y) = at_note(&v, 1);
        assert!(v.press(x, y, plain(), 0));
        assert!(v.release(x, y, plain()));
        assert!(v.editor.selection.contains(expression_editor_core::doc::NoteId(1)));
        assert!(!v.dragging());
    }

    /// The drum map's own convention: a drag on a hit is its velocity.
    #[test]
    fn a_drag_up_on_a_hit_raises_its_velocity() {
        let mut v = view();
        let before = v.editor.doc.notes[0].velocity;
        let (x, y) = at_note(&v, 1);
        v.press(x, y, plain(), 0);
        assert!(v.dragging());
        v.moved(x, y - 40.0, plain());
        v.release(x, y - 40.0, plain());
        assert!(!v.dragging());
        let after = v.editor.doc.notes[0].velocity;
        assert!(after > before, "velocity did not rise: {before} -> {after}");
        // And it was not also a click: nothing got selected by a drag.
        assert!(v.editor.selection.notes.is_empty());
    }

    /// Shift+drag moves a hit in time — snapped, so it lands a beat on.
    #[test]
    fn a_shift_drag_moves_a_hit_in_time() {
        let mut v = view();
        let before = v.editor.doc.notes[0].start;
        let (x, y) = at_note(&v, 1);
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        v.press(x, y, shift, 0);
        let beat = v.editor.units_per_beat() / v.editor.camera.units_per_px;
        v.moved(x + beat, y, shift);
        v.release(x + beat, y, shift);
        let after = v.editor.doc.notes[0].start;
        assert!(after > before, "the hit did not move: {before} -> {after}");
    }

    #[test]
    fn delete_removes_the_selection() {
        let mut v = view();
        let count = v.editor.doc.notes.len();
        let (x, y) = at_note(&v, 1);
        v.press(x, y, plain(), 0);
        v.release(x, y, plain());
        assert!(v.key("Delete", plain()));
        assert_eq!(v.editor.doc.notes.len(), count - 1);
    }

    #[test]
    fn the_strip_sets_velocity_by_height() {
        let mut v = view();
        let (x, _) = at_note(&v, 1);
        // Near the top of the strip: loud.
        let top = ORIGIN.1 + v.roll_h() + 4.0;
        assert!(v.press(x, top, plain(), 0));
        v.release(x, top, plain());
        let loud = v.editor.doc.notes[0].velocity;
        assert!(loud > 0.9, "top of the strip should be loud: {loud}");
        // Near the bottom: quiet.
        let bottom = ORIGIN.1 + SIZE.1 - 4.0;
        v.press(x, bottom, plain(), 0);
        v.release(x, bottom, plain());
        let quiet = v.editor.doc.notes[0].velocity;
        assert!(quiet < 0.1, "bottom of the strip should be quiet: {quiet}");
    }

    #[test]
    fn a_middle_drag_over_the_strip_pans_time() {
        let mut v = view();
        let t0 = v.editor.camera.time_span(v.editor.viewport).0;
        let y = ORIGIN.1 + v.roll_h() + 20.0;
        v.press(ORIGIN.0 + 400.0, y, plain(), 1);
        v.moved(ORIGIN.0 + 300.0, y, plain());
        v.release(ORIGIN.0 + 300.0, y, plain());
        let t1 = v.editor.camera.time_span(v.editor.viewport).0;
        assert!(t1 > t0, "dragging left should show later time: {t0} -> {t1}");
    }

    #[test]
    fn a_track_is_drums_by_its_words() {
        assert!(is_drum_track("Kick In"));
        assert!(is_drum_track("OH L"));
        assert!(is_drum_track("Snare Top"));
        assert!(!is_drum_track("Lead Vocal"));
        // A word inside a word is not the word.
        assert!(!is_drum_track("John Tomlinson"));
    }

    #[test]
    fn a_take_becomes_lanes() {
        use daw::service::midi::{MidiNote, MidiTakeSnapshot};
        let fts = DrumMap::fts();
        let kick = fts.lanes[0].pitch;
        let note = |pitch: i32, at: f64| MidiNote {
            index: 0,
            channel: 0,
            pitch: u8::try_from(pitch).unwrap_or(36),
            velocity: 100,
            start_ppq: at,
            length_ppq: 120.0,
            selected: false,
            muted: false,
        };
        let snapshot = MidiTakeSnapshot {
            notes: vec![note(kick, 0.0), note(kick, 960.0), note(127, 480.0)],
            ccs: vec![],
            pitch_bends: vec![],
            channel_pressures: vec![],
            poly_pressures: vec![],
            note_expressions: vec![],
            ppq: 960.0,
            length_ppq: 3840.0,
        };
        let v = Expression::from_take(&snapshot, "item".into(), true, ORIGIN, SIZE);
        // The kick lands on lane 0; pitch 127 has no lane and is dropped.
        assert_eq!(v.editor.doc.notes.len(), 2);
        assert!(v.editor.doc.notes.iter().all(|n| n.row == 0));
        assert_eq!(v.item.as_deref(), Some("item"));
    }
}
