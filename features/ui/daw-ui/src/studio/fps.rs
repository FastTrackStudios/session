//! What "frames per second" means here, and the only honest way to get it.
//!
//! Nothing in this crate paints — the browser engine does — so the only
//! number that matches what the eye sees is the rate its own compositor
//! presented at. `requestAnimationFrame` fires once per composited
//! frame, which is that. A count of dioxus renders, of DOM mutations, or
//! of engine ticks is a different quantity, and labelling one of those
//! "fps" is how a UI comes to be believed fast while feeling slow.
//!
//! Averaged in the page and reported twice a second: a bridge crossing
//! per frame would be a measurable share of the thing being measured,
//! and the interesting quantity over a gesture is the sustained rate
//! anyway. The **worst** frame in each window is reported beside the
//! mean, because a scroll that stutters averages beautifully and feels
//! terrible.
//!
//! It goes to `tracing`, not just to the toolbar, so a rate can be read
//! off a terminal instead of a screenshot:
//!
//! ```sh
//! RUST_LOG=daw_ui::studio::fps=info
//! ```

use crate::prelude::*;

const METER_JS: &str = r#"
let windowStart = performance.now();
let last = windowStart;
let frames = 0;
let worst = 0;
const tick = (now) => {
  const delta = now - last;
  last = now;
  frames += 1;
  if (delta > worst) worst = delta;
  const elapsed = now - windowStart;
  if (elapsed >= 500) {
    dioxus.send([(frames * 1000) / elapsed, worst]);
    windowStart = now;
    frames = 0;
    worst = 0;
  }
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);
"#;

/// The most recent reading, for anything that wants to show it.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct FrameRate {
    pub fps: f64,
    pub worst_frame_ms: f64,
}

/// Reports the window's real frame rate into `rate` and to the log.
///
/// Renders nothing; mounted once near the root.
#[component]
pub fn FrameMeter(rate: Signal<FrameRate>) -> Element {
    let mut eval = use_hook(|| document::eval(METER_JS));
    use_future(move || {
        let mut rate = rate;
        async move {
            while let Ok([fps, worst_frame_ms]) = eval.recv::<[f64; 2]>().await {
                tracing::info!(
                    ui.fps = fps,
                    ui.worst_frame_ms = worst_frame_ms,
                    "studio frame rate"
                );
                rate.set(FrameRate {
                    fps,
                    worst_frame_ms,
                });
            }
        }
    });
    rsx! {}
}

/// The readout in the transport bar.
///
/// Its own component so a reading twice a second re-renders one span
/// rather than the bar it sits in.
#[component]
pub fn FrameReadout(rate: Signal<FrameRate>) -> Element {
    let r = rate();
    if r.fps <= 0.0 {
        return rsx! {};
    }
    rsx! {
        span {
            class: "studio-stat",
            "data-testid": "studio-fps",
            "{r.fps:.0} fps · worst {r.worst_frame_ms:.1} ms"
        }
    }
}
