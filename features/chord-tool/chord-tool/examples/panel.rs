//! Run the chord panel in a desktop window.
//!
//! ```sh
//! cargo run -p chord-tool --example panel
//! ```
//!
//! The iteration loop, not the shipping target. This uses Dioxus's
//! desktop (WebView) renderer, while the REAPER panel renders through
//! Blitz — so CSS support differs, and layout that relies on anything
//! exotic can look right here and wrong there. Keep styles simple and
//! check the REAPER panel before believing it.
fn main() {
    dioxus::LaunchBuilder::desktop().launch(chord_tool::ChordToolPanel);
}
