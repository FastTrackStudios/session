//! Which theme the window opens on.
//!
//! The studio draws every colour from `daw_ui::theming::Theme`
//! (see `daw_ui::studio::theme`), and that type is the target of
//! `daw_ui::theming::reaper_import` — so a real `.ReaperTheme` is not a
//! skin bolted on afterwards, it *is* the theme. This resolves which one
//! to load, in the order a DAW should:
//!
//! 1. `FTS_REAPER_THEME` — a theme directory or a `.ReaperTheme` file.
//! 2. The user's own themes, where REAPER keeps them.
//! 3. The REAPER 7 default that ships in-tree, so the window looks like
//!    REAPER with nothing configured.
//! 4. The FTS dark tokens, if none of the above is readable.
//!
//! **Images are optional.** `ReaperTheme::load_dir` insists on the image
//! folder, because the components that blit a theme's PNGs need it. This
//! window draws in CSS, so it wants only the `[color theme]` palette and
//! `rtconfig.txt` — which is what `from_sources` parses. Requiring the
//! images meant the in-tree REAPER 7 default, which ships the ini and
//! rtconfig without them, silently fell through to the FTS greys.
//!
//! **`.ReaperThemeZip` is not read.** REAPER ships and installs themes
//! packed, so a user's own theme usually IS a zip; unpacking one needs a
//! zip dependency this workspace does not carry yet. Until then, unpack
//! the theme and point `FTS_REAPER_THEME` at it.
//!
//! A theme that will not load is a warning and a fallback, never a
//! failed launch: a window you cannot open tells you less about a broken
//! theme than a window that opens in the wrong colours.

use std::path::{Path, PathBuf};

use daw_ui::theming::{Theme, ThemeContext, reaper_import};

/// Point the window at an unpacked `.ReaperTheme` directory.
const THEME_ENV: &str = "FTS_REAPER_THEME";

/// The REAPER 7 default, unpacked in-tree. Resolved at compile time
/// against this file, so it works from any working directory.
const BUNDLED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../features/ui/daw-ui/examples/assets/reaper7"
);

/// The theme context the window is mounted under.
pub fn resolve() -> ThemeContext {
    let theme = candidates()
        .into_iter()
        .find_map(|dir| load(&dir))
        .unwrap_or_else(|| {
            tracing::warn!("no REAPER theme loaded; falling back to the FTS dark tokens");
            Theme::dark()
        });
    ThemeContext::new().with_theme(theme)
}

fn candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(from_env) = std::env::var(THEME_ENV) {
        dirs.push(PathBuf::from(from_env));
    }
    // REAPER keeps unpacked themes here; a packed `.ReaperThemeZip` is
    // not a directory and is skipped rather than unpacked behind the
    // user's back.
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".config/REAPER/ColorThemes"));
    }
    dirs.push(PathBuf::from(BUNDLED));
    dirs
}

fn load(path: &Path) -> Option<Theme> {
    // A whole theme, images and all, when the layout is there.
    if path.is_dir()
        && let Ok(theme) = reaper_import::theme_from_dir(path)
    {
        tracing::info!(theme.path = %path.display(), theme.images = true, "reaper theme loaded");
        return Some(theme);
    }
    // Otherwise the palette alone, which is all this window draws from.
    let ini_path = if path.is_dir() {
        find_ini(path)?
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ReaperTheme"))
    {
        path.to_path_buf()
    } else {
        return None;
    };
    let ini = std::fs::read_to_string(&ini_path).ok()?;
    // `rtconfig.txt` carries the theme's `define_parameter` knobs. Its
    // absence costs the parameters, not the colours, so an empty one is a
    // fine stand-in rather than a reason to reject the theme.
    let rtconfig = find_rtconfig(&ini_path).unwrap_or_default();
    let theme = reaper_import::theme_from_reaper(&reaper_import::ReaperTheme::from_sources(
        &ini, &rtconfig,
    ));
    tracing::info!(
        theme.path = %ini_path.display(),
        theme.images = false,
        "reaper palette loaded"
    );
    Some(theme)
}

/// The first `.ReaperTheme` in a directory. REAPER allows exactly one per
/// theme folder, so "first" is "the one".
fn find_ini(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ReaperTheme"))
        })
}

/// The `rtconfig.txt` belonging to an ini — in the folder named after it,
/// or beside it.
fn find_rtconfig(ini: &Path) -> Option<String> {
    let beside = ini.with_extension("").join("rtconfig.txt");
    let sibling = ini.parent()?.join("rtconfig.txt");
    for candidate in [beside, sibling] {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    None
}
