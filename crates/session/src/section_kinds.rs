//! The kinds of a song's sections and markers: what a section region or a
//! marker is (a verse, a chorus, a count-in, the song's end), as the
//! arrangement draws them and an app's actions make them. Plain data, for
//! every target.

/// A Keyflow hotkey action. The REAPER-side dispatch for these lives in
/// `keyflow::actions`; the type is here so wasm callers can name one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyflowAction {
    InsertSection(SectionKind),
    InsertMarker(MarkerKind),
    ConvertMarkersToSessionFormat,
}

/// Song-section vocabulary — what an arrangement is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Intro,
    Verse,
    PreChorus,
    Chorus,
    Bridge,
    Outro,
    Instrumental,
    Solo,
    Hits,
    Interlude,
    Breakdown,
    Vamp,
    Refrain,
    Tag,
    Turnaround,
    CountIn,
    End,
}

/// Point markers, as distinct from the ranged [`SectionKind`] regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    CountIn,
    Start,
    End,
    SongStart,
    SongEnd,
}

impl MarkerKind {
    /// The plain RGB each marker gets — the single source both
    /// `keyflow::actions::MarkerKind::default_color` (packed into
    /// REAPER's native marker-color format) and a toolbar CSS pill
    /// (`css_color`) build on, so the REAPER-side marker and the
    /// standalone toolbar button render the literal same color.
    #[must_use]
    pub const fn rgb(self) -> u32 {
        match self {
            Self::CountIn => 0x00EC_4899,   // pink-500
            Self::SongStart => 0x0022_C55E, // green-500
            Self::Start => 0x0073_7373,     // neutral-500
            Self::End => 0x00DC_2626,       // red-600
            Self::SongEnd => 0x0047_5569,   // slate-600
        }
    }

    /// `rgb()` as a CSS hex string (`#RRGGBB`).
    #[must_use]
    pub fn css_color(self) -> String {
        format!("#{:06X}", self.rgb())
    }
}
