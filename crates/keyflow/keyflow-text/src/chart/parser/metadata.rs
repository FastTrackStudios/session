//! Metadata parsing for charts
//!
//! Handles parsing of chart metadata including title, artist, tempo,
//! time signature, key, and settings.

use super::ChartParser;
use crate::key::Key;
use crate::metadata::SongMetadata;
use crate::time::{Tempo, TimeSignature};

// region:    --- Metadata Parsing

impl<'a> ChartParser<'a> {
    /// Phase 1: Parse metadata
    pub(super) fn parse_metadata(
        &mut self,
        lines: &[&str],
        start_idx: usize,
    ) -> Result<usize, String> {
        let mut idx = start_idx;

        // Skip empty lines and parse settings
        while idx < lines.len() {
            if lines[idx].is_empty() {
                idx += 1;
                continue;
            }

            if let Some((name, value)) = Self::parse_alias_declaration(lines[idx]) {
                self.aliases.insert(name, value);
                idx += 1;
                continue;
            }

            // Check for settings (lines starting with /)
            if lines[idx].starts_with('/') {
                self.parse_setting(lines[idx])?;
                idx += 1;
                continue;
            }

            break;
        }

        if idx >= lines.len() {
            return Ok(idx);
        }

        let first_line = lines[idx].trim();

        // Check if this looks like music content rather than a title:
        // - Section markers (VS, CH, [SOLO], etc.)
        // - Pure metadata line (just time sig and/or key, no title-like text)
        // - Chord content (starts with chord symbols)
        if Self::looks_like_section_marker(first_line) {
            // No title, jump straight to sections
            return Ok(idx);
        }

        if Self::looks_like_pure_metadata_line(first_line) {
            // No title, just metadata like "4/4 #G"
            self.parse_metadata_line(first_line)?;
            idx += 1;
            return self.continue_metadata_parsing(lines, idx);
        }

        if Self::looks_like_chord_content(first_line) {
            // No title, starts with chords
            return Ok(idx);
        }

        // First non-empty line is typically "Title - Artist" or "Title (Subtitle) - Artist"
        let (title, artist, subtitle) = SongMetadata::parse_title_artist_subtitle(first_line);
        self.metadata.title = title;
        self.metadata.artist = artist;
        self.metadata.subtitle = subtitle;
        idx += 1;

        // Skip empty lines and parse more settings
        while idx < lines.len() {
            if lines[idx].is_empty() {
                idx += 1;
                continue;
            }

            if let Some((name, value)) = Self::parse_alias_declaration(lines[idx]) {
                self.aliases.insert(name, value);
                idx += 1;
                continue;
            }

            // Check for settings (lines starting with /)
            if lines[idx].starts_with('/') {
                self.parse_setting(lines[idx])?;
                idx += 1;
                continue;
            }

            break;
        }

        // Check for "Transcribed By X" line (before metadata line)
        // This becomes the subtitle if not already set from parentheses
        if idx < lines.len() {
            let line = lines[idx].trim();
            let line_lower = line.to_lowercase();
            if line_lower.starts_with("transcribed by") {
                // Extract the name after "Transcribed By"
                let name = line[14..].trim(); // Skip "Transcribed By"
                if !name.is_empty() {
                    // Override subtitle with transcriber info
                    self.metadata.subtitle = Some(format!("Transcribed By {}", name));
                }
                idx += 1;
            } else if !Self::looks_like_metadata_line(line)
                && !line.is_empty()
                && !Self::looks_like_chord_content(line)
            {
                // If the line doesn't look like metadata (no bpm, time sig, key),
                // and doesn't look like chord content, treat it as a subtitle/transcriber line
                if self.metadata.subtitle.is_none() {
                    self.metadata.subtitle = Some(line.to_string());
                    idx += 1;
                }
            }
        }

        // Next line might be "120bpm 4/4 #G" (tempo, time sig, key)
        if idx < lines.len() {
            let line = lines[idx];
            if Self::looks_like_metadata_line(line) {
                self.parse_metadata_line(line)?;
                idx += 1;
            }
        }

        // Skip empty lines and parse settings/chord assignments after metadata line
        while idx < lines.len() {
            if lines[idx].is_empty() {
                idx += 1;
                continue;
            }

            if let Some((name, value)) = Self::parse_alias_declaration(lines[idx]) {
                self.aliases.insert(name, value);
                idx += 1;
                continue;
            }

            // Check for settings (lines starting with /)
            if lines[idx].starts_with('/') {
                self.parse_setting(lines[idx])?;
                idx += 1;
                continue;
            }

            // Check for global chord assignments (e.g., "Cm = Cm7b5")
            if let Some((basic, full)) = Self::parse_chord_assignment(lines[idx]) {
                self.chord_memory.add_global_assignment(&basic, &full);
                idx += 1;
                continue;
            }

            break;
        }

        // Check for version (V1, V2, etc.) or part name on the next line
        if idx < lines.len() {
            let line = lines[idx].trim();
            let line_upper = line.to_uppercase();

            // Check for version pattern: V1, V2, V3, etc.
            if let Some(version) = Self::parse_version_string(&line_upper) {
                self.metadata.version = Some(version);
                idx += 1;
            } else if !line.is_empty()
                && !line.starts_with("<<")
                && !Self::looks_like_section_marker(line)
                && !Self::looks_like_chord_content(line)
            {
                // If it's not a section marker or chord content, treat it as part name
                self.metadata.part_name = Some(line.to_string());
                idx += 1;

                // Check if next line is a version
                if idx < lines.len() {
                    let next_line = lines[idx].trim().to_uppercase();
                    if let Some(version) = Self::parse_version_string(&next_line) {
                        self.metadata.version = Some(version);
                        idx += 1;
                    }
                }
            }
        }

        Ok(idx)
    }

    /// Check if a line looks like metadata (contains tempo, time sig, or key)
    pub(super) fn looks_like_metadata_line(line: &str) -> bool {
        line.contains("bpm") || line.contains('/') || line.contains('#') || line.contains('b')
    }

    /// Parse a version string like "V1", "V2", etc.
    /// Returns the version number if valid, None otherwise.
    pub(super) fn parse_version_string(s: &str) -> Option<u8> {
        let s = s.trim();
        if s.starts_with('V') && s.len() >= 2 {
            s[1..].parse::<u8>().ok()
        } else {
            None
        }
    }

    /// Check if a line looks like a section marker (e.g., "VS 16", "CH", "Intro 4", "[SOLO] 8")
    ///
    /// A section marker is a line that consists of:
    /// - Just a section type: "VS", "Chorus"
    /// - Section type + number/expression: "VS 8", "Chorus 4+1"
    /// - Custom section: "[SOLO]", "[SOLO] 8"
    ///
    /// NOT a title that happens to start with a word that matches a section type.
    pub(super) fn looks_like_section_marker(line: &str) -> bool {
        use crate::sections::SectionType;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }

        // Bracketed custom section, e.g. "[SOLO]" / "[SOLO] 8".
        if trimmed.starts_with('[') && trimmed.contains(']') {
            return true;
        }

        // Inline-content forms put the chords after a `:` or `,`
        // (e.g. "VS 8: Cm Fm", "intro, Cm"); the marker is the part before it.
        let marker = match trimmed.find([':', ',']) {
            Some(i) => trimmed[..i].trim(),
            None => trimmed,
        };
        if marker.is_empty() {
            return false;
        }

        // Defer to the authoritative section-header parser, so sub-labels
        // (`CH 3A 4`), measure expressions, quoted comments, key changes
        // (`BR 8 #G`), and pre-/post- sections are recognised exactly as they
        // are when the section is really parsed — no second, drifting copy.
        SectionType::parse_with_measure_count(marker).is_some()
    }

    /// Parse a metadata line (e.g., "120bpm 4/4 #G")
    pub(super) fn parse_metadata_line(&mut self, line: &str) -> Result<(), String> {
        let parts: Vec<&str> = line.split_whitespace().collect();

        for part in parts {
            // Clef keyword (e.g. `64 BPM #E 4/4 bass`). Checked first so `bass`
            // isn't mistaken for a B-flat key token.
            if let Some(clef) = match part.to_ascii_lowercase().as_str() {
                "treble" | "g-clef" => Some(crate::chart::ChartClef::Treble),
                "bass" | "f-clef" => Some(crate::chart::ChartClef::Bass),
                "alto" => Some(crate::chart::ChartClef::Alto),
                "tenor" => Some(crate::chart::ChartClef::Tenor),
                "perc" | "percussion" => Some(crate::chart::ChartClef::Percussion),
                _ => None,
            } {
                self.initial_clef = Some(clef);
                continue;
            }

            // Try tempo
            if part.ends_with("bpm") || part.parse::<u32>().is_ok() {
                if let Some(tempo) = Self::parse_tempo(part) {
                    self.tempo = Some(tempo);
                    continue;
                }
            }

            // Try time signature
            if part.contains('/') {
                if let Some((num, den)) = Self::parse_time_signature(part) {
                    let time_signature = TimeSignature::new(num as u32, den as u32);
                    self.time_signature = Some(time_signature);
                    if self.initial_time_signature.is_none() {
                        self.initial_time_signature = Some(time_signature);
                    }
                    continue;
                }
            }

            // Try key signature
            if part.starts_with('#') || part.starts_with('b') {
                if let Ok(key) = Key::parse(part) {
                    self.current_key = Some(key.clone());
                    self.initial_key = Some(key);
                    continue;
                }
            }
        }

        Ok(())
    }

    /// Parse time signature (e.g., "4/4", "6/8")
    pub(super) fn parse_time_signature(s: &str) -> Option<(u8, u8)> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 2 {
            let num = parts[0].parse::<u8>().ok()?;
            let den = parts[1].parse::<u8>().ok()?;
            Some((num, den))
        } else {
            None
        }
    }

    /// Parse tempo from a token (e.g., "120bpm", "8th=168bpm", or "120")
    fn parse_tempo(token: &str) -> Option<Tempo> {
        let trimmed = token.trim();
        let value = if let Some(stripped) = trimmed.strip_suffix("bpm") {
            stripped.trim()
        } else {
            trimmed
        };
        let value = value.rsplit_once('=').map_or(value, |(_, bpm)| bpm.trim());
        let bpm = value.parse::<f64>().ok()?;
        Tempo::try_from_bpm(bpm).ok()
    }

    /// Parse a setting line (e.g., "/SMART_REPEATS=true")
    pub(super) fn parse_setting(&mut self, line: &str) -> Result<(), String> {
        if let Some((name, value)) = Self::parse_alias_declaration(line) {
            self.aliases.insert(name, value);
            return Ok(());
        }

        // A top-level `/Duration` (before any section) sets a chart-wide default
        // duration applied to every section unless that section overrides it.
        let trimmed = line.trim();
        if let Some(value) = trimmed
            .strip_prefix("/duration ")
            .or_else(|| trimmed.strip_prefix("/Duration "))
        {
            self.default_duration = Some(value.trim().to_string());
            return Ok(());
        }

        self.settings.parse_setting_line(line)
    }

    /// Parse a global chord assignment line (e.g., "Cm = Cm7b5", "C = Cmaj7")
    ///
    /// These assignments define chord memory that applies to all sections.
    /// Format: `BasicChord = FullChord` where both must look like chord symbols.
    ///
    /// Returns `Some((basic_chord, full_chord))` if valid, `None` otherwise.
    pub(super) fn parse_chord_assignment(line: &str) -> Option<(String, String)> {
        // Must contain '=' to be an assignment
        if !line.contains('=') {
            return None;
        }

        let parts: Vec<&str> = line.split('=').map(|s| s.trim()).collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return None;
        }

        let basic = parts[0];
        let full = parts[1];

        // Validate both look like chord symbols (start with A-G, optionally with accidental)
        let first_char = basic.chars().next()?;
        if !first_char.is_ascii_uppercase() || !('A'..='G').contains(&first_char) {
            return None;
        }

        let full_first_char = full.chars().next()?;
        if !full_first_char.is_ascii_uppercase() || !('A'..='G').contains(&full_first_char) {
            return None;
        }

        // Make sure this isn't a setting line (those start with /)
        if basic.starts_with('/') {
            return None;
        }

        Some((basic.to_string(), full.to_string()))
    }

    /// Check if a line is a pure metadata line (only time sig, key, and/or tempo - no title text)
    /// Examples: "4/4 #G", "120bpm 4/4", "#C"
    pub(super) fn looks_like_pure_metadata_line(line: &str) -> bool {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return false;
        }

        // All parts must be recognized as metadata tokens
        for part in &parts {
            // A bare number 1-7 is a Nashville scale degree (a chord), not a
            // tempo — real tempos are larger. Without this, `1 4 6 5` reads as a
            // pure-metadata line and is consumed before the chord check, so the
            // chart renders nothing.
            let is_tempo =
                part.ends_with("bpm") || part.parse::<u32>().is_ok_and(|n| !(1..=7).contains(&n));
            let is_time_sig = Self::parse_time_signature(part).is_some();
            let is_key =
                (part.starts_with('#') || part.starts_with('b')) && Key::parse(part).is_ok();

            if !is_tempo && !is_time_sig && !is_key {
                return false;
            }
        }

        true
    }

    /// Check if a line looks like chord content (starts with chord symbols or scale degrees)
    /// Examples: "C Am G D", "1 4 5 1", "Cmaj7/// Dm7///"
    pub(super) fn looks_like_chord_content(line: &str) -> bool {
        let first_token = line.split_whitespace().next().unwrap_or("");
        if first_token.is_empty() {
            return false;
        }

        // Check for scale degree (1-7 optionally followed by modifiers)
        // But NOT time signatures like "4/4" or "6/8"
        if let Some(first_char) = first_token.chars().next() {
            if first_char.is_ascii_digit() && ('1'..='7').contains(&first_char) {
                // Make sure it's not a time signature (digit/digit pattern)
                if Self::parse_time_signature(first_token).is_some() {
                    return false;
                }
                return true;
            }
        }

        // Check for chord symbol pattern: starts with A-G, optionally with # or b
        let chord_start = first_token.chars().next().unwrap_or(' ');
        if matches!(chord_start, 'A'..='G') {
            // Could be a chord like "C", "Am", "Cmaj7"
            // But also could be a word in a title - check if it has chord-like modifiers
            let rest = &first_token[1..];
            // Common chord modifiers: m, maj, min, 7, 9, sus, dim, aug, +, #, b, /
            if rest.is_empty()
                || rest.starts_with('m')
                || rest.starts_with("maj")
                || rest.starts_with("min")
                || rest.starts_with("dim")
                || rest.starts_with("aug")
                || rest.starts_with('+')
                || rest.starts_with("sus")
                || rest.starts_with('7')
                || rest.starts_with('9')
                || rest.starts_with("11")
                || rest.starts_with("13")
                || rest.starts_with('#')
                || rest.starts_with('b')
                || rest.starts_with('/')
                || rest.starts_with('_')
            {
                return true;
            }
        }

        false
    }

    /// Continue parsing metadata after a pure metadata line was found
    fn continue_metadata_parsing(
        &mut self,
        lines: &[&str],
        start_idx: usize,
    ) -> Result<usize, String> {
        let mut idx = start_idx;

        // Skip empty lines and parse settings
        while idx < lines.len() {
            if lines[idx].is_empty() {
                idx += 1;
                continue;
            }

            // Check for settings (lines starting with /)
            if lines[idx].starts_with('/') {
                self.parse_setting(lines[idx])?;
                idx += 1;
                continue;
            }

            break;
        }

        Ok(idx)
    }
}

// endregion: --- Metadata Parsing
