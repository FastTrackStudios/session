//! The songs worked on lately, and each song's alternatives.
//!
//! **Recent** is a list of songs (and setlists) in the order they were
//! last opened, kept beside the app's other memories
//! (`~/.local/share/Session/recent.tsv`): a line each, newest first —
//! when, the song, and which of its versions was open.
//!
//! **Alternatives** are a song's named checkpoints, as Logic keeps them: the
//! song as it stood when the checkpoint was made ("Organized", before the
//! mix), saved beside it as complete sessions —
//! `Unbreakable.alternatives/Organized.session` next to
//! `Unbreakable.session`. Making one does not open it: the song stays what
//! is open, under its own name, and a plain save writes the song. Any
//! alternative opens as itself later, and saving it then writes it.
//!
//! A song is known by its **base** — its folder and name without an
//! extension (`…/unbreakable/Unbreakable`) — so its `.RPP`, its `.session`
//! and its alternatives are one song.

use std::path::{Path, PathBuf};

/// How many songs the list keeps.
const KEPT: usize = 30;

/// What an alternatives folder's name ends with.
const ALTERNATIVES: &str = ".alternatives";

/// A song or setlist opened lately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    /// The song's base ([`base_of`]), or a setlist's path.
    pub base: PathBuf,
    /// The version that was open: the song's own file or an alternative.
    pub opened: PathBuf,
    /// When, seconds since the epoch.
    pub at: u64,
}

impl Recent {
    /// The song's name.
    #[must_use]
    pub fn name(&self) -> String {
        self.base
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    /// Whether it is a song (that can have alternatives) rather than a
    /// setlist.
    #[must_use]
    pub fn is_song(&self) -> bool {
        main_of(&self.base).is_some()
    }

    /// The alternative that was open, if it was one.
    #[must_use]
    pub fn opened_alternative(&self) -> Option<String> {
        alternative_name(&self.opened)
    }
}

/// One of a song's alternatives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alternative {
    pub name: String,
    pub path: PathBuf,
    /// When it was last saved, seconds since the epoch.
    pub saved: u64,
}

/// A song's base from any of its versions: `X.RPP`, `X.session` or
/// `X.alternatives/Name.session` all give `…/X`. `None` for what is not a
/// song file (a setlist, a folder).
#[must_use]
pub fn base_of(path: &Path) -> Option<PathBuf> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if ext != "rpp" && ext != "session" {
        return None;
    }
    if let Some(folder) = path.parent()
        && let Some(song) = folder
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(ALTERNATIVES))
    {
        return Some(folder.parent()?.join(song));
    }
    Some(path.with_extension(""))
}

/// The song's own version: its `.session` once it has one, else its
/// `.RPP`. `None` if neither is there.
#[must_use]
pub fn main_of(base: &Path) -> Option<PathBuf> {
    let session = base.with_extension("session");
    if session.is_dir() {
        return Some(session);
    }
    ["RPP", "rpp"]
        .iter()
        .map(|ext| base.with_extension(ext))
        .find(|p| p.is_file())
}

/// Where a song's alternatives are kept.
#[must_use]
pub fn alternatives_dir(base: &Path) -> PathBuf {
    let name = base
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    base.with_file_name(format!("{name}{ALTERNATIVES}"))
}

/// The alternative `path` is, by name, if it is one.
#[must_use]
pub fn alternative_name(path: &Path) -> Option<String> {
    let folder = path.parent()?.file_name()?.to_str()?;
    folder.ends_with(ALTERNATIVES).then(|| {
        path.file_stem()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    })
}

/// A song's alternatives, the newest first.
#[must_use]
pub fn alternatives(base: &Path) -> Vec<Alternative> {
    let Ok(entries) = std::fs::read_dir(alternatives_dir(base)) else {
        return Vec::new();
    };
    let mut found: Vec<Alternative> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "session") && p.is_dir())
        .map(|path| Alternative {
            name: path
                .file_stem()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            saved: modified(&path),
            path,
        })
        .collect();
    found.sort_by(|a, b| b.saved.cmp(&a.saved).then_with(|| a.name.cmp(&b.name)));
    found
}

/// Where the alternative `name` of the song `base` is saved. A name is a
/// file name: path separators and surrounding space are refused.
///
/// # Errors
///
/// An empty name, or one that is not a plain file name.
pub fn alternative_path(base: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("an alternative needs a name".to_owned());
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(format!("{name:?} is not a name a folder can have"));
    }
    Ok(alternatives_dir(base).join(format!("{name}.session")))
}

/// The songs and setlists opened lately, newest first — those still there.
#[must_use]
pub fn recent() -> Vec<Recent> {
    read().into_iter().filter(|r| r.opened.exists()).collect()
}

/// Note that `path` (a song's version, or a setlist) was just opened: to
/// the top of the list, once.
pub fn opened(path: &Path) {
    let Some(file) = list_file() else { return };
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let base = base_of(&path).unwrap_or_else(|| path.clone());
    let mut list = read();
    list.retain(|r| r.base != base);
    list.insert(
        0,
        Recent {
            base,
            opened: path,
            at: now(),
        },
    );
    list.truncate(KEPT);
    let text = list.iter().fold(String::new(), |mut text, r| {
        use std::fmt::Write as _;
        let _ = writeln!(
            text,
            "{}\t{}\t{}",
            r.at,
            r.base.display(),
            r.opened.display()
        );
        text
    });
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let part = file.with_extension("tsv.part");
            std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, &file))
        });
    if let Err(e) = written {
        // Not fatal: the list is a convenience.
        tracing::warn!(error = %e, "could not note the song as opened");
    }
}

fn list_file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Session").join("recent.tsv"))
}

fn read() -> Vec<Recent> {
    let Some(text) = list_file().and_then(|f| std::fs::read_to_string(f).ok()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let at = parts.next()?.parse().ok()?;
            let base = PathBuf::from(parts.next()?);
            let opened = PathBuf::from(parts.next()?);
            Some(Recent { base, opened, at })
        })
        .collect()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// "just now", "5 min ago", "3 h ago", "2 days ago" — when `at` was.
#[must_use]
pub fn ago(at: u64) -> String {
    let secs = now().saturating_sub(at);
    match secs {
        0..60 => "just now".to_owned(),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        86_400..172_800 => "yesterday".to_owned(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_version_of_a_song_is_the_song() {
        let base = PathBuf::from("/songs/unbreakable/Unbreakable");
        assert_eq!(base_of(&base.with_extension("RPP")), Some(base.clone()));
        assert_eq!(base_of(&base.with_extension("session")), Some(base.clone()));
        let organized = alternatives_dir(&base).join("Organized.session");
        assert_eq!(
            organized,
            PathBuf::from("/songs/unbreakable/Unbreakable.alternatives/Organized.session")
        );
        assert_eq!(base_of(&organized), Some(base));
        assert_eq!(alternative_name(&organized).as_deref(), Some("Organized"));
        assert_eq!(base_of(Path::new("/sets/Friday.setlist")), None);
    }

    #[test]
    fn an_alternative_is_named_as_a_file_is() {
        let base = PathBuf::from("/s/Song");
        assert!(
            alternative_path(&base, "  Organized ").is_ok_and(|p| p.ends_with("Organized.session"))
        );
        assert!(alternative_path(&base, "").is_err());
        assert!(alternative_path(&base, "a/b").is_err());
        assert!(alternative_path(&base, "..").is_err());
    }

    /// Alternatives are listed from their folder, newest first; a song's
    /// own version is its `.session` once it has one.
    #[test]
    fn a_songs_alternatives_and_its_own_version() {
        let tmp = std::env::temp_dir().join(format!("fts-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let base = tmp.join("Song");
        std::fs::create_dir_all(&tmp).expect("a folder");
        std::fs::write(base.with_extension("RPP"), "<REAPER_PROJECT\n>\n").expect("an rpp");
        assert_eq!(main_of(&base), Some(base.with_extension("RPP")));
        std::fs::create_dir_all(base.with_extension("session")).expect("a session");
        assert_eq!(main_of(&base), Some(base.with_extension("session")));
        for name in ["Organized", "Mix A"] {
            std::fs::create_dir_all(alternative_path(&base, name).expect("a name")).expect("made");
            std::thread::sleep(std::time::Duration::from_millis(1_100));
        }
        let names: Vec<String> = alternatives(&base).into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Mix A", "Organized"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
