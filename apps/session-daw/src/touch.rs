//! Touch mode: controls sized for a finger, and gestures made for one.
//!
//! A mouse can hit a 21-pixel mute button and drag a fader from anywhere
//! on its groove without anyone noticing. A finger covers the button and
//! its neighbours, and a finger landing on a groove is far more often the
//! start of a scroll than a decision to move the level. So touch mode is
//! not the same controls made bigger: it is bigger controls, a fader that
//! only moves when its cap is taken, and empty space that scrolls.
//!
//! On by default where the screen is the pointer (iOS, Android, a browser
//! whose primary pointer is coarse), and a switch everywhere, since a
//! touchscreen laptop is both.

use dioxus::prelude::*;

/// How much bigger everything is in touch mode. REAPER's mute is 21 by
/// 20; at this it is 37 by 35, the far side of the smallest target a
/// fingertip hits without looking.
pub const ZOOM: f64 = 1.75;

/// How much bigger the arrangement is drawn in touch mode: its track
/// panel's 21-pixel buttons to 28, its ruler's 15-pixel lanes to 20, its
/// nine-point names to twelve. Less than the mixer's [`ZOOM`]: the
/// arrangement is mostly lanes, and every pixel of chrome it gains is one
/// the music loses.
pub const ARRANGE_ZOOM: f64 = 1.35;

/// How much bigger to draw the arrangement, as touch mode is, in a panel
/// `width` CSS pixels wide (0 while it is not yet measured).
///
/// A phone held upright is some four hundred wide, and the full zoom would
/// give its track panel more of that than its lanes; there the zoom eases
/// down, reaching the full [`ARRANGE_ZOOM`] at a tablet's width.
#[must_use]
pub fn arrange_zoom(touch: bool, width: f64) -> f64 {
    if !touch {
        return 1.0;
    }
    if width <= 0.0 {
        return ARRANGE_ZOOM;
    }
    let narrow = 1.1;
    let t = ((width - 400.0) / (700.0 - 400.0)).clamp(0.0, 1.0);
    narrow + (ARRANGE_ZOOM - narrow) * t
}

/// How much bigger to draw the mixer's strips, as touch mode is, in a
/// panel `width` CSS pixels wide: [`ZOOM`] on a tablet, easing down to
/// 1.35 on an upright phone, where the full zoom fit two and a half
/// strips across the screen.
#[must_use]
pub fn mixer_zoom(touch: bool, width: f64) -> f64 {
    if !touch {
        return 1.0;
    }
    let narrow = 1.35;
    let t = ((width - 400.0) / (700.0 - 400.0)).clamp(0.0, 1.0);
    narrow + (ZOOM - narrow) * t
}

/// A scroll still moving after the finger let go: the speed it left at,
/// dying away as a thrown list does on a phone.
#[derive(Clone, Copy, Debug)]
pub struct Fling {
    /// CSS pixels a millisecond, on each axis, in the scroll's direction.
    pub velocity: (f64, f64),
    at: web_time::Instant,
}

/// How fast a fling dies: the time for its speed to fall to a third,
/// near enough what iOS's own scroll views take.
const FLING_DECAY_MS: f64 = 325.0;
/// Below this (CSS pixels a millisecond) a fling has stopped.
const FLING_REST: f64 = 0.02;
/// The fastest a fling starts, however hard the flick.
const FLING_MAX: f64 = 6.0;

impl Fling {
    /// A fling at `velocity` (CSS pixels a millisecond, in the scroll's
    /// direction), or none if that is too slow to be one.
    #[must_use]
    pub fn thrown(velocity: (f64, f64)) -> Option<Self> {
        let velocity = (
            velocity.0.clamp(-FLING_MAX, FLING_MAX),
            velocity.1.clamp(-FLING_MAX, FLING_MAX),
        );
        (velocity.0.hypot(velocity.1) > FLING_REST * 5.0).then(|| Self {
            velocity,
            at: web_time::Instant::now(),
        })
    }

    /// How far it has carried since it was last asked, and whether it is
    /// still going.
    pub fn step(&mut self) -> ((f64, f64), bool) {
        let now = web_time::Instant::now();
        let dt = now.duration_since(self.at).as_secs_f64() * 1000.0;
        self.at = now;
        // The distance under an exponentially dying speed over `dt`.
        let decay = (-dt / FLING_DECAY_MS).exp();
        let carried = FLING_DECAY_MS * (1.0 - decay);
        let moved = (self.velocity.0 * carried, self.velocity.1 * carried);
        self.velocity = (self.velocity.0 * decay, self.velocity.1 * decay);
        (moved, self.velocity.0.hypot(self.velocity.1) > FLING_REST)
    }
}

/// A way to ask for another frame, for a widget still moving with nothing
/// to prompt a paint (a fling): the window's `request_redraw` under
/// Blitz, which paints a widget only after an event; nothing on a page,
/// which paints every frame anyway. Read from context, so call it where a
/// context can be read (a component, or a hook's closure).
#[must_use]
pub fn redraw_hook() -> Option<std::rc::Rc<dyn Fn()>> {
    #[cfg(feature = "native")]
    {
        let window = try_consume_context::<std::sync::Arc<dyn winit::window::Window>>()?;
        Some(std::rc::Rc::new(move || window.request_redraw()))
    }
    #[cfg(not(feature = "native"))]
    None
}

/// A finger's speed, from its last few moves: CSS pixels a millisecond on
/// each axis. Kept short, so a finger that slowed before letting go
/// throws gently.
#[derive(Clone, Debug, Default)]
pub struct Speed {
    samples: Vec<(web_time::Instant, (f64, f64))>,
}

impl Speed {
    /// The finger is at `at`, now.
    pub fn moved(&mut self, at: (f64, f64)) {
        let now = web_time::Instant::now();
        self.samples.push((now, at));
        self.samples
            .retain(|(when, _)| now.duration_since(*when).as_millis() <= 100);
    }

    /// Its speed over the samples kept.
    #[must_use]
    pub fn velocity(&self) -> (f64, f64) {
        let (Some(first), Some(last)) = (self.samples.first(), self.samples.last()) else {
            return (0.0, 0.0);
        };
        let ms = last.0.duration_since(first.0).as_secs_f64() * 1000.0;
        if ms < 1.0 {
            return (0.0, 0.0);
        }
        ((last.1.0 - first.1.0) / ms, (last.1.1 - first.1.1) / ms)
    }
}

/// How far a finger may wander, in CSS pixels, and still be a tap. Past
/// it the press is a scroll.
pub const SLOP: f64 = 10.0;

/// Whether touch mode is on: a context the shell provides.
#[derive(Clone, Copy, PartialEq)]
pub struct Touch(pub Signal<bool>);

impl Touch {
    /// Touch mode as the device suggests: on where the screen is the
    /// pointer.
    #[must_use]
    pub fn detect() -> Self {
        Self(Signal::new(detected()))
    }
}

/// Whether touch mode is on here. Off with no [`Touch`] provided.
#[must_use]
pub fn use_touch() -> bool {
    try_use_context::<Touch>().is_some_and(|touch| (touch.0)())
}

/// Whether the device's own pointer is a finger.
#[must_use]
pub fn detected() -> bool {
    if cfg!(any(target_os = "ios", target_os = "android")) {
        return true;
    }
    coarse_pointer()
}

#[cfg(feature = "web")]
fn coarse_pointer() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(pointer: coarse)").ok().flatten())
        .is_some_and(|query| query.matches())
}

#[cfg(not(feature = "web"))]
const fn coarse_pointer() -> bool {
    false
}
