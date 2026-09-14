//! A scroll the window drives itself, for measuring.
//!
//! The developer's own window is a Wayland surface: `xdotool` cannot see
//! it and the compositor refuses screenshots, so there is no way to drive
//! a gesture into it from outside. The usual answer is a private Xvfb —
//! but Xvfb rasterises in SOFTWARE, and a track panel is ~150 nodes of
//! SVG, so the one thing that rig makes expensive is exactly the thing
//! this window does most of. Numbers from there are a floor.
//!
//! So the page scrolls itself. `scrollBy` on the real window exercises
//! the same style, layout, paint and compositing that a wheel does —
//! minus the input plumbing, which was never the interesting part — and
//! it runs on the developer's GPU where the answer matters.
//!
//! Opt-in, because a window that scrolls on its own is not a window
//! anyone wants:
//!
//! ```sh
//! FTS_STUDIO_AUTOSCROLL=v just daw    # down the tracks
//! FTS_STUDIO_AUTOSCROLL=h just daw    # across the timeline
//! just daw-fps
//! ```

use crate::prelude::*;

/// Set to run the sweep: `v` vertical only, `h` horizontal only,
/// anything else both.
///
/// One axis at a time, because they are different work: down the tracks
/// realises rows, across the timeline repaints a surface many screens
/// wide. A run that mixes them reports one number for two problems.
pub const ENV: &str = "FTS_STUDIO_AUTOSCROLL";

/// Scrolls the arrangement while the frame meter watches. Renders
/// nothing; mounted only when [`ENV`] is set.
#[component]
pub fn AutoScroll(axis: String) -> Element {
    use_hook(move || document::eval(&SWEEP.replace("__AXIS__", &axis)));
    rsx! {}
}

const SWEEP: &str = r"
const AXIS = '__AXIS__';
const pane = () => document.querySelector('.studio-scroll');
const LEG_MS = 8000;
let t0 = 0;

const step = (now) => {
    const el = pane();
    // Waits for the project: scrolling an empty pane measures nothing,
    // and the arrangement arrives seconds after the window does.
    if (!el || el.scrollHeight <= el.clientHeight) {
        requestAnimationFrame(step);
        return;
    }
    if (!t0) { t0 = now; }
    let elapsed = now - t0;
    // A triangle wave, so the pane sweeps the whole session and
    // reverses — a direction change is where a scroller that realises
    // content on demand hurts most.
    const wave = (ms) => { const t = (ms / LEG_MS) * 2; return t < 1 ? t : 2 - t; };

    if (AXIS === 'v') {
        if (elapsed > LEG_MS) { t0 = now; elapsed = 0; }
        el.scrollTop = (el.scrollHeight - el.clientHeight) * wave(elapsed);
    } else if (AXIS === 'h') {
        if (elapsed > LEG_MS) { t0 = now; elapsed = 0; }
        el.scrollLeft = (el.scrollWidth - el.clientWidth) * wave(elapsed);
    } else if (elapsed < LEG_MS) {
        el.scrollTop = (el.scrollHeight - el.clientHeight) * wave(elapsed);
    } else if (elapsed < LEG_MS * 2) {
        el.scrollLeft = (el.scrollWidth - el.clientWidth) * wave(elapsed - LEG_MS);
    } else {
        t0 = now;
    }
    requestAnimationFrame(step);
};
requestAnimationFrame(step);
";
