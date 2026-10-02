//! Write the painters' mixer strip out as themes.
//!
//! ```sh
//! cargo run -r -p session-daw --bin theme_export              # every layout, beside each other
//! cargo run -r -p session-daw --bin theme_export -- out.fm    # REAPER's alone, to out.fm
//! ```
//!
//! `apps/session-daw/themes/fts-dark.fm` is REAPER's layout to the pixel;
//! `fts-console.fm` the same controls laid out as a console strip. See
//! `session_daw::skin::export`.

fn main() {
    let palette = session_daw::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark());
    let themes: Vec<(String, String)> = match std::env::args().nth(1) {
        Some(out) => vec![(out, session_daw::skin::export::fts_dark(&palette))],
        None => vec![
            (
                "apps/session-daw/themes/fts-dark.fm".to_owned(),
                session_daw::skin::export::fts_dark(&palette),
            ),
            (
                "apps/session-daw/themes/fts-console.fm".to_owned(),
                session_daw::skin::export::fts_console(&palette),
            ),
        ],
    };
    for (out, markup) in themes {
        // Compiled before it is written: a theme the mixer would refuse is
        // not worth overwriting a good one with.
        if let Err(errors) =
            theme_runtime::Theme::from_markup(&markup, session_daw::skin::schema(), None)
        {
            eprintln!("{out}:");
            for e in errors.iter().take(20) {
                eprintln!("  {e}");
            }
            std::process::exit(1);
        }
        std::fs::write(&out, &markup).unwrap_or_else(|e| panic!("{out}: {e}"));
        println!("wrote {out} ({} lines)", markup.lines().count());
    }
}
