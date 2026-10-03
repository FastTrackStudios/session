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
    Mod(modulation_ui::viz::ModView),
    Sat(saturate_ui::viz::SatView),
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

const MOD_KNOBS: [Macro; 4] = [
    Macro {
        name: "Type",
        get: |t| {
            let all = modulation_ui::viz::Engine::ALL;
            let at = all
                .iter()
                .position(|e| *e == t.modulation.engine)
                .unwrap_or(0);
            crate::num::coord(at) / crate::num::coord(all.len().saturating_sub(1).max(1))
        },
        set: |t, f| {
            let all = modulation_ui::viz::Engine::ALL;
            let steps = crate::num::coord(all.len().saturating_sub(1));
            let at = crate::num::index((f.clamp(0.0, 1.0) * steps).round());
            if let Some(engine) = all.get(at) {
                t.modulation.engine = *engine;
            }
        },
        says: |t| format!("{:?}", t.modulation.engine),
    },
    Macro {
        name: "Rate",
        get: |t| frac(t.modulation.rate, 0.05, 10.0),
        set: |t, f| t.modulation.rate = span(f, 0.05, 10.0),
        says: |t| format!("{:.2} Hz", t.modulation.rate),
    },
    Macro {
        name: "Depth",
        get: |t| f64::from(t.modulation.depth.clamp(0.0, 1.0)),
        set: |t, f| t.modulation.depth = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.modulation.depth * 100.0),
    },
    Macro {
        name: "Mix",
        get: |t| f64::from(t.modulation.mix.clamp(0.0, 1.0)),
        set: |t, f| t.modulation.mix = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.modulation.mix * 100.0),
    },
];

/// The saturator's drive as its knob, 0..1: the gain back through the
/// profile's own drive scale (`Tone::set_sat_profile`'s rule).
fn drive_knob(tone: &Tone) -> f32 {
    let scale = saturate_profiles::PROFILES
        .get(tone.sat_profile)
        .map_or(1.0, |p| p.voicing.drive_scale)
        .max(f32::EPSILON);
    ((tone.sat.drive - 1.0) / (15.0 * scale)).clamp(0.0, 1.0)
}

const SAT_KNOBS: [Macro; 3] = [
    Macro {
        name: "Type",
        get: |t| {
            let last = saturate_profiles::PROFILES.len().saturating_sub(1).max(1);
            crate::num::coord(t.sat_profile) / crate::num::coord(last)
        },
        set: |t, f| {
            let last = crate::num::coord(saturate_profiles::PROFILES.len().saturating_sub(1));
            let at = crate::num::index((f.clamp(0.0, 1.0) * last).round());
            if at != t.sat_profile {
                t.set_sat_profile(at);
            }
        },
        says: |t| {
            saturate_profiles::PROFILES
                .get(t.sat_profile)
                .map_or_else(String::new, |p| p.name.to_owned())
        },
    },
    Macro {
        name: "Drive",
        get: |t| f64::from(drive_knob(t)),
        set: |t, f| {
            let scale = saturate_profiles::PROFILES
                .get(t.sat_profile)
                .map_or(1.0, |p| p.voicing.drive_scale);
            t.sat.drive = (15.0 * scale).mul_add(span(f, 0.0, 1.0), 1.0);
        },
        says: |t| format!("{:.0}%", drive_knob(t) * 100.0),
    },
    Macro {
        name: "Mix",
        get: |t| f64::from(t.sat.mix.clamp(0.0, 1.0)),
        set: |t, f| t.sat.mix = span(f, 0.0, 1.0),
        says: |t| format!("{:.0}%", t.sat.mix * 100.0),
    },
];

/// What the saturation editor's picture draws, for `tone`: its profile
/// voiced at this drive and mix.
fn sat_view(tone: &Tone) -> Option<saturate_ui::viz::SatView> {
    let profile = *saturate_profiles::PROFILES.get(tone.sat_profile)?;
    let controls = saturate_profiles::Controls {
        drive: drive_knob(tone),
        mix: tone.sat.mix,
        ..saturate_profiles::Controls::default()
    };
    Some(saturate_ui::viz::SatView::of(profile, controls))
}

/// What the modulation editor's picture draws, for `tone`.
#[must_use]
pub const fn mod_view(tone: &Tone) -> modulation_ui::viz::ModView {
    let wobble = tone.modulation;
    modulation_ui::viz::ModView {
        engine: wobble.engine,
        rate: wobble.rate,
        depth: wobble.depth,
        mix: wobble.mix,
        on: true,
        // Modulation is cyan-led and motion pink-led, as its editor has it.
        color: match wobble.engine.group() {
            modulation_ui::viz::Group::Colouring => [34, 211, 238],
            modulation_ui::viz::Group::Moving => [244, 114, 182],
        },
        time: 0.0,
    }
}

/// The modulation's vector picture in `at` — the rack's, at a strip's
/// width, where there is no GPU surface to hand it.
pub fn paint_mod_in(scene: &mut Scene, tone: &Tone, at: Panel) {
    if at.width < 2.0 || at.height < 2.0 {
        return;
    }
    let mut picture = Scene::new();
    modulation_ui::viz::paint_mod(&mut picture, &mod_view(tone), at.width, at.height);
    let place = Affine::translate((at.x, at.y));
    scene.push_clip_layer(place, &Rect::new(0.0, 0.0, at.width, at.height));
    scene.append_scene(picture, place);
    scene.pop_layer();
}

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
        Which::Sat => sat_view(tone).map(|view| Face {
            name: saturate_profiles::PROFILES
                .get(tone.sat_profile)
                .map_or_else(String::new, |p| p.name.to_owned()),
            picture: Picture::Sat(view),
            knobs: &SAT_KNOBS,
        }),
        Which::Mod => Some(Face {
            name: format!("{:?}", tone.modulation.engine),
            picture: Picture::Mod(mod_view(tone)),
            knobs: &MOD_KNOBS,
        }),
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
    mods: std::collections::HashMap<
        String,
        (
            modulation_ui::viz::Shared<modulation_ui::viz::ModView>,
            modulation_ui::viz::ModWidget,
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
                Picture::Mod(view) => modulation_ui::viz::paint_mod(&mut scene, view, w, h),
                Picture::Sat(view) => saturate_ui::viz::paint_sat(&mut scene, view, w, h),
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
            // No widget of its own: its picture is measured, not
            // animated, so the vector painter is the whole of it.
            Picture::Sat(view) => {
                let mut scene = Scene::new();
                let (w, h) = (f64::from(width), f64::from(height));
                saturate_ui::viz::paint_sat(&mut scene, view, w, h);
                scene
            }
            Picture::Mod(view) => {
                let (shared, widget) = self.mods.entry(key.to_owned()).or_insert_with(|| {
                    let shared = std::rc::Rc::new(std::cell::RefCell::new(view.clone()));
                    let mut widget =
                        modulation_ui::viz::ModWidget::new(std::rc::Rc::clone(&shared));
                    widget.create_surfaces(ctx);
                    (shared, widget)
                });
                shared.replace(view.clone());
                widget.paint_frame(ctx, width, height, 1.0)
            }
        }
    }
}
