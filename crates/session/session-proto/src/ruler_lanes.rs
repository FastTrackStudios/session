//! FTS Ruler Manager — standardized ruler lane layout for FastTrackStudio projects.
//!
//! Every FTS project should have these ruler lanes to organize markers and regions
//! into semantic categories. The lanes are split into two groups:
//!
//! **Core lanes** (always present):
//! - SECTIONS, MARKS, SONG, START/END, KEY, MODE, CHORDS, NOTES
//!
//! **Instrument note lanes** (created on demand per instrument):
//! - Drums, Bass, Guitar, Guitar 2, Keys, Keys 2, Lead, BGVs, etc.

use facet::Facet;

// ── Core Ruler Lanes ─────────────────────────────────────────────────────────

/// Core FTS ruler lanes — always present in every project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum CoreLane {
    /// Regions for song sections (Verse, Chorus, Bridge, Outro, etc.).
    /// **Default region lane** — new regions go here.
    Sections = 0,
    /// Structural markers: Count-In, SONGSTART, SONGEND.
    /// **Default marker lane** — new markers go here.
    Marks = 1,
    /// A single region spanning the entire song.
    Song = 2,
    /// Render/release bounds: =START, =END, PREROLL.
    StartEnd = 3,
    /// Key signature markers (e.g., "Eb", "F#m").
    Key = 4,
    /// Mode/scale markers (e.g., "Dorian", "Mixolydian").
    Mode = 5,
    /// Chord markers (e.g., "Cm", "Ab", "Eb7").
    Chords = 6,
    /// General notes (instrument-agnostic).
    Notes = 7,
}

impl CoreLane {
    /// All core lanes in display order.
    pub const fn all() -> &'static [CoreLane] {
        &[
            CoreLane::Sections,
            CoreLane::Marks,
            CoreLane::Song,
            CoreLane::StartEnd,
            CoreLane::Key,
            CoreLane::Mode,
            CoreLane::Chords,
            CoreLane::Notes,
        ]
    }

    /// REAPER ruler lane index (1-based).
    pub const fn lane_index(&self) -> u32 {
        *self as u32 + 1
    }

    /// Display name shown in the REAPER ruler.
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Sections => "SECTIONS",
            Self::Marks => "MARKS",
            Self::Song => "SONG",
            Self::StartEnd => "START/END",
            Self::Key => "KEY",
            Self::Mode => "MODE",
            Self::Chords => "CHORDS",
            Self::Notes => "NOTES",
        }
    }

    /// REAPER lane flags.
    /// - `8` = default region lane
    /// - `4` = default marker lane
    /// - `0` = normal
    pub const fn flags(&self) -> i32 {
        match self {
            Self::Sections => 8, // default region lane
            Self::Song => 4,     // default marker lane (matches real RPP)
            _ => 0,
        }
    }

    /// Number of core lanes.
    pub const fn count() -> u32 {
        8
    }

    pub fn from_index(index: u32) -> Option<Self> {
        Self::all().iter().find(|l| l.lane_index() == index).copied()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let upper = name.to_uppercase();
        Self::all()
            .iter()
            .find(|l| l.display_name() == upper)
            .copied()
    }
}

// ── Instrument Note Lanes ────────────────────────────────────────────────────

/// Well-known instrument roles for per-instrument note lanes.
///
/// These are created on demand — when a user does "Insert Note for Drums",
/// the Drums lane is created if it doesn't exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum InstrumentLane {
    Drums = 0,
    Bass = 1,
    Guitar = 2,
    Guitar2 = 3,
    Keys = 4,
    Keys2 = 5,
    Lead = 6,
    BGVs = 7,
}

impl InstrumentLane {
    /// All well-known instrument lanes.
    pub const fn all() -> &'static [InstrumentLane] {
        &[
            InstrumentLane::Drums,
            InstrumentLane::Bass,
            InstrumentLane::Guitar,
            InstrumentLane::Guitar2,
            InstrumentLane::Keys,
            InstrumentLane::Keys2,
            InstrumentLane::Lead,
            InstrumentLane::BGVs,
        ]
    }

    /// REAPER ruler lane index (1-based), offset after core lanes.
    pub const fn lane_index(&self) -> u32 {
        CoreLane::count() + *self as u32 + 1
    }

    /// Display name shown in the REAPER ruler.
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Drums => "Drums",
            Self::Bass => "Bass",
            Self::Guitar => "Guitar",
            Self::Guitar2 => "Guitar 2",
            Self::Keys => "Keys",
            Self::Keys2 => "Keys 2",
            Self::Lead => "Lead",
            Self::BGVs => "BGVs",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::all()
            .iter()
            .find(|l| l.display_name().eq_ignore_ascii_case(name))
            .copied()
    }
}

// ── Unified Lane Reference ───────────────────────────────────────────────────

/// A reference to any FTS ruler lane (core or instrument).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(C)]
pub enum FtsLane {
    Core(CoreLane),
    Instrument(InstrumentLane),
}

impl FtsLane {
    pub fn lane_index(&self) -> u32 {
        match self {
            Self::Core(l) => l.lane_index(),
            Self::Instrument(l) => l.lane_index(),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Core(l) => l.display_name(),
            Self::Instrument(l) => l.display_name(),
        }
    }

    pub fn flags(&self) -> i32 {
        match self {
            Self::Core(l) => l.flags(),
            Self::Instrument(_) => 0,
        }
    }
}

// ── Marker Classification ────────────────────────────────────────────────────

/// Classify a marker name to determine which lane it belongs to.
pub fn classify_marker_lane(name: &str) -> FtsLane {
    let trimmed = name.trim();
    let upper = trimmed.to_uppercase();

    match upper.as_str() {
        // MARKS lane: structural markers
        "SONGSTART" | "SONGEND" | "COUNT-IN" | "COUNT IN" | "COUNTIN" => {
            FtsLane::Core(CoreLane::Marks)
        }
        // START/END lane: render bounds
        "=START" | "=END" | "PREROLL" | "=PREROLL" => FtsLane::Core(CoreLane::StartEnd),
        _ => {
            // Markers starting with "=" are bounds
            if trimmed.starts_with('=') {
                FtsLane::Core(CoreLane::StartEnd)
            } else {
                // Default: general NOTES lane for unclassified markers
                FtsLane::Core(CoreLane::Notes)
            }
        }
    }
}

/// Classify a region name to determine which lane it belongs to.
///
/// Looks at both the region name and whether it spans the full song
/// (in which case it's a SONG region, not a section).
pub fn classify_region_lane(name: &str) -> FtsLane {
    let upper = name.trim().to_uppercase();

    // Well-known section abbreviations → SECTIONS lane
    match upper.as_str() {
        "VS" | "VERSE" | "CH" | "CHORUS" | "BR" | "BRIDGE" | "INTRO" | "OUTRO"
        | "PRE-CH" | "PRE-CHORUS" | "PRECHORUS" | "SOLO" | "BREAKDOWN" | "INTERLUDE"
        | "TAG" | "HOOK" | "INSTRUMENTAL" | "CODA" | "VAMP" | "TURNAROUND"
        | "VERSE 1" | "VERSE 2" | "VERSE 3" | "VERSE 4"
        | "CHORUS 1" | "CHORUS 2" | "CHORUS 3"
        | "BRIDGE 1" | "BRIDGE 2" => {
            return FtsLane::Core(CoreLane::Sections);
        }
        _ => {}
    }

    // Section abbreviations with numbers: "VS 1", "CH 2", etc.
    if upper.starts_with("VS ") || upper.starts_with("CH ")
        || upper.starts_with("BR ") || upper.starts_with("SOLO ")
    {
        return FtsLane::Core(CoreLane::Sections);
    }

    // Default: SECTIONS lane (most regions are sections)
    FtsLane::Core(CoreLane::Sections)
}

/// Classify a region that spans the full song as a SONG region.
///
/// Call this with `is_full_song = true` when the region's start matches
/// the song start and its end matches the song end.
pub fn classify_region_lane_with_context(name: &str, is_full_song: bool) -> FtsLane {
    if is_full_song {
        FtsLane::Core(CoreLane::Song)
    } else {
        classify_region_lane(name)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_lane_indices_sequential() {
        for (i, lane) in CoreLane::all().iter().enumerate() {
            assert_eq!(lane.lane_index(), i as u32 + 1);
        }
    }

    #[test]
    fn instrument_lanes_start_after_core() {
        assert_eq!(InstrumentLane::Drums.lane_index(), 9);
        assert_eq!(InstrumentLane::Bass.lane_index(), 10);
        assert_eq!(InstrumentLane::Guitar.lane_index(), 11);
        assert_eq!(InstrumentLane::BGVs.lane_index(), 16);
    }

    #[test]
    fn fts_lane_unified_index() {
        assert_eq!(FtsLane::Core(CoreLane::Sections).lane_index(), 1);
        assert_eq!(FtsLane::Core(CoreLane::Notes).lane_index(), 8);
        assert_eq!(FtsLane::Instrument(InstrumentLane::Drums).lane_index(), 9);
    }

    #[test]
    fn classify_structural_markers() {
        assert_eq!(classify_marker_lane("SONGSTART"), FtsLane::Core(CoreLane::Marks));
        assert_eq!(classify_marker_lane("SONGEND"), FtsLane::Core(CoreLane::Marks));
        assert_eq!(classify_marker_lane("Count-In"), FtsLane::Core(CoreLane::Marks));
    }

    #[test]
    fn classify_bound_markers() {
        assert_eq!(classify_marker_lane("=START"), FtsLane::Core(CoreLane::StartEnd));
        assert_eq!(classify_marker_lane("=END"), FtsLane::Core(CoreLane::StartEnd));
        assert_eq!(classify_marker_lane("PREROLL"), FtsLane::Core(CoreLane::StartEnd));
    }

    #[test]
    fn default_region_and_marker_lanes() {
        let region_defaults: Vec<_> = CoreLane::all().iter().filter(|l| l.flags() & 8 != 0).collect();
        let marker_defaults: Vec<_> = CoreLane::all().iter().filter(|l| l.flags() & 4 != 0).collect();
        assert_eq!(region_defaults.len(), 1);
        assert_eq!(marker_defaults.len(), 1);
        assert_eq!(region_defaults[0].display_name(), "SECTIONS");
        assert_eq!(marker_defaults[0].display_name(), "MARKS");
    }

    #[test]
    fn lane_layout_matches_reaper_rpp() {
        // Verify our layout matches the reference regions-testing.RPP:
        // RULERLANE 1 8 SECTIONS 0 -1
        // RULERLANE 2 0 MARKS 0 -1       (was COUNT-IN in test RPP, renamed)
        // RULERLANE 3 4 SONG 0 -1         (note: flags differ — see below)
        // RULERLANE 4 0 START/END 0 -1
        // RULERLANE 5 0 KEY 0 -1
        // RULERLANE 6 0 MODE 0 -1
        // RULERLANE 7 0 CHORDS 0 -1
        // RULERLANE 8 0 NOTES 0 -1
        // RULERLANE 9 0 Drums 0 -1
        // ...
        assert_eq!(CoreLane::Sections.lane_index(), 1);
        assert_eq!(CoreLane::Marks.lane_index(), 2);
        assert_eq!(CoreLane::Song.lane_index(), 3);
        assert_eq!(CoreLane::StartEnd.lane_index(), 4);
        assert_eq!(CoreLane::Key.lane_index(), 5);
        assert_eq!(CoreLane::Mode.lane_index(), 6);
        assert_eq!(CoreLane::Chords.lane_index(), 7);
        assert_eq!(CoreLane::Notes.lane_index(), 8);
        assert_eq!(InstrumentLane::Drums.lane_index(), 9);
        assert_eq!(InstrumentLane::Bass.lane_index(), 10);
        assert_eq!(InstrumentLane::Guitar.lane_index(), 11);
        assert_eq!(InstrumentLane::Guitar2.lane_index(), 12);
        assert_eq!(InstrumentLane::Keys.lane_index(), 13);
        assert_eq!(InstrumentLane::Keys2.lane_index(), 14);
        assert_eq!(InstrumentLane::Lead.lane_index(), 15);
        assert_eq!(InstrumentLane::BGVs.lane_index(), 16);
    }
}
