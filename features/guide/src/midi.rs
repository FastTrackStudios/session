//! MIDI note constants and mappings (ported from fts-guide `midi/notes.rs`).
//!
//! Kept so hosts can emit the guide as a MIDI track (e.g. for external
//! samplers) using the same note layout the legacy plugin used.

/// MIDI note constants for click subdivisions
pub const MIDI_NOTE_CLICK_ACCENT: u8 = 60; // C4
pub const MIDI_NOTE_CLICK_BEAT: u8 = 61; // C#4
pub const MIDI_NOTE_CLICK_EIGHTH: u8 = 62; // D4
pub const MIDI_NOTE_CLICK_SIXTEENTH: u8 = 63; // D#4
pub const MIDI_NOTE_CLICK_TRIPLET: u8 = 65; // F4

/// MIDI notes for count samples (1-8)
pub const MIDI_NOTES_COUNT: [u8; 8] = [72, 73, 74, 75, 76, 77, 78, 79]; // C5-C6

/// Map section types to MIDI notes (starting from C6 = 84)
pub fn get_midi_note_for_section_type(section_type: &str) -> Option<u8> {
    Some(match section_type {
        "Verse" => 84,                       // C6
        "Chorus" => 85,                      // C#6
        "Bridge" => 86,                      // D6
        "Intro" => 87,                       // D#6
        "Outro" => 88,                       // E6
        "Instrumental" => 89,                // F6
        "Pre Chorus" | "Pre-Chorus" => 90,   // F#6
        "Post Chorus" | "Post-Chorus" => 91, // G6
        "Breakdown" => 92,                   // G#6
        "Interlude" => 93,                   // A6
        "Tag" => 94,                         // A#6
        "Ending" => 95,                      // B6
        "Solo" => 96,                        // C7
        "Vamp" => 97,                        // C#7
        "Turnaround" => 98,                  // D7
        "Refrain" => 99,                     // D#7
        "Rap" => 100,                        // E7
        "Acapella" => 101,                   // F7
        "Exhortation" => 102,                // F#7
        _ => return None,                    // Unknown section type
    })
}
