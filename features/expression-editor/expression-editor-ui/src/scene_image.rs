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
    let (pw, ph) = pixel_size(w, h, scale);
    let buffer = rasterize(scene, pw, ph, scale, background);
    encode(&buffer, pw, ph)
}

/// The image's size in device pixels.
fn pixel_size(w: f64, h: f64, scale: f64) -> (u32, u32) {
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

/// The scene, rasterized to RGBA8 — the expensive step, and the one
/// neither encoding nor transport can avoid.
fn rasterize(scene: &Scene, pw: u32, ph: u32, scale: f64, background: Color) -> Vec<u8> {
    render_to_buffer::<VelloCpuImageRenderer, _>(
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
    )
}

/// Rasterize `scene` and return BMP bytes.
///
/// BMP because encoding it is very nearly a memcpy: a 54-byte header and
/// the pixels, with rows in place. PNG cost 1.20 ms of a 4.85 ms frame at
/// stack size and buys nothing here — the consumer is the WebView in the
/// same process, decoding it again microseconds later, so compressing is
/// paying twice to save a copy that is never sent anywhere.
///
/// Every engine we target (WebKitGTK, WebView2, WKWebView) decodes BMP.
pub fn scene_bmp(scene: &Scene, w: f64, h: f64, scale: f64, background: Color) -> Vec<u8> {
    let (pw, ph) = pixel_size(w, h, scale);
    let rgba = rasterize(scene, pw, ph, scale, background);
    bmp(&rgba, pw, ph)
}

/// RGBA8 to a 32-bit BMP.
///
/// Top-down via a negative height, so the rows go out in the order the
/// rasterizer produced them rather than being reversed. 32bpp means every
/// row is already 4-byte aligned, so there is no padding to insert.
fn bmp(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    const FILE_HEADER: u32 = 14;
    const INFO_HEADER: u32 = 40;
    let pixels = w * h * 4;
    let size = FILE_HEADER + INFO_HEADER + pixels;
    let mut out = Vec::with_capacity(size as usize);

    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    out.extend_from_slice(&(FILE_HEADER + INFO_HEADER).to_le_bytes());

    out.extend_from_slice(&INFO_HEADER.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // negative = top-down
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, uncompressed
    out.extend_from_slice(&pixels.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi, x
    out.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi, y
    out.extend_from_slice(&0u32.to_le_bytes()); // palette colours
    out.extend_from_slice(&0u32.to_le_bytes()); // important colours

    // RGBA to BGRA. The alpha byte rides along untouched; the scene is
    // painted over an opaque ground, so it is 255 throughout.
    out.extend(rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]));
    out
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

/// A painted surface served to the WebView as an ordinary image URL.
///
/// The pixels do not travel through the DOM. A `data:` URI puts the whole
/// image in an attribute — 1.5 MB at stack size, 6.6 MB on a HiDPI window
/// — which dioxus then diffs as a string, ships over the IPC bridge and
/// the engine parses back out of the markup, every frame. Serving the
/// same bytes from a handler makes the attribute a short URL and lets the
/// WebView fetch the image the way it fetches any other, binary and
/// direct.
///
/// The URL carries a revision that changes with the picture, because a
/// stable URL is a cached image: the engine would never ask again.
#[cfg(all(feature = "webview", not(target_arch = "wasm32")))]
pub mod served {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The latest bytes for one surface, and how many times they have
    /// changed.
    #[derive(Clone, Default)]
    pub struct Surface(Rc<RefCell<(u64, Vec<u8>)>>);

    impl Surface {
        pub fn new() -> Self {
            Self::default()
        }

        /// Publish a new frame. Returns the revision to put in the URL.
        pub fn put(&self, bytes: Vec<u8>) -> u64 {
            let mut held = self.0.borrow_mut();
            held.0 += 1;
            held.1 = bytes;
            held.0
        }

        /// The current revision, without publishing.
        pub fn revision(&self) -> u64 {
            self.0.borrow().0
        }

        /// The bytes for `revision`, if that is still the current frame.
        ///
        /// A request for an older revision is answered `None`: the image
        /// it wanted has already been replaced, and the element pointing
        /// at it has been told about the new one.
        pub fn get(&self, revision: u64) -> Option<Vec<u8>> {
            let held = self.0.borrow();
            (held.0 == revision).then(|| held.1.clone())
        }
    }
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
    fn a_scene_becomes_a_bmp_a_browser_can_decode() {
        let mut scene = Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(0xff, 0x00, 0x00),
            None,
            &Rect::new(0.0, 0.0, 4.0, 2.0),
        );
        let bytes = scene_bmp(&scene, 4.0, 2.0, 1.0, Color::BLACK);
        assert_eq!(&bytes[0..2], b"BM");
        // 54-byte header plus 4 bytes a pixel, and nothing else.
        assert_eq!(bytes.len(), 54 + 4 * 4 * 2);
        assert_eq!(
            u32::from_le_bytes(bytes[2..6].try_into().unwrap()) as usize,
            bytes.len()
        );
        // Top-down: the height is stored negative.
        assert_eq!(i32::from_le_bytes(bytes[22..26].try_into().unwrap()), -2);
        // The first pixel is the red fill, in BGRA order.
        assert_eq!(&bytes[54..58], &[0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn an_empty_scene_still_produces_its_background() {
        let uri = scene_data_uri(&Scene::new(), 4.0, 4.0, 1.0, Color::BLACK);
        assert!(uri.starts_with("data:image/png;base64,"));
    }
}
