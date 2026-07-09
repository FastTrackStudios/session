//! Name-to-color resolution with abbreviation and alias support.
//!
//! Provides lookup functions for resolving instrument names, section names,
//! and DAW region names to their canonical colors.

use std::collections::HashMap;
use std::sync::LazyLock;

use color_palette::Color;

use crate::instruments::*;
use crate::sections::{colors as sec, cues, utility};

// ============================================================================
// Lookup table
// ============================================================================

/// Master lookup table: lowercase name/abbreviation → Color.
/// Covers instruments, sections, abbreviations, and dynamic cues.
static COLOR_MAP: LazyLock<HashMap<&'static str, Color>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // ------------------------------------------------------------------
    // Instrument groups
    // ------------------------------------------------------------------

    // Top-level groups
    m.insert("drums", groups::DRUMS);
    m.insert("percussion", groups::PERCUSSION);
    m.insert("bass", groups::BASS);
    m.insert("guitars", groups::GUITARS);
    m.insert("keys", groups::KEYS);
    m.insert("synths", groups::SYNTHS);
    m.insert("horns", groups::HORNS);
    m.insert("harmonica", groups::HARMONICA);
    m.insert("fiddle", groups::FIDDLE);
    m.insert("violin", groups::FIDDLE);
    m.insert("viola", groups::FIDDLE);
    m.insert("cello", groups::FIDDLE);
    m.insert("vocals", groups::VOCALS);
    m.insert("choir", groups::CHOIR);
    m.insert("orchestra", groups::ORCHESTRA);
    m.insert("sfx", groups::SFX);
    m.insert("guide", groups::GUIDE);
    m.insert("reference", groups::REFERENCE);
    m.insert("stem split", groups::STEM_SPLIT);
    m.insert("stem_split", groups::STEM_SPLIT);

    // Guitar sub-groups
    m.insert("guitars/electric", guitars::ELECTRIC);
    m.insert("guitars/acoustic", guitars::ACOUSTIC);
    m.insert("guitars/steel", guitars::STEEL);
    m.insert("guitars/banjo", guitars::BANJO);
    m.insert("electric guitar", guitars::ELECTRIC);
    m.insert("electric_guitar", guitars::ELECTRIC);
    m.insert("acoustic guitar", guitars::ACOUSTIC);
    m.insert("acoustic_guitar", guitars::ACOUSTIC);
    m.insert("steel guitar", guitars::STEEL);
    m.insert("steel_guitar", guitars::STEEL);

    // Vocal sub-groups
    m.insert("vocals/lead", vocals::LEAD);
    m.insert("vocals/background", vocals::BACKGROUND);
    m.insert("vocals/bgvs", vocals::BACKGROUND);
    m.insert("lead vocals", vocals::LEAD);
    m.insert("lead_vocals", vocals::LEAD);
    m.insert("background vocals", vocals::BACKGROUND);
    m.insert("background_vocals", vocals::BACKGROUND);
    m.insert("bgvs", vocals::BACKGROUND);
    m.insert("bgv", vocals::BACKGROUND);
    m.insert("bvs", vocals::BACKGROUND);

    // Drum sub-groups
    m.insert("drums/drum_kit", drums::DRUM_KIT);
    m.insert("drums/electronic", drums::ELECTRONIC);
    m.insert("drums/kick", drums::KICK);
    m.insert("drums/snare", drums::SNARE);
    m.insert("drums/tom", drums::TOM);
    m.insert("drums/cymbals", drums::CYMBALS);
    m.insert("drums/hi_hat", drums::HI_HAT);
    m.insert("drums/room", drums::ROOM);
    m.insert("drum kit", drums::DRUM_KIT);
    m.insert("electronic kit", drums::ELECTRONIC);
    m.insert("kick", drums::KICK);
    m.insert("snare", drums::SNARE);
    m.insert("tom", drums::TOM);
    m.insert("cymbals", drums::CYMBALS);
    m.insert("hi hat", drums::HI_HAT);
    m.insert("hi-hat", drums::HI_HAT);
    m.insert("hihat", drums::HI_HAT);
    m.insert("room", drums::ROOM);

    // Bass sub-groups
    m.insert("bass/guitar", bass::GUITAR);
    m.insert("bass/synth", bass::SYNTH);
    m.insert("bass/upright", bass::UPRIGHT);
    m.insert("bass guitar", bass::GUITAR);
    m.insert("bass_guitar", bass::GUITAR);
    m.insert("synth bass", bass::SYNTH);
    m.insert("synth_bass", bass::SYNTH);
    m.insert("upright bass", bass::UPRIGHT);
    m.insert("upright_bass", bass::UPRIGHT);

    // Keys sub-groups
    m.insert("keys/piano", keys::PIANO);
    m.insert("keys/electric", keys::ELECTRIC);
    m.insert("keys/organ", keys::ORGAN);
    m.insert("keys/harpsichord", keys::HARPSICHORD);
    m.insert("keys/clavichord", keys::CLAVICHORD);
    m.insert("piano", keys::PIANO);
    m.insert("electric keys", keys::ELECTRIC);
    m.insert("electric_keys", keys::ELECTRIC);
    m.insert("organ", keys::ORGAN);
    m.insert("harpsichord", keys::HARPSICHORD);
    m.insert("clavichord", keys::CLAVICHORD);

    // Synth sub-groups
    m.insert("synths/lead", synths::LEAD);
    m.insert("synths/pad", synths::PAD);
    m.insert("synths/arp", synths::ARP);
    m.insert("synths/chord", synths::CHORD);
    m.insert("synths/keys", synths::KEYS);
    m.insert("synths/fx", synths::FX);
    m.insert("lead synth", synths::LEAD);
    m.insert("pad", synths::PAD);
    m.insert("arp", synths::ARP);
    m.insert("synth arp", synths::ARP);

    // Orchestra sub-groups
    m.insert("orchestra/strings", orchestra::STRINGS);
    m.insert("orchestra/woodwinds", orchestra::WOODWINDS);
    m.insert("orchestra/brass", orchestra::BRASS);
    m.insert("orchestra/harp", orchestra::HARP);
    m.insert("orchestra/percussion", orchestra::PERCUSSION);
    m.insert("strings", orchestra::STRINGS);
    m.insert("woodwinds", orchestra::WOODWINDS);
    m.insert("brass", orchestra::BRASS);
    m.insert("harp", orchestra::HARP);

    // String section
    m.insert("violins", orchestra::strings::VIOLINS);
    m.insert("viola", orchestra::strings::VIOLA);
    m.insert("cello", orchestra::strings::CELLO);
    m.insert("contrabass", orchestra::strings::CONTRABASS);

    // Woodwind section
    m.insert("flutes", orchestra::woodwinds::FLUTES);
    m.insert("flute", orchestra::woodwinds::FLUTES);
    m.insert("oboes", orchestra::woodwinds::OBOES);
    m.insert("oboe", orchestra::woodwinds::OBOES);
    m.insert("clarinets", orchestra::woodwinds::CLARINETS);
    m.insert("clarinet", orchestra::woodwinds::CLARINETS);
    m.insert("bassoons", orchestra::woodwinds::BASSOONS);
    m.insert("bassoon", orchestra::woodwinds::BASSOONS);
    m.insert("piccolo", orchestra::woodwinds::PICCOLO);

    // Brass section
    m.insert("trumpets", orchestra::brass::TRUMPETS);
    m.insert("trumpet", orchestra::brass::TRUMPETS);
    m.insert("french horns", orchestra::brass::HORNS);
    m.insert("french horn", orchestra::brass::HORNS);
    m.insert("trombones", orchestra::brass::TROMBONES);
    m.insert("trombone", orchestra::brass::TROMBONES);
    m.insert("tuba", orchestra::brass::TUBA);

    // Percussion sub-groups
    m.insert("shaker", percussion::SHAKER);
    m.insert("tambourine", percussion::TAMBOURINE);
    m.insert("cabasa", percussion::CABASA);
    m.insert("conga", percussion::CONGA);
    m.insert("bongo", percussion::BONGO);
    m.insert("cowbell", percussion::COWBELL);
    m.insert("clap", percussion::CLAP);
    m.insert("triangle", percussion::TRIANGLE);

    // ------------------------------------------------------------------
    // Utility / guide tracks
    // ------------------------------------------------------------------

    m.insert("click", utility::CLICK);
    m.insert("click track", utility::CLICK);
    m.insert("guide", utility::GUIDE_TRACK);
    m.insert("guide track", utility::GUIDE_TRACK);
    m.insert("loop", utility::CLICK);
    m.insert("count", utility::CLICK);
    m.insert("count in", utility::CLICK);
    m.insert("count-in", utility::CLICK);
    m.insert("countin", utility::CLICK);
    m.insert("cue", utility::CUE);
    m.insert("cue track", utility::CUE);

    // ------------------------------------------------------------------
    // Song sections — full names
    // ------------------------------------------------------------------

    m.insert("intro", sec::INTRO);
    m.insert("verse", sec::VERSE);
    m.insert("verse 1", sec::VERSE);
    m.insert("verse 2", sec::VERSE);
    m.insert("verse 3", sec::VERSE);
    m.insert("verse 4", sec::VERSE);
    m.insert("verse 5", sec::VERSE);
    m.insert("verse 6", sec::VERSE);
    m.insert("pre chorus", sec::PRE_CHORUS);
    m.insert("pre-chorus", sec::PRE_CHORUS);
    m.insert("prechorus", sec::PRE_CHORUS);
    m.insert("pre chorus 1", sec::PRE_CHORUS);
    m.insert("pre chorus 2", sec::PRE_CHORUS);
    m.insert("pre chorus 3", sec::PRE_CHORUS);
    m.insert("pre chorus 4", sec::PRE_CHORUS);
    m.insert("chorus", sec::CHORUS);
    m.insert("chorus 1", sec::CHORUS);
    m.insert("chorus 2", sec::CHORUS);
    m.insert("chorus 3", sec::CHORUS);
    m.insert("chorus 4", sec::CHORUS);
    m.insert("post chorus", sec::POST_CHORUS);
    m.insert("post-chorus", sec::POST_CHORUS);
    m.insert("postchorus", sec::POST_CHORUS);
    m.insert("bridge", sec::BRIDGE);
    m.insert("bridge 1", sec::BRIDGE);
    m.insert("bridge 2", sec::BRIDGE);
    m.insert("bridge 3", sec::BRIDGE);
    m.insert("bridge 4", sec::BRIDGE);
    m.insert("breakdown", sec::BREAKDOWN);
    m.insert("interlude", sec::INTERLUDE);
    m.insert("instrumental", sec::INSTRUMENTAL);
    m.insert("solo", sec::SOLO);
    m.insert("outro", sec::OUTRO);
    m.insert("ending", sec::ENDING);
    m.insert("tag", sec::TAG);
    m.insert("vamp", sec::VAMP);
    m.insert("turnaround", sec::TURNAROUND);
    m.insert("refrain", sec::REFRAIN);
    m.insert("acapella", sec::ACAPELLA);
    m.insert("a capella", sec::ACAPELLA);
    m.insert("rap", sec::RAP);
    m.insert("exhortation", sec::EXHORTATION);

    // ------------------------------------------------------------------
    // Song section abbreviations (common in DAW regions)
    // ------------------------------------------------------------------

    // Intro
    m.insert("in", sec::INTRO);
    m.insert("int", sec::INTRO);
    m.insert("i", sec::INTRO);
    // Verse
    m.insert("v", sec::VERSE);
    m.insert("vs", sec::VERSE);
    m.insert("v1", sec::VERSE);
    m.insert("v2", sec::VERSE);
    m.insert("v3", sec::VERSE);
    m.insert("v4", sec::VERSE);
    m.insert("v5", sec::VERSE);
    m.insert("v6", sec::VERSE);
    m.insert("vs1", sec::VERSE);
    m.insert("vs2", sec::VERSE);
    m.insert("vs3", sec::VERSE);
    m.insert("vs4", sec::VERSE);
    // Pre-chorus
    m.insert("pc", sec::PRE_CHORUS);
    m.insert("pre", sec::PRE_CHORUS);
    m.insert("pc1", sec::PRE_CHORUS);
    m.insert("pc2", sec::PRE_CHORUS);
    m.insert("pc3", sec::PRE_CHORUS);
    m.insert("pre1", sec::PRE_CHORUS);
    m.insert("pre2", sec::PRE_CHORUS);
    // Chorus
    m.insert("c", sec::CHORUS);
    m.insert("ch", sec::CHORUS);
    m.insert("cho", sec::CHORUS);
    m.insert("c1", sec::CHORUS);
    m.insert("c2", sec::CHORUS);
    m.insert("c3", sec::CHORUS);
    m.insert("c4", sec::CHORUS);
    m.insert("ch1", sec::CHORUS);
    m.insert("ch2", sec::CHORUS);
    m.insert("ch3", sec::CHORUS);
    m.insert("ch4", sec::CHORUS);
    // Post-chorus
    m.insert("poc", sec::POST_CHORUS);
    m.insert("post", sec::POST_CHORUS);
    // Bridge
    m.insert("b", sec::BRIDGE);
    m.insert("br", sec::BRIDGE);
    m.insert("brg", sec::BRIDGE);
    m.insert("b1", sec::BRIDGE);
    m.insert("b2", sec::BRIDGE);
    m.insert("br1", sec::BRIDGE);
    m.insert("br2", sec::BRIDGE);
    // Breakdown
    m.insert("bd", sec::BREAKDOWN);
    m.insert("brkdn", sec::BREAKDOWN);
    m.insert("brkdwn", sec::BREAKDOWN);
    // Interlude
    m.insert("inter", sec::INTERLUDE);
    m.insert("intl", sec::INTERLUDE);
    m.insert("il", sec::INTERLUDE);
    // Instrumental
    m.insert("inst", sec::INSTRUMENTAL);
    m.insert("instr", sec::INSTRUMENTAL);
    // Solo
    m.insert("s", sec::SOLO);
    m.insert("sol", sec::SOLO);
    // Outro
    m.insert("o", sec::OUTRO);
    m.insert("out", sec::OUTRO);
    m.insert("end", sec::ENDING);
    // Tag
    m.insert("t", sec::TAG);
    m.insert("tg", sec::TAG);
    // Turnaround
    m.insert("ta", sec::TURNAROUND);
    m.insert("turn", sec::TURNAROUND);
    // Vamp
    m.insert("vp", sec::VAMP);
    // Refrain
    m.insert("ref", sec::REFRAIN);
    m.insert("rf", sec::REFRAIN);

    // ------------------------------------------------------------------
    // Dynamic cues
    // ------------------------------------------------------------------

    m.insert("build", cues::BUILD);
    m.insert("slowly build", cues::SLOWLY_BUILD);
    m.insert("swell", cues::SWELL);
    m.insert("all in", cues::ALL_IN);
    m.insert("all-in", cues::ALL_IN);
    m.insert("softly", cues::SOFTLY);
    m.insert("hold", cues::HOLD);
    m.insert("break", cues::BREAK);
    m.insert("hits", cues::HITS);
    m.insert("drums in", cues::DRUMS_IN);
    m.insert("big ending", cues::BIG_ENDING);
    m.insert("last time", cues::LAST_TIME);
    m.insert("key change up", cues::KEY_CHANGE_UP);
    m.insert("key change down", cues::KEY_CHANGE_DOWN);
    m.insert("ad lib", cues::AD_LIB);
    m.insert("ad-lib", cues::AD_LIB);
    m.insert("adlib", cues::AD_LIB);
    m.insert("worship freely", cues::WORSHIP_FREELY);

    m
});

// ============================================================================
// Public lookup functions
// ============================================================================

/// Look up a color by name (case-insensitive).
///
/// Supports instrument groups, sub-groups, sections, abbreviations,
/// path-based lookups ("Guitars/Electric"), and alternative formats
/// ("electric_guitar", "Electric Guitar").
pub fn color_for_name(name: &str) -> Option<Color> {
    let normalized = name.to_lowercase();
    COLOR_MAP.get(normalized.as_str()).copied()
}

/// Look up a color for a DAW region/marker name, with smart parsing.
///
/// Handles common DAW region naming conventions:
/// - `"V1"`, `"VS2"`, `"CH"`, `"BR"`
/// - `"1. Verse"`, `"2. Chorus"`
/// - `"INTRO - 8 bars"`
pub fn color_for_region(name: &str) -> Option<Color> {
    // Try exact match first
    if let Some(color) = color_for_name(name) {
        return Some(color);
    }

    // Normalize: lowercase, trim, remove leading numbers/dots,
    // trailing numbers, and split on common separators
    let normalized = name
        .to_lowercase()
        .trim()
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ')
        .trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ')
        .split(&['-', '–', '—', '|', ':'][..])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    color_for_name(&normalized)
}

/// Look up a color for a hierarchical path (e.g., `["Drums", "Kick", "In"]`).
///
/// Tries most-specific to least-specific:
/// 1. Full path joined with `/` ("drums/kick")
/// 2. Each element from right to left
pub fn color_for_path(path: &[&str]) -> Option<Color> {
    // Try full path first
    let full_path = path.join("/").to_lowercase();
    if let Some(color) = COLOR_MAP.get(full_path.as_str()) {
        return Some(*color);
    }

    // Try from most specific to least specific
    for i in (0..path.len()).rev() {
        if let Some(color) = color_for_name(path[i]) {
            return Some(color);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::{drums, groups, guitars, vocals};
    use crate::sections::colors as sec;

    #[test]
    fn test_instrument_groups() {
        assert_eq!(color_for_name("Drums"), Some(groups::DRUMS));
        assert_eq!(color_for_name("drums"), Some(groups::DRUMS));
        assert_eq!(color_for_name("DRUMS"), Some(groups::DRUMS));
        assert_eq!(color_for_name("Bass"), Some(groups::BASS));
        assert_eq!(color_for_name("Vocals"), Some(groups::VOCALS));
    }

    #[test]
    fn test_instrument_subgroups() {
        assert_eq!(color_for_name("Electric Guitar"), Some(guitars::ELECTRIC));
        assert_eq!(color_for_name("Kick"), Some(drums::KICK));
        assert_eq!(color_for_name("BGVs"), Some(vocals::BACKGROUND));
    }

    #[test]
    fn test_path_lookup() {
        assert_eq!(color_for_path(&["Drums", "Kick"]), Some(drums::KICK));
        assert_eq!(color_for_path(&["Drums"]), Some(groups::DRUMS));
    }

    #[test]
    fn test_section_names() {
        assert_eq!(color_for_name("verse"), Some(sec::VERSE));
        assert_eq!(color_for_name("chorus"), Some(sec::CHORUS));
        assert_eq!(color_for_name("bridge"), Some(sec::BRIDGE));
    }

    #[test]
    fn test_section_abbreviations() {
        assert_eq!(color_for_name("v1"), Some(sec::VERSE));
        assert_eq!(color_for_name("ch"), Some(sec::CHORUS));
        assert_eq!(color_for_name("br"), Some(sec::BRIDGE));
    }

    #[test]
    fn test_region_parsing() {
        assert_eq!(color_for_region("Verse"), Some(sec::VERSE));
        assert_eq!(color_for_region("1. Verse"), Some(sec::VERSE));
        assert_eq!(color_for_region("V1"), Some(sec::VERSE));
        assert_eq!(color_for_region("CHORUS - 8 bars"), Some(sec::CHORUS));
    }

    #[test]
    fn test_chorus_is_blue_not_red() {
        // Chorus should be blue (the strong, memorable anchor)
        assert_eq!(color_for_name("chorus"), Some(sec::CHORUS));
        assert_eq!(sec::CHORUS, color_palette::palette::blue::S500);
    }
}
