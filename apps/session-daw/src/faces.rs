//! Devices with faces of their own: the effect's painted picture over a
//! row of its controls.
//!
//! The rack draws a reverb as its tail and a delay as its repeats, at a
//! strip's width. In the FX view a device has an editor's room, and the
//! effect's own editor already paints what it does — `reverb_ui`'s
//! halls, plates, springs and rooms, `delay_ui`'s tape, analog and
//! digital repeats — so the device draws that, from this track's
//! settings, with the controls that move it underneath.

use anyrender::{PaintScene, Scene};
use vello::kurbo::{Affine, Rect};

use crate::arrangement::Palette;
use crate::macros::{Macro, frac, span};
use crate::text::Font;
use crate::tone::{Panel, Tone, Which};

/// What a face paints.
#[derive(Clone)]
pub enum Picture {
    Reverb(reverb_ui::viz::ReverbView),
    Delay(delay_ui::viz::DelayView),
}

/// One device's face: what it is called, its picture, its controls.
pub struct Face {
    /// The machine, as its editor names it: Plate, Hall, Tape…
    pub name: String,
    pub picture: Picture,
    pub knobs: &'static [Macro],
}

/// How tall the face's name is.
const HEAD_H: f64 = 14.0;

/// Seconds per beat the pictures are measured against, until the
/// device knows the song's tempo: 120 bpm.
const BEAT: f32 = 0.5;

const REVERB_KNOBS: [Macro; 5] = [
    Macro {
        name: "Decay",
        get: |t| frac(t.reverb.decay, 0.1, 10.0),
        set: |t, f| t.reverb.decay = span(f, 0.1, 10.0),
        says: |t| format!("{:.1} s", t.reverb.decay),
    },
    Macro {
        name: "Pre",
        get: |t| frac(t.reverb.predelay, 0.0, 200.0),
        set: |t, f| t.reverb.predelay = span(f, 0.0, 200.0),
        says: |t| format!("{:.0} ms", t.reverb.predelay),
    },
    Macro {
        name: "Size",
        get: |t| f64::from(t.reverb.size.clamp(0.0, 1.0)),
        set: |t, f| t.reverb.size = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.reverb.size * 100.0),
    },
    Macro {
        name: "Damp",
        get: |t| f64::from(t.reverb.damping.clamp(0.0, 1.0)),
        set: |t, f| t.reverb.damping = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.reverb.damping * 100.0),
    },
    Macro {
        name: "Mix",
        get: |t| f64::from(t.reverb.mix.clamp(0.0, 1.0)),
        set: |t, f| t.reverb.mix = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.reverb.mix * 100.0),
    },
];

const DELAY_KNOBS: [Macro; 5] = [
    Macro {
        name: "Time",
        get: |t| frac(t.delay.time, 20.0, 2000.0),
        set: |t, f| t.delay.time = span(f, 20.0, 2000.0),
        says: |t| format!("{:.0} ms", t.delay.time),
    },
    Macro {
        name: "Repeats",
        get: |t| frac(t.delay.feedback, 0.0, 0.95),
        set: |t, f| t.delay.feedback = span(f, 0.0, 0.95),
        says: |t| format!("{:.0}%", t.delay.feedback * 100.0),
    },
    Macro {
        name: "Tone",
        get: |t| f64::from(t.delay.tone.clamp(0.0, 1.0)),
        set: |t, f| t.delay.tone = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.delay.tone * 100.0),
    },
    Macro {
        name: "Width",
        get: |t| f64::from(t.delay.width.clamp(0.0, 1.0)),
        set: |t, f| t.delay.width = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.delay.width * 100.0),
    },
    Macro {
        name: "Mix",
        get: |t| f64::from(t.delay.mix.clamp(0.0, 1.0)),
        set: |t, f| t.delay.mix = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.delay.mix * 100.0),
    },
];

/// The repeats `tone`'s delay makes: every tap until it has faded under
/// a fiftieth, alternating sides as wide as the delay is.
fn taps(tone: &Tone) -> Vec<delay_ui::viz::Tap> {
    let echo = &tone.delay;
    let gap = (echo.time / 1000.0).max(0.001);
    let pan = echo.width.clamp(0.0, 1.0) * 0.8;
    std::iter::successors(Some((1.0_f32, 1.0_f32)), |(n, level)| {
        let next = level * echo.feedback.clamp(0.0, 0.95);
        (next > 0.02 && *n < 32.0).then_some((n + 1.0, next))
    })
    .map(|(n, level)| delay_ui::viz::Tap {
        at: gap * n,
        level,
        pan: if n % 2.0 < 1.0 { pan } else { -pan },
    })
    .collect()
}

/// `which`'s face over `tone`, if it has one.
#[must_use]
pub fn of(which: Which, tone: &Tone) -> Option<Face> {
    match which {
        Which::Reverb => {
            let room = &tone.reverb;
            let family = room.algorithm.family();
            Some(Face {
                name: format!("{family:?}"),
                picture: Picture::Reverb(reverb_ui::viz::ReverbView {
                    decay: room.decay,
                    density: room.diffusion,
                    damp: room.damping,
                    predelay: room.predelay / 1000.0,
                    mix: room.mix,
                    on: true,
                    beat: BEAT,
                    family,
                    color: [167, 139, 250],
                    time: 0.0,
                }),
                knobs: &REVERB_KNOBS,
            })
        }
        Which::Delay => {
            let family = tone.delay.style.family();
            let taps = taps(tone);
            let window = taps
                .last()
                .map_or(1.0, |last| last.at + tone.delay.time / 1000.0);
            Some(Face {
                name: format!("{family:?}"),
                picture: Picture::Delay(delay_ui::viz::DelayView {
                    taps,
                    window,
                    mix: 1.0,
                    on: true,
                    beat: BEAT,
                    division: String::new(),
                    family,
                    color: [96, 165, 250],
                    time: 0.0,
                }),
                knobs: &DELAY_KNOBS,
            })
        }
        _ => None,
    }
}

/// Where a face's controls sit in `panel`: a row along its bottom.
#[must_use]
pub fn controls(panel: Panel) -> Panel {
    Panel {
        x: panel.x,
        y: panel.y + panel.height - crate::macros::CELL_H,
        width: panel.width,
        height: crate::macros::CELL_H,
    }
}

/// Draw `face`'s ground and name in `panel` — its picture goes in
/// [`picture_at`], painted by [`Painters`]; its controls in [`controls`].
pub fn chrome(scene: &mut Scene, palette: &Palette, font: &Font, face: &Face, panel: Panel) {
    let ground = Rect::new(
        panel.x,
        panel.y,
        panel.x + panel.width,
        panel.y + panel.height,
    );
    scene.fill(
        vello::peniko::Fill::NonZero,
        Affine::IDENTITY,
        palette.row_a,
        None,
        &ground,
    );
    crate::tcp::glyphs(
        scene,
        font,
        palette.text_dim,
        &face.name,
        panel.x + 6.0,
        panel.y + 10.5,
        8.0,
    );
}

/// Where a face's picture sits in `panel`: between its name and its
/// controls.
#[must_use]
pub fn picture_at(panel: Panel) -> Panel {
    Panel {
        x: panel.x,
        y: panel.y + HEAD_H,
        width: panel.width,
        height: (panel.height - HEAD_H - crate::macros::CELL_H).max(0.0),
    }
}

/// The effects' own widgets, one per device, kept so each keeps its GPU
/// surface and its animation's clock: the shader where the renderer
/// hands over a device, the vector painter where it does not.
#[derive(Default)]
pub struct Painters {
    reverbs: std::collections::HashMap<
        String,
        (
            reverb_ui::viz::Shared<reverb_ui::viz::ReverbView>,
            reverb_ui::viz::ReverbWidget,
        ),
    >,
    delays: std::collections::HashMap<
        String,
        (
            delay_ui::viz::Shared<delay_ui::viz::DelayView>,
            delay_ui::viz::DelayWidget,
        ),
    >,
}

impl Painters {
    /// `picture`, `width` × `height` device pixels, as device `key`'s
    /// widget paints it — the vector painter when there is no `ctx`.
    pub fn paint(
        &mut self,
        ctx: Option<&mut dyn anyrender::RenderContext>,
        key: &str,
        picture: &Picture,
        (width, height): (u32, u32),
    ) -> Scene {
        let Some(ctx) = ctx else {
            let mut scene = Scene::new();
            let (w, h) = (f64::from(width), f64::from(height));
            match picture {
                Picture::Reverb(view) => reverb_ui::viz::paint_reverb(&mut scene, view, w, h),
                Picture::Delay(view) => delay_ui::viz::paint_delay(&mut scene, view, w, h),
            }
            return scene;
        };
        match picture {
            Picture::Reverb(view) => {
                let (shared, widget) = self.reverbs.entry(key.to_owned()).or_insert_with(|| {
                    let shared = std::rc::Rc::new(std::cell::RefCell::new(view.clone()));
                    let mut widget = reverb_ui::viz::ReverbWidget::new(std::rc::Rc::clone(&shared));
                    widget.create_surfaces(ctx);
                    (shared, widget)
                });
                shared.replace(view.clone());
                widget.paint_frame(ctx, width, height, 1.0)
            }
            Picture::Delay(view) => {
                let (shared, widget) = self.delays.entry(key.to_owned()).or_insert_with(|| {
                    let shared = std::rc::Rc::new(std::cell::RefCell::new(view.clone()));
                    let mut widget = delay_ui::viz::DelayWidget::new(std::rc::Rc::clone(&shared));
                    widget.create_surfaces(ctx);
                    (shared, widget)
                });
                shared.replace(view.clone());
                widget.paint_frame(ctx, width, height, 1.0)
            }
        }
    }
}
