//! The Session app's panels under Blitz, rendered to a picture.
//!
//! The app is moving from the WRY webview onto Blitz (`docs/app-on-blitz.md`),
//! and the performance panels were written against a browser: Tailwind
//! classes, hovers, transitions. This draws them headlessly so what Blitz
//! makes of them can be looked at rather than guessed at.
//!
//! ```sh
//! cargo run -p session-daw --bin blitz_panels -- /tmp/panels.png
//! ```

use anyrender::ImageRenderer as _;
use anyrender_vello::VelloImageRenderer;
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use session_ui::components::progress::{ProgressSection, SongProgressBar};

/// The Session app's compiled Tailwind, as the WRY app inlines it.
const TAILWIND: &str = include_str!("../../../desktop/assets/tailwind-signal.css");

fn sections() -> Vec<ProgressSection> {
    let at = |from: f64, to: f64, name: &str, short: &str, color: &str| ProgressSection {
        start_percent: from,
        end_percent: to,
        color: color.to_owned(),
        name: name.to_owned(),
        short_name: short.to_owned(),
        comment: None,
    };
    vec![
        at(0.0, 8.0, "Count", "CT", "#6b21a8"),
        at(8.0, 20.0, "Intro", "IN", "#1d4ed8"),
        at(20.0, 42.0, "Verse 1", "VS1", "#15803d"),
        at(42.0, 55.0, "Pre Chorus", "PRE", "#a16207"),
        at(55.0, 75.0, "Chorus", "CH", "#b91c1c"),
        at(75.0, 90.0, "Bridge", "BR", "#0e7490"),
        at(90.0, 100.0, "Ending", "END", "#4b5563"),
    ]
}

#[component]
fn Panels(tailwind: bool) -> Element {
    rsx! {
        // A plain `<style>` in the tree: `document::Style` goes through the
        // window's head, and a headless document has no window.
        if tailwind {
            style { {TAILWIND} }
        }
        div {
            style: "width: 1600px; height: 400px; background: #111214; padding: 24px; \
                    display: flex; flex-direction: column; gap: 24px; color: #e5e7eb;",
            div { style: "font-size: 14px;", "SongProgressBar — 47%" }
            SongProgressBar { progress: 47.0, sections: sections(), song_key: Some("F".to_owned()) }
            div { style: "font-size: 14px;", "SongProgressBar — 5%" }
            SongProgressBar { progress: 5.0, sections: sections() }
        }
    }
}

fn render(tailwind: bool, out: &str) {
    let (w, h) = (1600u32, 400u32);
    let vdom = VirtualDom::new_with_props(Panels, PanelsProps { tailwind });
    let mut document = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(w, h, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    document.initial_build();
    for _ in 0..6 {
        document.poll(None);
        document.inner_mut().resolve(0.0);
    }
    let mut image = VelloImageRenderer::new(w, h);
    let mut buffer = Vec::new();
    image.render_to_vec(
        |painter| {
            let mut inner = document.inner_mut();
            blitz_paint::paint_scene(painter, &mut inner, 1.0, w, h, 0, 0);
        },
        &mut buffer,
    );
    let image = image::RgbaImage::from_raw(w, h, buffer).expect("a buffer the right size");
    image.save(out).expect("write the picture");
    println!("wrote {out}");
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/panels.png".to_owned());
    let bare = out.replace(".png", "-bare.png");
    render(true, &out);
    render(false, &bare);
}
