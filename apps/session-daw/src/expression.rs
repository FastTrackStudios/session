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
//! The view is a box in window space: the toolbar across the top, the
//! roll under it with its key gutter and ruler, the velocity strip
//! under that, and the status bar along the bottom. The editor's
//! viewport is the roll's *note area* — the box minus all that chrome —
//! which is the convention every `interaction` handler assumes, so the
//! pointer is translated into roll space here and nowhere else.
//!
//! # Input
//!
//! Keys go through the editor's own keymap first (`keys::resolve`,
//! with its prefixes and sequences, and the spring-loaded zoom and
//! velocity tools), then the mode's own keys, then the plain
//! `interaction::key_down` table — the same order the Dioxus roll
//! uses. A right-click opens the context menu the core builds for
//! what was under it; the toolbar and status bar are the core's state
//! drawn as buttons, so a press on one is one call back into it.

use anyrender::PaintScene;
use expression_editor_core::mouse::{Action, Context, Gesture};
use expression_editor_core::rows::DrumMap;
use expression_editor_core::tools::{self, Hit};
use expression_editor_core::{Edit, Editor, Mode, RowSpace, StripLane, Tool, Viewport, memagic};
use expression_editor_paint::chrome::{self, Button, Choice, Control, Menu, Pending, STATUS_H, TOOLBAR_H};
use expression_editor_paint::interaction::{self, Drag};
use expression_editor_paint::paint::{self, Look, Overlay};
use expression_editor_paint::text::Labeller;
use expression_editor_paint::stack::{self, HitGesture, Stack, interact::ViewKey};
use expression_editor_paint::{canvas, demo, keys};
use input::InputCommand;
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
    /// The bars' buttons, laid out for this box and this state.
    toolbar: Vec<Button>,
    status: Vec<Button>,
    /// The button under the pointer.
    hover: Option<Control>,
    /// Where the pointer last was over the roll, in roll space — what
    /// a zoom key anchors on.
    hover_at: Option<(f64, f64)>,
    /// The context menu, while one is open.
    menu: Option<Menu>,
    /// The tool before a spring-loaded one was armed by a held key.
    spring_from: Option<Tool>,
    /// What a menu command still needs from the host.
    pending: Option<Pending>,
    /// The colours, from the host's palette.
    look: Look,
    /// The stacked view's gestures, for a workspace of several tracks
    /// — the audio drum workflow's surface.
    stack: Stack,
    /// The stack's picture, kept until the view it was built for
    /// changes: a transport tick or a slip then replays it and draws
    /// the overlays, rather than laying a song's worth of hits out
    /// again.
    stack_cache: Option<(ViewKey, anyrender::Scene)>,
}

/// Which part of the view a point is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    Toolbar,
    Roll,
    Strip,
    Status,
    Outside,
}

impl Expression {
    /// The demo drum groove, sized to `size` at `origin`.
    #[must_use]
    pub fn demo(origin: (f64, f64), size: (f64, f64)) -> Self {
        let vp = viewport_for(size, DEFAULT_STRIP);
        let editor = demo::editor(demo::Scene::Drums, vp);
        let mut this = Self::hold(editor, None, origin, size);
        this.editor.set_mode(Mode::Drums);
        this.relayout();
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
        let vp = viewport_for(size, DEFAULT_STRIP);
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
        let mut this = Self::hold(editor, Some(item), origin, size);
        this.relayout();
        this
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
            toolbar: Vec::new(),
            status: Vec::new(),
            hover: None,
            hover_at: None,
            menu: None,
            spring_from: None,
            pending: None,
            look: Look::default(),
            stack: Stack::editable(),
            stack_cache: None,
        }
    }

    /// Forget the stack's picture — the document changed under it.
    fn invalidate_stack(&mut self) {
        self.stack_cache = None;
    }

    /// A tracked kit's audio, as the drum workflow edits it: every mic
    /// with its peaks and its hits, folded into role lanes, stacked.
    /// `bars` of groove — two hundred is a song's worth and some ten
    /// thousand hits.
    #[must_use]
    pub fn audio_kit(bars: usize, origin: (f64, f64), size: (f64, f64)) -> Self {
        let vp = viewport_for(size, 0.0);
        let editor = demo::audio_kit(bars, vp);
        let mut this = Self::hold(editor, None, origin, size);
        this.relayout();
        this
    }

    /// Whether the stacked view is showing rather than the roll.
    #[must_use]
    pub const fn stacked(&self) -> bool {
        self.editor.stacked
    }

    /// Carry out what the stack asked of the host. This window has no
    /// audio to slip, so the hit list is what changes — see
    /// `stack::interact::apply_to_document`.
    fn apply_hits(&mut self, gestures: &[HitGesture]) -> bool {
        let mut changed = false;
        for g in gestures {
            changed |= stack::interact::apply_to_document(&mut self.editor, g);
        }
        if changed {
            self.invalidate_stack();
        }
        changed
    }

    /// Draw in the host's colours from now on.
    pub const fn set_look(&mut self, look: Look) {
        self.look = look;
    }

    /// Give the view its box. Cheap when nothing changed.
    pub fn layout(&mut self, origin: (f64, f64), size: (f64, f64)) {
        self.origin = origin;
        if self.size != size {
            self.size = size;
            self.relayout();
        }
    }

    /// The editor's viewport and the bars, for the box and the state
    /// now. The bars are rebuilt every frame anyway — they are the
    /// state drawn — so this is the one place that sizes anything.
    fn relayout(&mut self) {
        // The stack has no strip and its own, taller ruler.
        let vp = if self.stacked() {
            let ruler = Stack::ruler_h(&self.editor);
            Viewport::new(
                (self.size.0 - canvas::GUTTER_W).max(1.0),
                (self.size.1 - TOOLBAR_H - STATUS_H - ruler).max(1.0),
            )
        } else {
            viewport_for(self.size, self.editor.lane_strip_h)
        };
        if (vp.w - self.editor.viewport.w).abs() > 0.5 || (vp.h - self.editor.viewport.h).abs() > 0.5 {
            self.editor.resize(vp);
        }
        self.toolbar = chrome::toolbar(&self.editor, self.size.0, &mut self.labels);
        self.status = chrome::status(&self.editor, self.size.0, &mut self.labels);
    }

    /// The roll's height — the box less the bars and the strip.
    fn roll_h(&self) -> f64 {
        let strip = if self.stacked() { 0.0 } else { self.editor.lane_strip_h };
        (self.size.1 - TOOLBAR_H - STATUS_H - strip).max(canvas::RULER_H + 1.0)
    }

    /// A window point in the stack's space — past the toolbar, with
    /// the gutter and the ruler still inside it.
    fn stack_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        (lx, ly - Self::roll_top())
    }

    /// The playhead in seconds, for the stack.
    fn playhead_secs(&self) -> Option<f64> {
        let ups = self.editor.doc.time_base.units_per_second(self.editor.bpm);
        self.editor.playhead.filter(|_| ups > 1e-9).map(|p| p / ups)
    }

    /// Where the roll starts, below the toolbar.
    const fn roll_top() -> f64 {
        TOOLBAR_H
    }

    /// Draw the bars, the roll, the strip and any menu into the frame.
    pub fn paint(&mut self, painter: &mut impl PaintScene) {
        // The strip may have been toggled by a key or a button since
        // the last frame; the viewport follows it.
        self.relayout();
        let (w, h) = self.size;
        let (ox, oy) = self.origin;
        let roll_h = self.roll_h();
        let strip_h = (h - TOOLBAR_H - STATUS_H - roll_h).max(0.0);
        self.overlay.marquee = match &self.drag {
            Drag::Marquee { origin, current, .. } => Some((
                origin.0.min(current.0),
                origin.1.min(current.1),
                (current.0 - origin.0).abs(),
                (current.1 - origin.1).abs(),
            )),
            _ => None,
        };
        self.overlay.razor = match &self.drag {
            Drag::RazorCreate { pending, .. } => *pending,
            _ => None,
        };
        let bar = chrome::paint(&self.toolbar, self.hover, w, TOOLBAR_H, &mut self.labels, &self.look);
        painter.append_scene(bar, Affine::translate((ox, oy)));
        if self.stacked() {
            let key = self.stack.view_key(&self.editor);
            let at = Affine::translate((ox, oy + TOOLBAR_H));
            let fresh = self.stack_cache.as_ref().is_none_or(|(k, _)| *k != key);
            if fresh {
                let scene = self.stack.scene(&self.editor, &mut self.labels, &self.look);
                self.stack_cache = Some((key, scene));
            }
            if let Some((_, scene)) = &self.stack_cache {
                for cmd in &scene.commands {
                    stack::submit(painter, cmd, at);
                }
            }
            let playhead = self.playhead_secs();
            let over = self.stack.overlays(&self.editor, playhead, &self.look);
            painter.append_scene(over, at);
        } else {
            let roll = paint::roll_scene(&self.editor, w, roll_h, &self.overlay, &mut self.labels, &self.look);
            painter.append_scene(roll, Affine::translate((ox, oy + TOOLBAR_H)));
            if strip_h > 0.0 {
                let strip = paint::strip_scene(&self.editor, w, strip_h, &mut self.labels, &self.look);
                painter.append_scene(strip, Affine::translate((ox, oy + TOOLBAR_H + roll_h)));
            }
        }
        let status = chrome::paint(&self.status, self.hover, w, STATUS_H, &mut self.labels, &self.look);
        painter.append_scene(status, Affine::translate((ox, oy + h - STATUS_H)));
        if let Some(menu) = &self.menu {
            let scene = menu.paint(&mut self.labels, &self.look);
            painter.append_scene(
                scene,
                Affine::translate((ox + canvas::GUTTER_W, oy + TOOLBAR_H + canvas::RULER_H)),
            );
        }
    }

    /// What a menu command still needs from the host, once.
    pub const fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    /// A window point in the view's own space.
    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.origin.0, y - self.origin.1)
    }

    fn zone(&self, x: f64, y: f64) -> Zone {
        let (lx, ly) = self.local(x, y);
        if lx < 0.0 || ly < 0.0 || lx >= self.size.0 || ly >= self.size.1 {
            Zone::Outside
        } else if ly < Self::roll_top() {
            Zone::Toolbar
        } else if ly >= self.size.1 - STATUS_H {
            Zone::Status
        } else if ly < Self::roll_top() + self.roll_h() {
            Zone::Roll
        } else {
            Zone::Strip
        }
    }

    /// Whether a window point is inside the view's box.
    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.zone(x, y) != Zone::Outside
    }

    /// A window point in roll space — past the toolbar, the gutter and
    /// the ruler.
    fn roll_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        (lx - canvas::GUTTER_W, ly - Self::roll_top() - canvas::RULER_H)
    }

    /// A window point in strip space.
    fn strip_point(&self, x: f64, y: f64) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        (lx, ly - Self::roll_top() - self.roll_h())
    }

    /// A window point in a bar's space.
    fn bar_point(&self, x: f64, y: f64, zone: Zone) -> (f64, f64) {
        let (lx, ly) = self.local(x, y);
        match zone {
            Zone::Status => (lx, ly - (self.size.1 - STATUS_H)),
            _ => (lx, ly),
        }
    }

    /// Whether a gesture is in flight, so the window keeps sending
    /// moves here even after the pointer leaves the box.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.drag.is_active() || self.strip_drag || self.strip_pan.is_some() || self.stack.dragging()
    }

    /// A button press. `button` is 0 left, 1 middle, 2 right. `true`
    /// when the view took it.
    pub fn press(&mut self, x: f64, y: f64, mods: Mods, button: u16) -> bool {
        // An open menu owns the next press: on an item it runs it, and
        // anywhere else it closes without doing anything — a click
        // that both dismissed a menu and moved a note would be two
        // surprises in one.
        if let Some(menu) = self.menu.take() {
            let (rx, ry) = self.roll_point(x, y);
            if button == 0
                && let Choice::Done(pending) = menu.choose(&mut self.editor, rx, ry)
            {
                self.pending = pending;
            }
            return true;
        }
        let zone = self.zone(x, y);
        if self.stacked() && matches!(zone, Zone::Roll | Zone::Strip) {
            let (sx, sy) = self.stack_point(x, y);
            let mut out = Vec::new();
            let took = self.stack.press(&mut self.editor, sx, sy, button, mods_of(mods), &mut out);
            let edited = self.apply_hits(&out);
            if took || edited {
                self.relayout();
            }
            return took || edited;
        }
        match zone {
            Zone::Outside => false,
            Zone::Toolbar | Zone::Status => {
                let zone = self.zone(x, y);
                let (bx, by) = self.bar_point(x, y, zone);
                let bar = if zone == Zone::Toolbar { &self.toolbar } else { &self.status };
                if button == 0
                    && let Some(control) = chrome::hit(bar, bx, by)
                    && bar.iter().any(|b| b.control == control && b.enabled)
                {
                    chrome::activate(&mut self.editor, &self.drag, control);
                    self.invalidate_stack();
                    self.relayout();
                }
                true
            }
            // r[impl flow.keys.midi-editing]
            Zone::Roll => {
                let (rx, ry) = self.roll_point(x, y);
                let under = match self.editor.hit_test(rx, ry) {
                    Hit::Note { id, .. } | Hit::NoteEdge { id, .. } => Some(id),
                    _ => None,
                };
                self.pressed = (button == 0).then_some(((rx, ry), under));
                let drag = interaction::pointer_down(&mut self.editor, rx, ry, mods_of(mods), button);
                // A right-click resolves to a menu request rather than
                // a drag. Opened here, so the core pointer path stays
                // free of UI state — the same split as the Dioxus roll.
                self.drag = match drag {
                    Drag::ContextMenu { x, y, under, t, row } => {
                        self.menu = Some(Menu::open(&self.editor, x, y, under, t, row));
                        Drag::None
                    }
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
        let zone = self.zone(x, y);
        self.hover_at = (zone == Zone::Roll).then(|| self.roll_point(x, y));
        let (rx, ry) = self.roll_point(x, y);
        if let Some(menu) = self.menu.as_mut() {
            return menu.hover_at(rx, ry);
        }
        if self.stacked() && (self.stack.dragging() || matches!(zone, Zone::Roll | Zone::Strip)) {
            let (sx, sy) = self.stack_point(x, y);
            return self.stack.moved(&mut self.editor, sx, sy, mods_of(mods));
        }
        let hover = match zone {
            Zone::Toolbar => {
                let (bx, by) = self.bar_point(x, y, zone);
                chrome::hit(&self.toolbar, bx, by)
            }
            Zone::Status => {
                let (bx, by) = self.bar_point(x, y, zone);
                chrome::hit(&self.status, bx, by)
            }
            _ => None,
        };
        let lit = hover != self.hover;
        self.hover = hover;
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
            return lit;
        }
        let (rx, ry) = self.roll_point(x, y);
        interaction::pointer_move(&mut self.editor, &mut self.drag, rx, ry, mods_of(mods));
        true
    }

    /// The button came up. `true` when a gesture ended.
    pub fn release(&mut self, x: f64, y: f64, mods: Mods) -> bool {
        if self.stacked() {
            let mut out = Vec::new();
            let ended = self.stack.release(&mut self.editor, &mut out);
            let edited = self.apply_hits(&out);
            return ended || edited;
        }
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
        if self.stacked() {
            let (sx, sy) = self.stack_point(x, y);
            self.stack.moved(&mut self.editor, sx, sy, mods_of(mods));
            return self.stack.wheel(&mut self.editor, dx, dy, mods_of(mods));
        }
        let (rx, ry) = self.roll_point(x, y);
        interaction::wheel(&mut self.editor, rx, ry, dx, dy, mods_of(mods));
        true
    }

    /// A key, by its browser-style name (`"Delete"`, `"ArrowLeft"`,
    /// `"a"`). `true` when the editor took it.
    pub fn key(&mut self, key: &str, mods: Mods) -> bool {
        let m = mods_of(mods);
        if key == "Escape" && self.menu.take().is_some() {
            return true;
        }
        // The stack's own keys first while it is showing: zoom,
        // paging, nudge and delete on the selected hit.
        if self.stacked() {
            let mut out = Vec::new();
            let took = self.stack.key(&mut self.editor, key, m, &mut out);
            let edited = self.apply_hits(&out);
            if took || edited {
                self.relayout();
                return true;
            }
        }
        // Escape backs out of a half-typed sequence rather than firing
        // whatever a bare Escape means.
        if key == "Escape" && keys::is_pending() {
            keys::cancel();
            return true;
        }
        // The editor's own keymap: prefixes, sequences, the zoom tree.
        let commands = keys::resolve(key, m);
        let mut ran = false;
        for cmd in &commands {
            let action = match cmd {
                InputCommand::Action(a) => Some(a.0.as_str()),
                InputCommand::ActionWithArgs { action, .. } => Some(action.0.as_str()),
                _ => None,
            };
            if let Some(action) = action {
                let (region, anchor) = self.memagic();
                ran |= keys::dispatch(&mut self.editor, action, region, anchor);
            }
        }
        // A spring-loaded tool arms the instant its prefix goes down,
        // so the toolbar lights while the surface is already in it.
        if let Some(armed) = keys::held_prefix().as_deref().and_then(spring_tool)
            && self.spring_from.is_none()
        {
            let previous = self.editor.tool;
            if previous != armed {
                self.spring_from = Some(previous);
                self.editor.tool = armed;
            }
        }
        if ran || keys::is_pending() {
            self.invalidate_stack();
            self.relayout();
            return true;
        }
        // Drum-mode keys before the general ones: `f` is a flam here.
        if key == "f" && !m.ctrl && self.editor.mode == Mode::Drums && self.editor.flam_selection() > 0 {
            return true;
        }
        let took = interaction::key_down(&mut self.editor, &self.drag, key, m);
        if took {
            self.invalidate_stack();
            self.relayout();
        }
        took
    }

    /// A key came up. The keymap has to hear it, or a held prefix
    /// walks its sequence tree on auto-repeat; and a spring-loaded tool
    /// goes back when its key does.
    pub fn key_up(&mut self, key: &str, mods: Mods) -> bool {
        keys::release(key, mods_of(mods));
        let mut changed = self.stacked() && self.stack.key_up(&mut self.editor, key);
        match key {
            "r" => {
                self.editor.refs_to_front = false;
                changed = true;
            }
            "m" => {
                self.editor.reference_to_front = false;
                changed = true;
            }
            _ => {}
        }
        if spring_tool(key).is_some()
            && let Some(previous) = self.spring_from.take()
        {
            self.editor.tool = previous;
            changed = true;
        }
        if changed {
            self.relayout();
        }
        changed
    }

    /// Where a zoom key anchors: the playhead while running, else the
    /// pointer, else the middle of the view.
    fn memagic(&self) -> (memagic::Region, memagic::Anchor) {
        let ed = &self.editor;
        let Some((x, y)) = self.hover_at else {
            return (
                memagic::Region::Elsewhere,
                memagic::Anchor {
                    t: ed.playhead.unwrap_or_else(|| ed.camera.t_at(ed.viewport.w * 0.5)),
                    row: None,
                },
            );
        };
        let region = if y < 0.0 {
            memagic::Region::Ruler
        } else if x < 0.0 {
            memagic::Region::Piano
        } else {
            memagic::Region::NoteArea
        };
        (
            region,
            memagic::Anchor {
                t: ed.playhead.unwrap_or_else(|| ed.camera.t_at(x)),
                row: Some(ed.camera.pitch_at(y, ed.viewport)),
            },
        )
    }

    /// Set the velocity of the notes under a strip point to its height.
    ///
    /// A generous grab around the onset: a stem is a few pixels wide and
    /// this is a value edit, not a precision selection.
    fn strip_write(&mut self, at_x: f64, at_y: f64) {
        let strip_h = (self.size.1 - TOOLBAR_H - STATUS_H - self.roll_h()).max(1.0);
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

/// The editor's colours from the window's palette.
///
/// So a docked roll reads as part of the arrangement above it rather
/// than a second application: the same surface, the same row shades,
/// the same grid and text, the accent for what is selected and
/// playing. A drum family's band is only a tint over the rows here,
/// where the standalone editor paints it solid.
#[must_use]
pub fn look_of(palette: &crate::arrangement::Palette) -> Look {
    let canonical = Look::default();
    Look {
        bg: palette.surface,
        row_a: palette.row_a,
        row_b: palette.row_b,
        grid_beat: palette.grid_beat,
        grid_sub: palette.grid,
        panel: palette.tcp_tint,
        panel_border: palette.divider,
        border_strong: palette.item_edge,
        surface_inset: palette.tcp_field,
        surface_bar: palette.tcp_gutter,
        gutter_bg: palette.tcp_column,
        key_white: palette.tcp_button,
        key_black: palette.tcp_field,
        key_label: palette.text_dim,
        text: palette.text,
        text_dim: palette.text_dim,
        text_bright: palette.text,
        accent: palette.accent,
        selected: palette.accent,
        playhead: palette.accent,
        control: palette.tcp_button,
        control_active: palette.accent.multiply_alpha(0.55),
        control_hover: palette.tcp_field,
        band_tint: 0.35,
        ..canonical
    }
}

/// How far a press may travel and still be a click, in pixels.
const CLICK_SLOP: f64 = 3.0;
/// The strip's height before the editor has said otherwise.
const DEFAULT_STRIP: f64 = 96.0;

/// The tools a held key springs into: `z` zooms, `v` edits velocity.
fn spring_tool(key: &str) -> Option<Tool> {
    match key {
        "z" => Some(Tool::Zoom),
        "v" => Some(Tool::Velocity),
        _ => None,
    }
}

/// The roll's viewport inside a box: the note area, less the bars, the
/// gutter, the ruler and the strip.
fn viewport_for(size: (f64, f64), strip_h: f64) -> Viewport {
    Viewport::new(
        (size.0 - canvas::GUTTER_W).max(1.0),
        (size.1 - TOOLBAR_H - STATUS_H - strip_h - canvas::RULER_H).max(1.0),
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
            y + canvas::RULER_H + ORIGIN.1 + TOOLBAR_H,
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
        assert!(
            (v.editor.viewport.h - (SIZE.1 - TOOLBAR_H - STATUS_H - 96.0 - canvas::RULER_H)).abs()
                < 1e-9
        );
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
    // r[verify flow.keys.midi-editing]
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
    // r[verify flow.keys.midi-editing]
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
        let top = ORIGIN.1 + TOOLBAR_H + v.roll_h() + 4.0;
        assert!(v.press(x, top, plain(), 0));
        v.release(x, top, plain());
        let loud = v.editor.doc.notes[0].velocity;
        assert!(loud > 0.9, "top of the strip should be loud: {loud}");
        // Near the bottom: quiet.
        let bottom = ORIGIN.1 + SIZE.1 - STATUS_H - 4.0;
        v.press(x, bottom, plain(), 0);
        v.release(x, bottom, plain());
        let quiet = v.editor.doc.notes[0].velocity;
        assert!(quiet < 0.1, "bottom of the strip should be quiet: {quiet}");
    }

    #[test]
    fn a_middle_drag_over_the_strip_pans_time() {
        let mut v = view();
        let t0 = v.editor.camera.time_span(v.editor.viewport).0;
        let y = ORIGIN.1 + TOOLBAR_H + v.roll_h() + 20.0;
        v.press(ORIGIN.0 + 400.0, y, plain(), 1);
        v.moved(ORIGIN.0 + 300.0, y, plain());
        v.release(ORIGIN.0 + 300.0, y, plain());
        let t1 = v.editor.camera.time_span(v.editor.viewport).0;
        assert!(t1 > t0, "dragging left should show later time: {t0} -> {t1}");
    }

    #[test]
    fn a_toolbar_press_sets_the_tool() {
        let mut v = view();
        let button = v
            .toolbar
            .iter()
            .find(|b| b.control == Control::Tool(Tool::Eraser))
            .expect("an eraser button")
            .rect
            .center();
        assert!(v.press(ORIGIN.0 + button.x, ORIGIN.1 + button.y, plain(), 0));
        v.release(ORIGIN.0 + button.x, ORIGIN.1 + button.y, plain());
        assert_eq!(v.editor.tool, Tool::Eraser);
        // And the bar shows it lit — as the mode shows it, which for
        // drums is the note eraser.
        let shown = v.editor.shown_tool();
        assert!(v.toolbar.iter().any(|b| b.control == Control::Tool(shown) && b.active));
    }

    #[test]
    fn a_status_press_toggles_snap() {
        let mut v = view();
        let was = v.editor.grid.enabled;
        let button = v
            .status
            .iter()
            .find(|b| b.control == Control::SnapGrid)
            .expect("a snap button")
            .rect
            .center();
        let y = ORIGIN.1 + SIZE.1 - STATUS_H + button.y;
        assert!(v.press(ORIGIN.0 + button.x, y, plain(), 0));
        assert_eq!(v.editor.grid.enabled, !was);
    }

    #[test]
    fn a_right_click_opens_the_menu_and_a_click_elsewhere_closes_it() {
        let mut v = view();
        let (x, y) = at_note(&v, 1);
        assert!(v.press(x, y, plain(), 2));
        v.release(x, y, plain());
        assert!(v.menu.is_some(), "a right-click on a hit opens its menu");
        // A press outside the menu closes it and does nothing else.
        let count = v.editor.doc.notes.len();
        v.press(x + 400.0, y, plain(), 0);
        assert!(v.menu.is_none());
        assert_eq!(v.editor.doc.notes.len(), count);
    }

    #[test]
    fn the_menu_runs_a_command_on_the_hit() {
        let mut v = view();
        let (x, y) = at_note(&v, 1);
        v.press(x, y, plain(), 2);
        v.release(x, y, plain());
        let menu = v.menu.as_ref().expect("a menu");
        let delete = menu
            .items
            .iter()
            .position(|i| i.command == expression_editor_core::menu::Command::Delete)
            .expect("delete on the menu");
        let (rx, ry) = {
            let r = menu.item_rect(delete);
            (r.center().x, r.center().y)
        };
        let count = v.editor.doc.notes.len();
        // Menu space is roll space; back to the window.
        let wx = rx + canvas::GUTTER_W + ORIGIN.0;
        let wy = ry + canvas::RULER_H + ORIGIN.1 + TOOLBAR_H;
        v.press(wx, wy, plain(), 0);
        assert!(v.menu.is_none());
        assert_eq!(v.editor.doc.notes.len(), count - 1);
    }

    #[test]
    fn a_held_zoom_key_springs_the_tool_and_lets_go() {
        let mut v = view();
        let before = v.editor.tool;
        v.key("z", plain());
        assert_eq!(v.editor.tool, Tool::Zoom);
        v.key_up("z", plain());
        assert_eq!(v.editor.tool, before);
    }

    fn kit() -> Expression {
        Expression::audio_kit(16, ORIGIN, SIZE)
    }

    #[test]
    fn the_audio_kit_is_a_stack_of_role_lanes_with_many_hits() {
        let v = kit();
        assert!(v.stacked());
        assert_eq!(v.editor.tracks.len(), 12);
        // Sixteen bars of snare on the twos and fours, with a ghost
        // every fourth bar, is thirty-six hits per snare mic; the hats
        // are heard and not detected, so their mic carries none.
        let notes_of = |name: &str| {
            v.editor
                .tracks
                .index_of(name)
                .and_then(|i| v.editor.tracks.doc_of(i))
                .map_or(0, |d| d.notes.len())
        };
        assert_eq!(notes_of("Snare Top"), 16 * 2 + 4);
        assert_eq!(notes_of("HH"), 0);
        assert!(!v.editor.doc.peaks.is_empty());
    }

    #[test]
    fn a_wheel_over_the_stack_zooms_time() {
        let mut v = kit();
        let before = v.editor.camera.units_per_px;
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let x = ORIGIN.0 + 400.0;
        let y = ORIGIN.1 + TOOLBAR_H + 200.0;
        // The shared bindings put horizontal zoom on the wheel with a
        // modifier; whichever it is, the camera must move.
        let mut moved = false;
        for m in [ctrl, Mods { alt: true, ..Mods::default() }, Mods::default()] {
            v.wheel(x, y, 0.0, -3.0, m);
            if (v.editor.camera.units_per_px - before).abs() > 1e-12
                || (v.editor.camera.t0).abs() > 1e-9
            {
                moved = true;
                break;
            }
        }
        assert!(moved, "no wheel binding moved the stack's camera");
    }

    #[test]
    // r[verify flow.drums.editing.hands]
    fn dragging_a_kick_hit_slips_it_in_the_hit_list() {
        let mut v = kit();
        // Find the kick lane and its first hit on screen.
        let lanes = stack::lanes(&v.editor, Editor::ACTIVE_BOOST, v.editor.lane_floor().max(22.0));
        let kick = lanes
            .iter()
            .find(|l| l.name == "Kick")
            .expect("a kick lane");
        let hit = kick
            .notes
            .iter()
            .find(|n| n.x > 10.0 && n.x < v.editor.viewport.w - 10.0)
            .expect("a kick hit in view");
        let secs = hit.at_secs;
        let ruler = Stack::ruler_h(&v.editor);
        let x = ORIGIN.0 + canvas::GUTTER_W + hit.x;
        let y = ORIGIN.1 + TOOLBAR_H + ruler + kick.y + kick.h * 0.5;
        assert!(v.press(x, y, plain(), 0));
        assert!(v.dragging());
        v.moved(x + 40.0, y, plain());
        v.release(x + 40.0, y, plain());
        // The kick's document moved that hit later.
        let ups = v.editor.doc.time_base.units_per_second(v.editor.bpm);
        let kick_doc = v
            .editor
            .tracks
            .index_of("Kick In")
            .and_then(|i| {
                if i == v.editor.tracks.active() {
                    Some(&v.editor.doc)
                } else {
                    v.editor.tracks.doc_of(i)
                }
            })
            .expect("the kick's document");
        let still_there = kick_doc
            .notes
            .iter()
            .any(|n| (n.start / ups - secs).abs() < 0.002);
        // Forty pixels at a whole-song zoom is a second or two.
        let moved = kick_doc
            .notes
            .iter()
            .any(|n| n.start / ups > secs + 0.02 && n.start / ups < secs + 4.0 && (n.start / ups - 1.0).abs() > 0.01);
        assert!(!still_there && moved, "the hit at {secs}s did not slip");
    }

    /// A press on the other lane's audio selects the lane and never
    /// picks up or adds a hit: hats are context, not a hit list.
    #[test]
    // r[verify flow.drums.editing.stack]
    fn the_other_lane_takes_no_hit_gesture() {
        let mut v = kit();
        let lanes = stack::lanes(&v.editor, Editor::ACTIVE_BOOST, v.editor.lane_floor().max(22.0));
        let other = lanes.iter().find(|l| l.name == "Other").expect("an other lane");
        assert!(!other.detects && other.notes.is_empty());
        let ruler = Stack::ruler_h(&v.editor);
        let x = ORIGIN.0 + canvas::GUTTER_W + 200.0;
        let y = ORIGIN.1 + TOOLBAR_H + ruler + other.y + other.h * 0.5;
        let notes: usize = (0..v.editor.tracks.len())
            .map(|i| v.editor.tracks.doc_of(i).map_or(v.editor.doc.notes.len(), |d| d.notes.len()))
            .sum();
        // Alt-click is "add a hit" on a lane that detects.
        v.press(x, y, Mods { alt: true, ..Mods::default() }, 0);
        v.release(x, y, Mods { alt: true, ..Mods::default() });
        let after: usize = (0..v.editor.tracks.len())
            .map(|i| v.editor.tracks.doc_of(i).map_or(v.editor.doc.notes.len(), |d| d.notes.len()))
            .sum();
        assert_eq!(notes, after);
        assert!(!v.dragging());
    }

    /// Hold `z`, drag right and up: time and rows both zoom in, anchored
    /// on the press; let go of `z` and the tool springs back.
    #[test]
    fn holding_z_makes_a_drag_zoom_both_axes() {
        let mut v = view();
        let (upp, ppr) = (v.editor.camera.units_per_px, v.editor.camera.vertical.px_per_row);
        let before = v.editor.tool;
        assert!(v.key("z", plain()));
        assert_eq!(v.editor.tool, Tool::Zoom);
        let (x, y) = (ORIGIN.0 + 400.0, ORIGIN.1 + TOOLBAR_H + 200.0);
        let t_under = v.editor.camera.t_at(400.0 - canvas::GUTTER_W);
        assert!(v.press(x, y, plain(), 0));
        v.moved(x + 200.0, y - 200.0, plain());
        v.release(x + 200.0, y - 200.0, plain());
        assert!(v.editor.camera.units_per_px < upp, "time zoomed in");
        assert!(v.editor.camera.vertical.px_per_row > ppr, "rows zoomed in");
        assert!((v.editor.camera.t_at(400.0 - canvas::GUTTER_W) - t_under).abs() < 1e-6);
        v.key_up("z", plain());
        assert_eq!(v.editor.tool, before);
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
