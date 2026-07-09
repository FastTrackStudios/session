//! Guide track group definitions
//!
//! This group captures timing and cue tracks for performers:
//! - Click tracks (metronome)
//! - Count-ins
//! - Cue tracks for performers' in-ears
//! - Song section markers (Verse, Chorus, Bridge, etc.)
//! - Dynamic cues (Build, Softly, All In, etc.)

use crate::item_metadata::ItemMetadata;
use monarchy::Group;

/// Top-level Guide group containing timing, cue, and section tracks
pub struct Guide;

impl From<Guide> for Group<ItemMetadata> {
    fn from(_val: Guide) -> Self {
        Group::builder("Guide")
            .prefix("GDE")
            .patterns(vec![
                // Generic patterns (matched by subgroups)
                "click",
                "metronome",
                "count",
                "guide",
                "cue",
            ])
            .group(Click)
            .group(Count)
            .group(Cues)
            .group(Sections)
            .group(DynamicCues)
            .build()
    }
}

/// Click/Metronome subgroup for timing tracks
pub struct Click;

impl From<Click> for Group<ItemMetadata> {
    fn from(_val: Click) -> Self {
        Group::builder("Click")
            .patterns(vec![
                "click",
                "click track",
                "metronome",
                "met",
                "tempo",
                "tempo track",
            ])
            .build()
    }
}

/// Count subgroup for count-ins (not part of final song)
pub struct Count;

impl From<Count> for Group<ItemMetadata> {
    fn from(_val: Count) -> Self {
        Group::builder("Count")
            .patterns(vec![
                "count",
                "count in",
                "count-in",
                "countin",
                "count off",
                "1234",
                "one two three four",
                "intro count",
            ])
            .build()
    }
}

/// Cues subgroup for performer cue tracks (in-ears)
pub struct Cues;

impl From<Cues> for Group<ItemMetadata> {
    fn from(_val: Cues) -> Self {
        Group::builder("Cues")
            .patterns(vec![
                // Guide patterns
                "guide",
                "guide track",
                "guide vox",
                "guide vocal",
                // Section guides
                "sections",
                "sections guide",
                "section guide",
                "tracks guide",
                // Cue/callout patterns
                "cue",
                "cue track",
                "callout",
                "callouts",
                "call out",
                "call outs",
                // IEM/monitor patterns
                "iem",
                "in ear",
                "in-ear",
                "monitor cue",
                "ear cue",
            ])
            .build()
    }
}

/// Song section markers (Verse, Chorus, Bridge, etc.)
///
/// These patterns match common song structure terms used in
/// markers, regions, and cue tracks.
pub struct Sections;

impl From<Sections> for Group<ItemMetadata> {
    fn from(_val: Sections) -> Self {
        Group::builder("Sections")
            .patterns(vec![
                // Explicit section markers (multi-word or unambiguous — avoids stealing
                // "Solo Gtr", "Outro Gtr", "Chorus Harmony" etc. from instrument groups)
                "pre chorus",
                "pre-chorus",
                "prechorus",
                "post chorus",
                "post-chorus",
                "postchorus",
                "breakdown",
                "interlude",
                "instrumental",
                "ending",
                "tag",
                "vamp",
                "turnaround",
                "refrain",
                "acapella",
                "a capella",
                "exhortation",
                // Numbered sections (common in arrangements)
                "section 1",
                "section 2",
                "section 3",
                "section 4",
                "section 5",
                "section 6",
                "section 7",
            ])
            .build()
    }
}

/// Dynamic cues for performers (Build, Softly, All In, etc.)
///
/// These patterns match performance direction terms used in
/// cue tracks and markers to guide dynamics and energy.
pub struct DynamicCues;

impl From<DynamicCues> for Group<ItemMetadata> {
    fn from(_val: DynamicCues) -> Self {
        Group::builder("Dynamic Cues")
            .patterns(vec![
                // Energy/intensity cues
                "build",
                "slowly build",
                "all in",
                "all-in",
                "allin",
                "softly",
                "soft",
                "hold",
                "break",
                "hits",
                "hit",
                // Instrument entry cues
                "drums in",
                "bass in",
                "keys in",
                "guitars in",
                "band in",
                // Endings
                "big ending",
                "big end",
                "last time",
                "final",
                // Key changes
                "key change up",
                "key change down",
                "modulation",
                "mod up",
                "mod down",
                // Improvisation
                "ad lib",
                "ad-lib",
                "adlib",
                "worship freely",
                "free worship",
                "spontaneous",
                // Other dynamic directions
                "crescendo",
                "decrescendo",
                "fade",
                "fade out",
                "fade in",
                "stop",
                "tacet",
                "rest",
                "wait",
                "cue",
            ])
            .build()
    }
}
