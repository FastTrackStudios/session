//! The Session DAW view in a browser: the web demo's bundle.
//!
//! Built with cargo and wasm-bindgen (`just web-daw`), served as static
//! files: `index.html`, the module, and the session's files under
//! `session/`. See `session_daw::web_host`.

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

#[component]
fn App() -> Element {
    rsx! {
        style { {TAILWIND} }
        session_daw::web_host::WebDemo {
            name: "Always On Time".to_owned(),
            rpp_url: "session/demo.RPP".to_owned(),
            chart_url: Some("session/demo.kf".to_owned()),
        }
    }
}
