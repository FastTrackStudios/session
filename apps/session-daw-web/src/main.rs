//! The Session DAW view in a browser: the web demo's bundle.
//!
//! Built with cargo and wasm-bindgen (`just web-daw`), served as static
//! files: `index.html` and the module. The session comes from a Task share
//! link to its folder — `?share=<link>&project=<Song.RPP>&chart=<Song.kf>`
//! — whose documents open it and whose audio renditions stream its stems
//! from their proxies. Without a link it opens the copy bundled under
//! `session/`, silent. See `session_daw::web_host`.

use dioxus::prelude::*;

/// The compiled Tailwind the session-ui panels (the progress bar, the
/// transport buttons) are styled with — the desktop app's sheet.
const TAILWIND: &str = include_str!("../../desktop/assets/tailwind-signal.css");

fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        tracing_wasm::set_as_global_default_with_config(
            tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(tracing::Level::INFO)
                .build(),
        );
    }
    dioxus::launch(App);
}

/// What the page opens: the project and chart URLs, and where the audio
/// streams from.
#[derive(Clone)]
struct Source {
    name: String,
    rpp: String,
    chart: Option<String>,
    media: Option<String>,
}

/// The page's query, or the bundled copy.
fn source() -> Source {
    let query = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok());
    let param = |key: &str| query.as_ref().and_then(|q| q.get(key));
    if let (Some(share), Some(project)) = (param("share"), param("project")) {
        let share = share.trim_end_matches('/').to_owned();
        let doc = |name: &str| format!("{share}/doc/{}", js_sys::encode_uri_component(name));
        return Source {
            name: project
                .trim_end_matches(".RPP")
                .trim_end_matches(".rpp")
                .to_owned(),
            rpp: doc(&project),
            chart: param("chart").map(|c| doc(&c)),
            media: Some(share),
        };
    }
    Source {
        name: "Always On Time".to_owned(),
        rpp: "session/demo.RPP".to_owned(),
        chart: Some("session/demo.kf".to_owned()),
        media: None,
    }
}

#[component]
fn App() -> Element {
    let source = use_hook(source);
    rsx! {
        style { {TAILWIND} }
        session_daw::web_host::WebDemo {
            name: source.name.clone(),
            rpp_url: source.rpp.clone(),
            chart_url: source.chart.clone(),
            media: source.media.clone(),
        }
    }
}
