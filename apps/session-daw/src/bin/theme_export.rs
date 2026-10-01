//! Write the painters' mixer strip out as a theme.
//!
//! ```sh
//! cargo run -r -p session-daw --bin theme_export -- apps/session-daw/themes/fts-dark.fm
//! ```
//!
//! See `session_daw::skin::export`.

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "apps/session-daw/themes/fts-dark.fm".to_owned());
    let palette = session_daw::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark());
    let markup = session_daw::skin::export::fts_dark(&palette);
    // Compiled before it is written: a theme the mixer would refuse is
    // not worth overwriting a good one with.
    if let Err(errors) =
        theme_runtime::Theme::from_markup(&markup, session_daw::skin::schema(), None)
    {
        for e in errors.iter().take(20) {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }
    std::fs::write(&out, &markup).unwrap_or_else(|e| panic!("{out}: {e}"));
    println!("wrote {out} ({} lines)", markup.lines().count());
}
