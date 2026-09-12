//! The live pass, over the recorded one.
//!
//! Three things cannot be recorded with the strips: the control under
//! the pointer (it changes with the mouse), the meters (they change
//! with the audio), and the play cursor (it changes with the clock).
//! All three are the same shape of problem — something live over
//! something recorded — so they are one pass.
//!
//! Drawing them here costs a handful of shapes a frame. Re-recording
//! the mixer to show a hover would cost the whole mixer, which is the
//! thing the recorded-scene architecture exists to avoid: a hover is
//! the most frequent event a window gets.

use anyrender::PaintScene;
use daw_theme_art::geometry::mcp as g;
use daw_theme_art::mixer_controls::Interaction;
use daw_theme_art::paint::tcp as art;
use daw_proto::Track;
use vello::kurbo::Affine;

use crate::arrangement::Palette;
use crate::mcp::{Control, Mixer};
use crate::pointer::Spot;
use crate::text::Font;

/// Redraw one control in its hover or pressed state.
///
/// `transform` is the mixer's own — the same one the strips are
/// replayed under — so the overlay lands on top of the control it is
/// replacing rather than beside it.
pub fn control(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    mixer: &Mixer,
    track: &Track,
    spot: Spot,
    state: Interaction,
    rack_h: f64,
    transform: Affine,
) {
    let Some((left, width, height)) = mixer.strip_box(spot.row) else {
        return;
    };
    let columns = crate::mcp::Columns::at(left, width);
    let shape = daw_ui::controls::Collapse::at(crate::mcp::f64_to_f32(
        (height - rack_h).max(1.0),
    ));
    let band_bottom = f64::from(daw_theme_art::collapse::FX_SECTION)
        + rack_h
        + f64::from(shape.pan_band)
        + f64::from(shape.input_band);

    // Recorded in content space and replayed under the transform, so
    // the overlay records the same way and rides the same transform.
    let mut scene = anyrender::Scene::new();
    match spot.control {
        Control::Mute | Control::Solo => {
            let (label, on, lit) = if spot.control == Control::Mute {
                ("M", track.muted, crate::tcp::mute_lit(palette))
            } else {
                ("S", track.soloed, crate::tcp::solo_lit(palette))
            };
            let row = if spot.control == Control::Mute { 0.0 } else { 1.0 };
            crate::art::place(
                &mut scene,
                &art::gutter_button(&palette.chrome, label, on, lit, state),
                font,
                columns.column_x,
                band_bottom + f64::from(g::RECMON_FROM_ARM)
                    + row * (f64::from(g::BUTTON_H) + 1.0),
            );
        }
        Control::RecArm => {
            crate::art::place(
                &mut scene,
                &art::record_arm(
                    &palette.chrome,
                    crate::tcp::lit(palette).rec,
                    track.armed,
                    state,
                    art::Arm::Mixer,
                    crate::tcp::to_theme(palette.tcp_tint),
                ),
                font,
                columns.column_axis - f64::from(g::ARM_CELL_W) * 0.486,
                band_bottom + f64::from(g::ARM_OVERHANG) - f64::from(g::ARM_CELL_H),
            );
        }
        Control::Pan => {
            crate::art::place(
                &mut scene,
                &art::pan_knob(
                    &palette.chrome,
                    track.pan.clamp(-1.0, 1.0),
                    crate::tcp::to_theme(palette.pan),
                ),
                font,
                left + (width - f64::from(g::PAN_KNOB_W)) / 2.0,
                f64::from(daw_theme_art::collapse::FX_SECTION) + rack_h + 2.0,
            );
        }
        Control::Fx => {
            crate::art::place(
                &mut scene,
                &art::fx_pill(
                    &palette.chrome,
                    crate::tcp::lit(palette),
                    art::Chain::Empty,
                    state,
                ),
                font,
                left + 7.0,
                rack_h + f64::from(g::FX_PILL_TOP),
            );
        }
        // The fader, the name plate and the routing widget have no
        // hover cell in the traced art — REAPER does not light them
        // either. They are still hit-testable and still draggable; they
        // just do not change appearance under the pointer.
        Control::Volume | Control::Name | Control::Routing => {}
    }

    for command in &scene.commands {
        crate::arrangement::submit_command(painter, command, transform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_ui::studio::RowsRef;

    fn mixer() -> (Mixer, Palette, Font, Vec<Track>) {
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = Font::embedded().expect("the embedded font");
        let tracks: Vec<Track> = (0..4)
            .map(|i| Track {
                guid: format!("t{i}"),
                name: format!("Track {i}"),
                width: Some(133),
                ..Track::default()
            })
            .collect();
        let rows = RowsRef(std::sync::Arc::new(
            tracks.iter().cloned().map(|t| (t, 0)).collect(),
        ));
        let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(
            daw_ui::studio::Project::default(),
        ));
        let mixer = Mixer::build(
            &palette,
            &font,
            &project,
            &rows,
            600.0,
            crate::layout::Layout::default(),
            false,
        );
        (mixer, palette, font, tracks)
    }

    /// The overlay draws SOMETHING for the controls that have a hover
    /// cell — otherwise hovering would do nothing at all and look like
    /// a dead control.
    #[test]
    fn the_lit_controls_draw() {
        let (mixer, palette, font, tracks) = mixer();
        for control in [Control::Mute, Control::Solo, Control::RecArm, Control::Fx] {
            let mut scene = anyrender::Scene::new();
            control_for(&mut scene, &palette, &font, &mixer, &tracks, control);
            assert!(
                !scene.commands.is_empty(),
                "{control:?} drew nothing when hovered"
            );
        }
    }

    /// And nothing for the ones REAPER does not light either. They stay
    /// hit-testable and draggable; they just do not change under the
    /// pointer, and drawing a highlight REAPER has no art for would be
    /// inventing one.
    #[test]
    fn the_unlit_controls_draw_nothing() {
        let (mixer, palette, font, tracks) = mixer();
        for control in [Control::Volume, Control::Name, Control::Routing] {
            let mut scene = anyrender::Scene::new();
            control_for(&mut scene, &palette, &font, &mixer, &tracks, control);
            assert!(
                scene.commands.is_empty(),
                "{control:?} invented a highlight"
            );
        }
    }

    /// A row that is not there draws nothing rather than panicking —
    /// the pointer can outlive a project reload by a frame.
    #[test]
    fn a_stale_row_is_ignored() {
        let (mixer, palette, font, tracks) = mixer();
        let mut scene = anyrender::Scene::new();
        control(
            &mut scene,
            &palette,
            &font,
            &mixer,
            &tracks[0],
            Spot {
                row: 999,
                control: Control::Mute,
            },
            Interaction::Hover,
            0.0,
            Affine::IDENTITY,
        );
        assert!(scene.commands.is_empty());
    }

    fn control_for(
        scene: &mut anyrender::Scene,
        palette: &Palette,
        font: &Font,
        mixer: &Mixer,
        tracks: &[Track],
        which: Control,
    ) {
        control(
            scene,
            palette,
            font,
            mixer,
            &tracks[1],
            Spot {
                row: 1,
                control: which,
            },
            Interaction::Hover,
            0.0,
            Affine::IDENTITY,
        );
    }
}
