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

/// How much bigger to draw, as [`use_touch`] says.
#[must_use]
pub fn zoom(touch: bool) -> f64 {
    if touch { ZOOM } else { 1.0 }
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
