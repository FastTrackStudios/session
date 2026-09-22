//! The setlist: the songs a session holds, and which of them is up.
//!
//! One engine, several projects (`daw_standalone::sync::Standalone` keeps
//! them side by side, as REAPER keeps tabs), and one of them current. This
//! is what the window reads: the songs in order, where the playhead is in
//! the one playing, and the colour each is known by — the setlist tabs in
//! the top bar are that colour, filling as the song plays
//! ([`crate::shell::SongTabs`]).
//!
//! A song's SPAN is the SONG region the chart stamps (`prepare`), which is
//! the part that is the song rather than the count-in before it; without
//! one it is everything the project holds. The span is what a progress
//! reads against, and what tells Live mode a song has ended.

use crate::studio::StudioSession;

/// The colours songs are known by when they carry none of their own.
///
/// Distinct at a glance in a row of tabs rather than a smooth ramp: the
/// question a setlist tab answers is "which song is that", and two greens
/// apart by a shade answer it slowly.
pub const COLORS: [&str; 8] = [
    "#3aa0ff", "#f0883e", "#4ac26b", "#d2a8ff", "#f778ba", "#e3b341", "#56d4dd", "#ff7b72",
];

/// One song in the setlist.
#[derive(Clone)]
pub struct Song {
    pub name: String,
    /// The project's guid in the engine — what `set_current_project` takes.
    pub project: String,
    pub session: StudioSession,
    /// The song's own span in project seconds: the SONG region, or
    /// everything there is.
    pub span: (f64, f64),
    /// The colour it is known by, as CSS.
    pub color: String,
}

impl PartialEq for Song {
    fn eq(&self, other: &Self) -> bool {
        self.project == other.project && self.name == other.name && self.span == other.span
    }
}

impl Song {
    /// A song from what was opened. `index` picks its colour when the
    /// project names none.
    #[must_use]
    pub fn of(name: String, project: String, session: StudioSession, index: usize) -> Self {
        let sections = &session.project.sections;
        let song = sections.iter().find(|s| s.lane == 0);
        let span = song.map(|s| (s.start, s.end)).unwrap_or_else(|| {
            let start = sections.iter().map(|s| s.start).fold(f64::INFINITY, f64::min);
            let end = sections.iter().map(|s| s.end).fold(0.0, f64::max);
            if end > start { (start, end) } else { (0.0, 0.0) }
        });
        let color = song
            .and_then(|s| s.color.clone())
            .unwrap_or_else(|| COLORS[index % COLORS.len()].to_owned());
        Self {
            name,
            project,
            session,
            span,
            color,
        }
    }

    /// How far through the song `at` seconds is, 0 before it and 1 at its
    /// end.
    #[must_use]
    pub fn progress(&self, at: f64) -> f64 {
        let (from, to) = self.span;
        if to <= from {
            return 0.0;
        }
        ((at - from) / (to - from)).clamp(0.0, 1.0)
    }

    /// Whether the playhead has run past the song's end — what Live mode
    /// reads to start the next one.
    #[must_use]
    pub fn ended(&self, at: f64) -> bool {
        self.span.1 > self.span.0 && at >= self.span.1
    }
}

/// The songs, in order, and which one is up.
#[derive(Clone, Default, PartialEq)]
pub struct Setlist {
    pub songs: Vec<Song>,
    /// Which song is current — an index into `songs`, past the end when
    /// the list is empty.
    pub at: usize,
}

impl Setlist {
    #[must_use]
    pub fn of(songs: Vec<Song>) -> Self {
        Self { songs, at: 0 }
    }

    #[must_use]
    pub fn current(&self) -> Option<&Song> {
        self.songs.get(self.at)
    }

    /// The song after the current one, if the set goes on.
    #[must_use]
    pub fn next(&self) -> Option<usize> {
        (self.at + 1 < self.songs.len()).then_some(self.at + 1)
    }

    /// Move `song` to `to`, carrying the current selection with it so the
    /// song playing stays the song playing.
    pub fn reorder(&mut self, song: usize, to: usize) {
        if song >= self.songs.len() || to >= self.songs.len() || song == to {
            return;
        }
        let current = self.songs.get(self.at).cloned();
        let moved = self.songs.remove(song);
        self.songs.insert(to, moved);
        if let Some(current) = current
            && let Some(found) = self.songs.iter().position(|s| *s == current)
        {
            self.at = found;
        }
    }

    /// Drop a song. The selection follows the song that was playing, or
    /// stays in range.
    pub fn remove(&mut self, song: usize) {
        if song >= self.songs.len() {
            return;
        }
        let current = self.songs.get(self.at).cloned();
        self.songs.remove(song);
        self.at = current
            .filter(|c| self.songs.iter().any(|s| s == c))
            .and_then(|c| self.songs.iter().position(|s| *s == c))
            .unwrap_or_else(|| self.at.min(self.songs.len().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(name: &str) -> Song {
        Song {
            name: name.to_owned(),
            project: format!("{name}-guid"),
            session: StudioSession {
                project: daw_ui::studio::ProjectRef(std::sync::Arc::default()),
                rows: daw_ui::studio::RowsRef(std::sync::Arc::default()),
                previews: crate::midi::Previews::default(),
                chart: None,
                planner: crate::studio::Planner {
                    raw: std::sync::Arc::default(),
                    scene: None,
                    kinds: std::sync::Arc::default(),
                },
            },
            span: (4.0, 24.0),
            color: "#fff".to_owned(),
        }
    }

    #[test]
    fn a_progress_runs_over_the_songs_own_span() {
        let song = song("one");
        assert_eq!(song.progress(0.0), 0.0, "the count-in is before the song");
        assert_eq!(song.progress(14.0), 0.5);
        assert_eq!(song.progress(99.0), 1.0);
        assert!(!song.ended(23.9) && song.ended(24.0));
    }

    #[test]
    fn reordering_keeps_the_song_that_is_playing_current() {
        let mut setlist = Setlist::of(vec![song("one"), song("two"), song("three")]);
        setlist.at = 2;
        setlist.reorder(2, 0);
        assert_eq!(setlist.current().map(|s| s.name.clone()), Some("three".into()));
        assert_eq!(setlist.at, 0);
    }

    #[test]
    fn removing_another_song_leaves_this_one_playing() {
        let mut setlist = Setlist::of(vec![song("one"), song("two"), song("three")]);
        setlist.at = 2;
        setlist.remove(0);
        assert_eq!(setlist.current().map(|s| s.name.clone()), Some("three".into()));
        setlist.remove(setlist.at);
        assert_eq!(setlist.current().map(|s| s.name.clone()), Some("two".into()));
    }
}
