//! The watch remote's session-domain wire DTOs.
//!
//! A tiny JSON projection of the setlist/transport/mixer surface served by
//! the engine's `/watch/v1` HTTP+SSE bridge (watchOS can't speak vox over
//! WebSocket; see `signal-guitar-proto::watch` for the rig-side twin).
//!
//! Below them, the Session watch app's guide feed ([`WatchMessage`]): the
//! song, the beat and a few seconds of clicks, relayed by the iPhone over
//! `WatchConnectivity`.
//!
//! These shapes are the source of truth for the Swift side: the
//! `gen_watch_swift` example reflects them through facet and emits the
//! matching `Codable` structs into the watch apps
//! (`apps/desktop/watchos/` for the remote, `apps/session-watch/` for the
//! guide). Change a field here → re-run the generator → Swift follows.

use facet::Facet;

/// One mixer track as the watch renders it.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchTrack {
    /// Stable id used to address the track (mute/solo/volume commands).
    pub guid: String,
    pub name: String,
    pub index: u32,
    pub muted: bool,
    pub soloed: bool,
    /// Fader position (0 = −inf, 1 = 0 dB).
    pub volume: f32,
    /// Pan −1..1.
    pub pan: f32,
    pub is_folder: bool,
    /// 0RGB track color; 0 = unset.
    pub color: u32,
}

/// One chord of the watch's chord window (the current chord + the next 3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet)]
pub struct WatchChord {
    /// Display symbol (e.g. "Gmaj7", "/D").
    pub symbol: String,
    /// Absolute measure (from song start) the chord lands on.
    pub measure: i32,
    /// Beat within the measure (0-based).
    pub beat: i32,
    /// True for the chord under the playhead (first entry of the window).
    pub is_current: bool,
}

/// The watch's session page state: setlist cursor + transport + the chord
/// window + the mixer — refreshed over `/watch/v1/session/events`.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchSessionState {
    /// Setlist song names, in order.
    pub songs: Vec<String>,
    /// Index of the current song (−1 = none).
    pub song_index: i32,
    /// Current song's section names, in order.
    pub sections: Vec<String>,
    /// Index of the current section (−1 = none).
    pub section_index: i32,
    pub is_playing: bool,
    /// 0..1 through the current song.
    pub song_progress: f32,
    /// 0..1 through the current section.
    pub section_progress: f32,
    /// The chord window: current chord first, then the next three.
    pub chords: Vec<WatchChord>,
    /// The current chart section's lyric line ("" when the chart has none).
    pub lyric_line: String,
    /// The mixer, in track order.
    pub tracks: Vec<WatchTrack>,
    /// Bumped on every state build (client-side dedup aid).
    pub revision: u64,
}

// ── The guide feed: Session's watch app ─────────────────────────────────
//
// The Session watch app (apps/session-watch) shows where the band is in the
// song and taps the beat on the wrist. It does no timing of its own: the
// iPhone app builds these from the same Rust the desktop's guide runs on
// (session-watch-guide) and relays them over `WatchConnectivity`, with every
// time already in the WATCH's clock. The watch fires what it is given, a
// haptic lead early, and shows the beat it last fired.

/// How strongly a beat is felt on the wrist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum WatchAccent {
    /// Any other beat: the lightest tap.
    #[default]
    Beat,
    /// Beat one of a bar.
    Downbeat,
    /// A beat of the count into the song — before its first downbeat.
    CountIn,
}

/// One beat of the song, where it falls on the watch's clock and what it
/// means.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchBeat {
    /// Its place in the song's beat grid (0 = the first beat of the song,
    /// count-in included) — stable for as long as [`WatchGuideFeed::run`]
    /// is, so a beat sent twice is tapped once.
    pub index: u32,
    /// When it sounds, microseconds of the watch's continuous monotonic
    /// clock (the one it answers pings with).
    pub at_us: f64,
    /// Where it is in the project, seconds.
    pub position: f64,
    /// 0..1 through the song.
    pub progress: f32,
    /// The song's bar, counted from its first musical downbeat as 1; the
    /// count-in's bars are 0, −1, …
    pub bar: i32,
    /// Which beat of the bar, from 1.
    pub beat: u32,
    pub beats_per_bar: u32,
    /// The tempo in force, quarter notes a minute.
    pub bpm: f32,
    /// The section it is in (an index into [`WatchGuideFeed::sections`]),
    /// −1 before the first.
    pub section: i32,
    /// Its bar within that section, from 1, and how many the section has.
    pub section_bar: u32,
    pub section_bars: u32,
    /// The count the guide speaks on this beat ("1" … "4"), 0 for none.
    pub count: u32,
    /// The section the guide announces on this beat, "" for none.
    pub cue: String,
    pub accent: WatchAccent,
}

/// A section of the song, for the progress bar and "what's next".
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchSection {
    pub name: String,
    /// Project seconds.
    pub start: f64,
    pub end: f64,
    /// 0RGB; 0 = unset.
    pub color: u32,
    /// Whole bars it spans.
    pub bars: u32,
}

/// What the watch is told a few times a second: the song, where the band
/// is in it, and the next few seconds of beats to tap.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchGuideFeed {
    /// Bumped on every feed.
    pub revision: u64,
    /// Bumped whenever the transport starts, stops, jumps or changes song:
    /// beats of an older run are dropped, and beat indexes start over.
    pub run: u64,
    /// The live set's title ("" when not in one).
    pub set_title: String,
    pub song_title: String,
    /// Which song of the set, −1 for none; and how many there are.
    pub song_index: i32,
    pub song_count: u32,
    pub playing: bool,
    /// Whether both clock mappings (Task's clock → phone → watch) are
    /// known. Unlocked, the watch shows the guide but does not tap.
    pub clock_locked: bool,
    /// How far off the watch-clock times may be, microseconds — half the
    /// round trips of both links, the most their asymmetry can hide.
    pub clock_error_us: f64,
    pub sections: Vec<WatchSection>,
    /// Where the song was when this feed was built — what to show while
    /// stopped, and until the first beat below has sounded.
    pub here: WatchBeat,
    /// The coming beats, in time order; empty when stopped. The watch
    /// taps none past the last: a phone gone quiet stops the click rather
    /// than leaving it running on its own.
    pub beats: Vec<WatchBeat>,
}

/// The phone asking for the watch's clock (NTP's first stamp).
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchPing {
    /// The phone's clock when it sent this, microseconds.
    pub sent_us: f64,
}

/// The watch's answer: its clock when the ping arrived and when it
/// replied (NTP's middle stamps).
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchPong {
    pub sent_us: f64,
    pub received_us: f64,
    pub replied_us: f64,
}

/// Everything the phone sends the watch, one kind per message.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WatchMessage {
    pub ping: Option<WatchPing>,
    pub feed: Option<WatchGuideFeed>,
}
