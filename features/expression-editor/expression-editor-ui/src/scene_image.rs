//! A painted scene, presented to a renderer that cannot replay one.
//!
//! [`crate::paint`] builds an [`anyrender::Scene`] — a backend-agnostic
//! recording, `Vec<RenderCommand>` of fills, strokes and glyph runs. Blitz
//! replays it directly through a custom widget and that is the fast path.
//! A WebView cannot: it has no seam for handing a native renderer a
//! display list, and panics outright on the attribute that carries one.
//!
//! So the scene is rasterized here and handed over as an image. The
//! drawing code does not change and does not fork — `paint.rs` stays the
//! single description of what these surfaces look like, and this is only
//! how the pixels arrive:
//!
//! | target | seam |
//! |---|---|
//! | dioxus-native | `Widget::paint` replays the scene (GPU) |
//! | dioxus-desktop | this, rasterized natively |
//! | dioxus-web | this, rasterized in wasm |
//!
//! `vello_cpu` is pure Rust with no platform I/O, so the same call works
//! natively and in wasm — which is what lets one seam serve both webview
//! targets.
//!
//! **This is a demo of the approach, not a finished path.** Encoding a
//! PNG and base64ing it into an attribute is the portable way to move
//! pixels into a DOM without any JavaScript, and it is honestly the
//! slowest thing here. What it buys is a measurement: if a pane is
//! comfortable at this cost it needs nothing more, and if it is not, the
//! escalation (a canvas fed raw RGBA, or vello_hybrid's WebGL renderer on
//! web) is a change to this file alone.

use anyrender::{PaintScene, Scene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use kurbo::{Affine, Rect};
use peniko::{Color, Fill};

/// Rasterize `scene` at `w` x `h` CSS pixels and return a `data:` URI.
///
/// `scale` is the device pixel ratio: the image is rendered at that
/// multiple and displayed at the CSS size, so the result is sharp on a
/// HiDPI display rather than a scaled-up blur.
pub fn scene_data_uri(scene: &Scene, w: f64, h: f64, scale: f64, background: Color) -> String {
    let (pw, ph) = (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    );
    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |painter: &mut <VelloCpuImageRenderer as anyrender::ImageRenderer>::ScenePainter<'_>| {
            // Opaque ground: the scene paints its own background, but an
            // image with transparent edges would let the page show
            // through wherever it did not.
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                background,
                None,
                &Rect::new(0.0, 0.0, pw as f64, ph as f64),
            );
            // The recording, replayed. `append_scene` is what Blitz's own
            // painter does with a widget's scene, so both targets execute
            // exactly the same command list.
            painter.append_scene(scene.clone(), Affine::scale(scale));
        },
        pw,
        ph,
    );
    encode(&buffer, pw, ph)
}

/// RGBA8 to a `data:image/png;base64,…` URI.
///
/// `Fast` compression on purpose. This runs per frame, and a PNG is
/// being decoded again on the other side of the attribute a moment later;
/// spending milliseconds to make it smaller is the wrong trade when the
/// consumer is in the same process.
fn encode(rgba: &[u8], w: u32, h: u32) -> String {
    use base64::Engine as _;
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let Ok(mut writer) = encoder.write_header() else {
            return String::new();
        };
        if writer.write_image_data(rgba).is_err() {
            return String::new();
        }
    }
    let mut uri = String::with_capacity(png.len() * 4 / 3 + 32);
    uri.push_str("data:image/png;base64,");
    base64::engine::general_purpose::STANDARD.encode_string(&png, &mut uri);
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_becomes_a_png_data_uri() {
        let mut scene = Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(0xff, 0x00, 0x00),
            None,
            &Rect::new(0.0, 0.0, 8.0, 8.0),
        );
        let uri = scene_data_uri(&scene, 8.0, 8.0, 1.0, Color::BLACK);
        assert!(uri.starts_with("data:image/png;base64,"));
        // Long enough to be an actual image rather than a header stub.
        assert!(uri.len() > 100, "suspiciously short: {} bytes", uri.len());
    }

    #[test]
    fn an_empty_scene_still_produces_its_background() {
        let uri = scene_data_uri(&Scene::new(), 4.0, 4.0, 1.0, Color::BLACK);
        assert!(uri.starts_with("data:image/png;base64,"));
    }
}
