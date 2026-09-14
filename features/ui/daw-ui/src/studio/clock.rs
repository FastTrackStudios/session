//! The playhead and the zoom — the two quantities that must never cause
//! a render.
//!
//! **The playhead.** A transport tick arrives about thirty times a
//! second. Writing it into a dioxus `Signal` re-renders everything that
//! reads it, and the thing that reads a playhead is the component
//! spanning the whole timeline — so "play" would mean re-rendering the
//! arrangement continuously while the material on screen did not change
//! at all. Instead the page owns the playhead: Rust publishes *events*
//! (seek here, start running, the tempo is this) and a
//! `requestAnimationFrame` loop extrapolates between them, writing one
//! `transform` and one text node. Playback costs zero renders, and —
//! because it extrapolates rather than follows — the line moves every
//! frame the compositor draws instead of stepping at the engine's tick
//! rate.
//!
//! **The zoom.** Every position in the window is `calc(var(--t0) *
//! var(--pps) * 1px)`, so zooming is one custom property. Publishing it
//! the same way means a wheel gesture re-renders nothing whatsoever; the
//! browser's style engine moves several thousand elements at compositor
//! speed.
//!
//! All the state that needs a clock to interpret it lives in the page,
//! which has one. Rust keeps only what a component has to *read* —
//! the zoom, for turning a click into a time, and the tempo — and those
//! are plain `Cell`s behind [`Clock`], not signals: this is a handle you
//! call, not state you render from. The one genuinely reactive piece,
//! whether the transport is running (a button draws it), stays a
//! separate `Signal<bool>` owned by the root.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::prelude::*;
use dioxus::document::Eval;

/// The page-side loop. Owns the playhead's motion, and the zoom variable
/// once Rust has published it.
///
/// Messages are `[kind, value]`:
/// - `0` — seek: the playhead is at `value` seconds
/// - `1` — zoom: `value` pixels per second
/// - `2` — transport: running if `value` is non-zero
/// - `3` — tempo: `value` BPM
/// - `4` — length: the song is `value` seconds long (the animation's
///   duration, so it advances in real time)
///
/// Every kind re-anchors first, so a change of scale or state never
/// snaps the line back to wherever it was last told about.
const CLOCK_JS: &str = r#"
const st = { playing: 0, origin: 0, pps: null, bpm: 120, len: 60 };
let el = null, anim = null, appliedPps = null;

// The playhead is animated by the ENGINE, not by this script.
//
// A `transform` written from a rAF callback is the script-driven
// rendering path, and WebKit caps that near 60 fps on purpose. A
// transform-only Web Animation is the *accelerated* path — the
// compositor owns it and runs it at the display's refresh rate, which is
// the whole reason the line is built this way. It also means playback
// costs no script at all between seeks.
const build = () => {
  if (!el || st.pps === null) return;
  // Carry the position and the run state across a rebuild, or a zoom
  // would restart the song from the top.
  const at = anim ? anim.currentTime : st.origin * 1000;
  const running = anim ? anim.playState === "running" : !!st.playing;
  if (anim) anim.cancel();
  anim = el.animate(
    [
      { transform: "translate3d(0px,0,0)" },
      { transform: "translate3d(" + st.len * st.pps + "px,0,0)" },
    ],
    { duration: st.len * 1000, easing: "linear", fill: "both" },
  );
  anim.currentTime = at;
  if (running) { anim.play(); } else { anim.pause(); }
};

// The line is mounted with the project, not with this script, and it is
// replaced whenever the lanes re-render — so the animation is attached
// from the frame loop rather than once at startup.
const attach = () => {
  const found = document.querySelector(".studio-playhead");
  if (!found) return;
  if (found !== el) { el = found; anim = null; build(); }
};

// Commands arrive as CALLS, not as channel messages.
//
// `Eval::send` from Rust is silently dropped by this renderer —
// MEASURED: the send returns `Ok`, and `dioxus.recv()` in here never
// fires (`commands_received` stayed 0 while tempo and length were sent).
// JS→Rust works, which is what the heartbeat below uses. So the driver
// evaluates a call to this function instead, which is the same mechanism
// that installed this script and is known to work.
let received = 0;
window.__ftsClockCmd = (kind, value) => {
  received += 1;
  {
    if (kind === 0) {
      st.origin = value;
      if (anim) { anim.currentTime = value * 1000; } 
    } else if (kind === 1) {
      st.pps = value;
      build();
    } else if (kind === 2) {
      st.playing = value;
      if (anim) { if (value) { anim.play(); } else { anim.pause(); } }
    } else if (kind === 3) {
      st.bpm = value;
    } else if (kind === 4) {
      st.len = value;
      build();
    }
  }
};

const two = (n) => (n < 10 ? "0" + n : "" + n);
const fmt = (secs, bpm) => {
  const beats = secs * bpm / 60;
  const bar = Math.floor(beats / 4) + 1;
  const beat = Math.floor(beats % 4) + 1;
  const m = Math.floor(secs / 60);
  const s = secs - m * 60;
  return bar + "." + beat + "." + two(Math.floor((beats % 1) * 100))
       + "   " + m + ":" + (s < 10 ? "0" : "") + s.toFixed(3);
};

// What is LEFT on the script path: the readout's text, which cannot be a
// compositor animation, and adopting the stylesheet's opening zoom. Both
// are fine at WebKit's capped rate — a clock that updates 60 times a
// second is not the thing anyone can see stutter. The line above is.
let lastText = "";
const tick = () => {
  attach();
  const root = document.querySelector(".studio");
  if (root) {
    if (st.pps === null) {
      const declared = parseFloat(getComputedStyle(root).getPropertyValue("--pps"));
      if (!Number.isNaN(declared)) { st.pps = declared; appliedPps = declared; build(); }
    } else if (appliedPps !== st.pps) {
      root.style.setProperty("--pps", st.pps);
      appliedPps = st.pps;
    }
  }
  const clock = document.querySelector(".studio-clock");
  if (clock) {
    const secs = anim ? anim.currentTime / 1000 : st.origin;
    const text = fmt(secs, st.bpm);
    if (text !== lastText) { clock.textContent = text; lastText = text; }
  }
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);

// The heartbeat, twice a second. It carries the clock's state for the
// log, and — more importantly — it is what gives `ClockDriver` something
// to await, which is what pumps commands the other way. See there.
setInterval(() => {
  dioxus.send([
    anim ? 1 : 0,
    anim ? anim.currentTime / 1000 : -1,
    st.pps === null ? -1 : st.pps,
    st.len,
    st.playing,
    received,
  ]);
}, 500);
"#;

/// How far the window may zoom. Below the floor a whole song is a smear;
/// above the ceiling a bar is wider than the screen.
pub const MIN_PPS: f64 = 0.5;
pub const MAX_PPS: f64 = 4_000.0;

/// The opening scale — wide enough to read the ruler, tight enough to
/// see the shape of a song.
///
/// **Must match `--pps` in [`super::css`].** The stylesheet is the one
/// that actually opens the window at this scale (the page reads it on
/// its first frame); this copy exists so a click can be turned into a
/// time before anything has zoomed.
pub const DEFAULT_PPS: f64 = 40.0;

struct Inner {
    /// Commands waiting to go to the page.
    ///
    /// NOT an `Eval` held here, and that is the whole shape of this
    /// type. A dioxus `Eval`'s channel only pumps while something is
    /// polling it: an `Eval` that is created, stashed and then only ever
    /// `send`-ed to delivers NOTHING, silently and forever. That bug is
    /// invisible — the page runs, the script is live, the sends return
    /// `Ok` — and it presents as a playhead that will not move and a
    /// zoom that will not apply. So the handle lives in
    /// [`ClockDriver`], which owns it and awaits it, and everything
    /// else queues here.
    queue: RefCell<Vec<[f64; 2]>>,
    /// Mirrored here only because components have to turn a pointer
    /// position into a time. The page holds the authoritative copy.
    pps: Cell<f64>,
    bpm: Cell<f64>,
    length: Cell<f64>,
}

/// A handle on the page clock. Cheap to clone; lives in context.
#[derive(Clone)]
pub struct Clock(Rc<Inner>);

impl PartialEq for Clock {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Clock {
    /// A clock with nothing driving it yet. Mount [`ClockDriver`] with
    /// it — until then, commands queue rather than being lost.
    pub fn new() -> Self {
        Self(Rc::new(Inner {
            queue: RefCell::new(Vec::new()),
            pps: Cell::new(DEFAULT_PPS),
            bpm: Cell::new(120.0),
            length: Cell::new(60.0),
        }))
    }

    /// Pixels per second, as the window is currently zoomed. Read by
    /// anything turning a pointer position into a time.
    pub fn pps(&self) -> f64 {
        self.0.pps.get()
    }

    pub fn bpm(&self) -> f64 {
        self.0.bpm.get()
    }

    /// The tempo the readout counts bars at. Set once the project is
    /// read; the ruler draws its own bar lines from the same number.
    pub fn set_bpm(&self, bpm: f64) {
        if (self.0.bpm.get() - bpm).abs() > f64::EPSILON {
            self.0.bpm.set(bpm);
            self.send(3, bpm);
        }
    }

    /// Move the playhead. Lands on this frame rather than after the
    /// engine has acknowledged — the UI's own value is the truthful one
    /// until the round trip completes.
    pub fn seek(&self, secs: f64) {
        self.send(0, secs.max(0.0));
    }

    /// Start or stop the line moving.
    pub fn set_playing(&self, playing: bool) {
        self.send(2, if playing { 1.0 } else { 0.0 });
    }

    /// How long the song is. The playhead's animation runs for exactly
    /// this many seconds, which is what makes one second of animation one
    /// second of music.
    pub fn set_length(&self, secs: f64) {
        let secs = secs.max(1.0);
        if (self.0.length.get() - secs).abs() > f64::EPSILON {
            self.0.length.set(secs);
            self.send(4, secs);
        }
    }

    /// Re-anchor to the engine without disturbing the play state.
    /// Rate-limited by its caller — see [`super::transport`].
    pub fn resync(&self, secs: f64) {
        self.send(0, secs);
    }

    /// Change the zoom. Writes `--pps` through the page, so the whole
    /// timeline re-lays out without a single dioxus render.
    pub fn set_pps(&self, pps: f64) {
        let pps = pps.clamp(MIN_PPS, MAX_PPS);
        self.0.pps.set(pps);
        self.send(1, pps);
    }

    fn send(&self, kind: u8, value: f64) {
        self.0.queue.borrow_mut().push([f64::from(kind), value]);
    }

    /// Take everything queued. Called only by [`ClockDriver`].
    fn drain(&self) -> Vec<[f64; 2]> {
        std::mem::take(&mut *self.0.queue.borrow_mut())
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns the page clock's channel and pumps it, both ways.
///
/// Mounted once, near the root. This is the component that makes
/// [`Clock`] work at all: it holds the `Eval`, and awaiting `recv` in a
/// loop is what drives the channel so queued commands actually reach the
/// page. The page heartbeats twice a second, which both paces the drain
/// and reports what it thinks it is doing — a playhead that does not
/// move has several indistinguishable causes (no element, no zoom yet,
/// an animation never built, a transport that simply is not running) and
/// this separates them in one line.
///
/// `RUST_LOG=daw_ui::studio::clock=debug`.
#[component]
pub fn ClockDriver(clock: Clock) -> Element {
    let mut eval = use_hook(|| document::eval(CLOCK_JS));
    use_future(move || {
        let clock = clock.clone();
        async move {
            loop {
                // Awaiting the page's heartbeat is what pumps the
                // channel; the drain rides along on it.
                let beat = eval.recv::<[f64; 6]>().await;
                let commands = clock.drain();
                // `clock-mute` keeps the driver and its heartbeat but
                // issues no command evals, to separate the cost of the
                // channel from the cost of merely having a clock.
                if use_hook(|| super::probe::Probes::from_env().has("clock-mute")) {
                    continue;
                }
                for [kind, value] in commands {
                    // One short eval per command. Commands are user
                    // actions — a seek, a zoom notch, a play — not
                    // per-frame traffic, so this is cheap and, unlike
                    // the channel, it arrives.
                    document::eval(&format!("window.__ftsClockCmd?.({kind}, {value});"));
                }
                match beat {
                    Ok([animated, position, pps, length, playing, received]) => {
                        tracing::debug!(
                            clock.animated = animated > 0.5,
                            clock.position_secs = position,
                            clock.pps = pps,
                            clock.length_secs = length,
                            clock.playing = playing > 0.5,
                            clock.commands_received = received,
                            "page clock"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = ?e, "page clock channel closed");
                        return;
                    }
                }
            }
        }
    });
    rsx! {}
}
