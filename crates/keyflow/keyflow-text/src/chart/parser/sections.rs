//! Section parsing for charts
//!
//! Handles parsing of chart sections including verse, chorus, bridge, etc.
//! Also handles track grouping and section content parsing.

use super::ChartParser;
use crate::chart::melody::Melody;
use crate::chart::track::{Track, TrackType};
use crate::chart::types::{ChartSection, Measure, RhythmElement};
use crate::primitives::RootNotation;
use crate::sections::{Section, SectionType};
use crate::time::{AbsolutePosition, MusicalDuration, TimeSignature, TimeSignatureExt};

// region:    --- Section Parsing

/// Check if a line looks like chord content (bars, chords, slashes, rests, etc.)
/// This helps detect section-less content that should still be parsed.
///
/// This function is conservative to avoid false positives:
/// - Requires bar delimiter (|) OR valid chord-like first token
/// - Won't match titles (multi-word text), metadata lines (bpm, time sig, key)
fn looks_like_chord_content(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Starts with pushed chord marker - definitely chord content
    if trimmed.starts_with('\'') || trimmed.starts_with('>') {
        return true;
    }

    // Has measure bar delimiter - definitely chord content
    // But not if it's a time signature like "120bpm 4/4 #C"
    if trimmed.contains('|') {
        return true;
    }

    // Get first token to analyze
    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    if first_token.is_empty() {
        return false;
    }

    // Check if this looks like metadata (has bpm, time signature pattern, or key).
    // A bare number is only metadata when it ISN'T a Nashville scale degree
    // (1–7) — `120` is a tempo, but `1 4 6 5` is music, so it must read as
    // chord content (otherwise a chart can't start with bare number-system
    // chords and you'd be forced to add a title/meter line first).
    if first_token.ends_with("bpm")
        || first_token
            .parse::<u32>()
            .is_ok_and(|n| !(1..=7).contains(&n))
        || first_token.contains('/')
        || first_token.starts_with('#')
        || first_token.starts_with('b')
    {
        return false;
    }

    // Multi-word lines that start with A-G are likely titles, not chord content
    // e.g., "Empty Section Test" or "Every Breath You Take"
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() > 2 {
        // If there are multiple words that are NOT chord-like, it's probably a title
        // Chord lines with multiple words usually have chord symbols with modifiers
        let non_chord_words = words
            .iter()
            .filter(|w| {
                let c = w.chars().next().unwrap_or(' ');
                // Not A-G, not a scale degree (1-7), not a slash/continuation.
                !matches!(c, 'A'..='G' | '1'..='7' | '/' | '.' | '\'' | '>')
            })
            .count();
        if non_chord_words > 1 {
            return false; // Too many non-chord words, likely a title
        }
    }

    // Check if first character is a chord root.
    let first_char = first_token.chars().next().unwrap_or(' ');
    // Nashville scale-degree root (1–7) — a bare number-system chord like `1`,
    // `4`, `6m`. The metadata check above already let single digits 1–7 through.
    if matches!(first_char, '1'..='7') {
        return true;
    }
    if matches!(first_char, 'A'..='G') {
        // Make sure it looks like a chord, not just a word starting with A-G
        // Single letter or has chord-like suffix
        if first_token.len() == 1 {
            return true; // Single letter chord like "C", "G"
        }
        let rest = &first_token[1..];
        // Check for chord modifiers
        if rest.starts_with('m')
            || rest.starts_with('#')
            || rest.starts_with('b')
            || rest.starts_with('/')
            || rest.starts_with('+')
            || rest.starts_with("maj")
            || rest.starts_with("dim")
            || rest.starts_with("aug")
            || rest.starts_with("sus")
            || rest.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
    }

    false
}

impl<'a> ChartParser<'a> {
    fn empty_measure_with_active_time_signature(&self) -> Measure {
        let time_sig = self.time_signature.unwrap_or(TimeSignature::common_time());
        let mut measure = Measure::new();
        measure.time_signature = (time_sig.numerator as u8, time_sig.denominator as u8);
        measure
    }

    /// Phase 2: Parse sections and content
    pub(super) fn parse_sections(
        &mut self,
        lines: &[&str],
        start_idx: usize,
    ) -> Result<usize, String> {
        let mut idx = start_idx;

        while idx < lines.len() {
            let line = lines[idx];

            // Skip empty lines
            if line.is_empty() {
                idx += 1;
                continue;
            }

            if let Some((name, value)) = Self::parse_alias_declaration(line) {
                self.aliases.insert(name, value);
                idx += 1;
                continue;
            }

            // Split potential inline content by colon or comma
            // Colon syntax: "VS 8: Cm | Fm |" - section with inline measures
            // Comma syntax: "VS 8, Cm | Fm |" - alternative syntax
            let (marker_part, inline_content) = if let Some(colon_idx) = line.find(':') {
                let (marker, content) = line.split_at(colon_idx);
                let content = content[1..].trim();
                // Only treat as inline content if there's actual content after colon
                if content.is_empty() {
                    (line.trim(), None)
                } else {
                    (marker.trim(), Some(content))
                }
            } else if let Some(comma_idx) = line.find(',') {
                let (marker, content) = line.split_at(comma_idx);
                (marker.trim(), Some(content[1..].trim()))
            } else {
                (line.trim(), None)
            };

            // Check for "pre" or "post" special handling (based on marker part only)
            let line_lower = marker_part.to_lowercase();
            if line_lower == "pre" || line_lower.starts_with("pre ") {
                idx = self.parse_pre_section(lines, idx)?;
                continue;
            }
            if line_lower == "post" || line_lower.starts_with("post ") {
                idx = self.parse_post_section(lines, idx)?;
                continue;
            }

            // Check for subsection prefix (^)
            let (is_subsection, section_marker) = if marker_part.starts_with('^') {
                (true, marker_part.strip_prefix('^').unwrap_or(marker_part))
            } else {
                (false, marker_part)
            };

            // Check if this is a section marker (based on marker part only)
            if let Some(parsed) = SectionType::parse_with_measure_count(section_marker) {
                let section_type = parsed.section_type;
                let measure_expr = parsed.measure_expr;
                let section_comment = parsed.comment;

                // A key change written on the header line (`BR 8 #G`) takes
                // effect at the start of this section, before its chords parse,
                // so scale-degree chords resolve in the new key and the key
                // signature updates here.
                if let Some(key_token) = parsed.key_change.as_ref() {
                    if let Ok(new_key) = crate::key::Key::parse(key_token) {
                        let section_index = self.chart.sections.len();
                        let position =
                            AbsolutePosition::new(MusicalDuration::new(0, 0, 0), section_index);
                        let key_change = crate::chart::types::KeyChange::new(
                            position,
                            self.current_key.clone(),
                            new_key.clone(),
                            section_index,
                        );
                        self.key_changes.push(key_change);
                        self.current_key = Some(new_key);
                    }
                }

                // Resolve the measure expression using section memory
                let measure_count = if let Some(expr) = measure_expr.as_ref() {
                    self.resolve_measure_expression(&section_type, expr)
                } else {
                    // No expression provided - use section memory if available
                    self.section_measure_memory.get(&section_type).copied()
                };

                if let Some(content) = inline_content {
                    // Handle inline content separated by comma
                    let mut section =
                        Section::new(section_type.clone()).with_subsection(is_subsection);
                    if let Some(count) = measure_count {
                        section.measure_count = Some(count);
                    }
                    if let Some(comment) = section_comment.clone() {
                        section.comment = Some(comment);
                    }

                    let measures = if content.is_empty() {
                        // Empty inline content
                        if let Some(count) = measure_count {
                            vec![self.empty_measure_with_active_time_signature(); count]
                        } else {
                            self.templates
                                .recall_transposed(&section_type, self.current_key.as_ref())
                                .unwrap_or_default()
                        }
                    } else {
                        // Has inline content - enter section (clears section memory for non-first sections)
                        let was_first_section = self.chord_memory.in_first_section();
                        self.chord_memory.enter_section();
                        let mut parsed_measures =
                            self.parse_section_measures(&[content], &section_type, measure_count)?;

                        // Pad with empty measures if fewer than expected count
                        // This ensures "Count 2: | |" creates 2 empty measures
                        if let Some(count) = measure_count {
                            while parsed_measures.len() < count {
                                parsed_measures
                                    .push(self.empty_measure_with_active_time_signature());
                            }
                        }

                        // Save as template if not Intro/Outro/Pre/Post
                        if !matches!(
                            section_type,
                            SectionType::Intro
                                | SectionType::Outro
                                | SectionType::Pre(_)
                                | SectionType::Post(_)
                        ) {
                            let current_key = self.current_key.clone();
                            self.templates.store(
                                &section_type,
                                &parsed_measures,
                                current_key.as_ref(),
                            );
                        }

                        // Complete first section if this was the first section
                        if was_first_section {
                            self.chord_memory.complete_first_section();
                        }

                        parsed_measures
                    };

                    let chart_section = ChartSection::new(section).with_measures(measures);
                    self.sections.push(chart_section);
                    idx += 1;
                } else {
                    // Parse section content from subsequent lines
                    idx = self.parse_section_content(
                        lines,
                        idx,
                        section_type,
                        measure_count,
                        is_subsection,
                        section_comment,
                    )?;
                }
            } else if looks_like_chord_content(line) {
                // Content without a section header - create an unnamed section
                // This allows charts like "Cm | Fm |" without requiring "IN" or "VS" prefix

                // Collect consecutive chord content lines
                let mut content_lines = Vec::new();
                while idx < lines.len() {
                    let content_line = lines[idx];

                    // Stop if empty line or new section marker
                    if content_line.is_empty() {
                        idx += 1;
                        break;
                    }

                    // Check if this is a new section marker
                    let (marker_part, _) = if let Some(comma_idx) = content_line.find(',') {
                        let (marker, content) = content_line.split_at(comma_idx);
                        (marker.trim(), Some(content[1..].trim()))
                    } else {
                        (content_line.trim(), None)
                    };
                    if SectionType::parse_with_measure_count(marker_part).is_some() {
                        break;
                    }

                    let line_lower = content_line.to_lowercase();
                    if line_lower == "pre"
                        || line_lower.starts_with("pre ")
                        || line_lower == "post"
                        || line_lower.starts_with("post ")
                    {
                        break;
                    }

                    // Collect this line as content
                    content_lines.push(content_line);
                    idx += 1;
                }

                // Parse the collected content as an Intro section
                if !content_lines.is_empty() {
                    let section_type = SectionType::Intro;
                    let was_first_section = self.chord_memory.in_first_section();
                    self.chord_memory.enter_section();

                    let measures =
                        self.parse_section_measures(&content_lines, &section_type, None)?;

                    // Complete first section if this was the first section
                    if was_first_section {
                        self.chord_memory.complete_first_section();
                    }

                    let section = Section::new(section_type);
                    // No section header was written — this Intro is synthesized
                    // so the engraver/editor don't show a bogus "Intro" label.
                    let chart_section = ChartSection::new(section)
                        .with_measures(measures)
                        .as_implicit();
                    self.sections.push(chart_section);
                }
            } else {
                // Not a recognized line or chord content, skip it
                idx += 1;
            }
        }

        Ok(idx)
    }

    /// Parse a track marker line like "[melody]" or "[melody lead]"
    /// Returns (track_type, optional_name, remaining_content)
    pub(super) fn parse_track_marker(line: &str) -> Option<(TrackType, Option<String>, &str)> {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            return None;
        }

        let close_bracket = trimmed.find(']')?;
        let marker_content = &trimmed[1..close_bracket];
        let remaining = trimmed[close_bracket + 1..].trim();

        let (track_type, name) = TrackType::parse(marker_content)?;
        Some((track_type, name, remaining))
    }

    /// Group content lines by track type
    /// Returns a Vec of (TrackType, Option<name>, Vec<lines>)
    pub(super) fn group_lines_by_track<'b>(
        &self,
        lines: &[&'b str],
    ) -> Vec<(TrackType, Option<String>, Vec<&'b str>)> {
        let mut groups: Vec<(TrackType, Option<String>, Vec<&'b str>)> = Vec::new();
        let mut current_type = TrackType::Chords;
        let mut current_name: Option<String> = None;
        let mut current_lines: Vec<&'b str> = Vec::new();

        for line in lines {
            if let Some((track_type, name, remaining)) = Self::parse_track_marker(line) {
                // Save current group if it has content
                if !current_lines.is_empty() {
                    groups.push((
                        current_type,
                        current_name.take(),
                        std::mem::take(&mut current_lines),
                    ));
                }

                // Start new track
                current_type = track_type;
                current_name = name;

                // If there's content after the marker on the same line, add it
                if !remaining.is_empty() {
                    current_lines.push(remaining);
                }
            } else {
                current_lines.push(line);
            }
        }

        // Don't forget the last group
        if !current_lines.is_empty() {
            groups.push((current_type, current_name, current_lines));
        }

        groups
    }

    /// Parse a section and its content
    pub(super) fn parse_section_content(
        &mut self,
        lines: &[&str],
        start_idx: usize,
        section_type: SectionType,
        measure_count: Option<usize>,
        is_subsection: bool,
        comment: Option<String>,
    ) -> Result<usize, String> {
        use crate::chord::{ChordRhythm, LilySyntax};

        let mut idx = start_idx + 1; // Skip section marker line
        let mut section = Section::new(section_type.clone()).with_subsection(is_subsection);
        if let Some(count) = measure_count {
            section.measure_count = Some(count);
        }
        if let Some(c) = comment {
            section.comment = Some(c);
        }

        // Collect content lines for this section
        let mut content_lines = Vec::new();

        // Peek at next line to see if it's content or another section
        while idx < lines.len() {
            let line = lines[idx];

            // If empty line, check what's after it
            if line.is_empty() {
                idx += 1;
                continue;
            }

            // If this looks like a new section or pre/post, stop collecting content
            let (marker_part, _) = if let Some(comma_idx) = line.find(',') {
                let (marker, content) = line.split_at(comma_idx);
                (marker.trim(), Some(content[1..].trim()))
            } else {
                (line.trim(), None)
            };
            if SectionType::parse_with_measure_count(marker_part).is_some() {
                break;
            }

            let line_lower = line.to_lowercase();
            if line_lower == "pre"
                || line_lower.starts_with("pre ")
                || line_lower == "post"
                || line_lower.starts_with("post ")
            {
                break;
            }

            // This is content for the current section
            content_lines.push(line);
            idx += 1;
        }

        // Extract and apply section-scoped settings
        // Settings inside a section are temporary - they reset after the section ends
        let settings_checkpoint = self.settings.checkpoint();
        let mut actual_content_lines = Vec::new();
        let mut has_section_settings = false;

        for line in &content_lines {
            let trimmed = line.trim();
            // Check if this looks like a settings line:
            // - Starts with '/' and contains '=' (e.g., "/PUSH=8t")
            // - Starts with '/push ' (space-separated syntax, e.g., "/push 4")
            let is_setting_line = trimmed.starts_with('/')
                && (trimmed.contains('=')
                    || trimmed.to_lowercase().starts_with("/push ")
                    || trimmed.to_lowercase().starts_with("/chordlength "));

            if is_setting_line {
                if !trimmed.to_lowercase().starts_with("/chordlength ") {
                    // This is a settings line - apply it temporarily
                    if let Err(e) = self.settings.parse_setting_line(line) {
                        tracing::warn!("Failed to parse section setting '{}': {}", line, e);
                    } else {
                        has_section_settings = true;
                    }
                }
                actual_content_lines.push(*line);
            } else {
                actual_content_lines.push(*line);
            }
        }

        // Group content lines by track type (excluding settings lines)
        let track_groups = self.group_lines_by_track(&actual_content_lines);

        // Parse each track group into tracks
        let mut tracks: Vec<Track> = Vec::new();

        if track_groups.is_empty() {
            // No explicit content - prefer template recall over empty measures
            let template_measures = self
                .templates
                .recall_transposed(&section_type, self.current_key.as_ref());

            let measures = if let Some(mut template) = template_measures {
                // Have a template - optionally adjust to measure_count
                if let Some(count) = measure_count {
                    if template.len() > count {
                        template.truncate(count);
                    }
                    // Note: if template is shorter, padding is handled below like for explicit content
                }
                template
            } else if let Some(count) = measure_count {
                // No template, but have measure count - create empty measures
                vec![self.empty_measure_with_active_time_signature(); count]
            } else {
                Vec::new()
            };
            if !measures.is_empty() {
                tracks.push(Track::chords(measures));
            }
        } else {
            // Has explicit content - enter section (clears section memory for non-first sections)
            let was_first_section = self.chord_memory.in_first_section();
            self.chord_memory.enter_section();

            for (track_type, track_name, track_lines) in track_groups {
                match track_type {
                    TrackType::Chords | TrackType::Rhythm => {
                        // Parse as chord/rhythm content
                        let mut parsed_measures = self.parse_section_measures(
                            &track_lines,
                            &section_type,
                            measure_count,
                        )?;

                        // Apply padding if this is the chord track and measure count is specified
                        if track_type == TrackType::Chords {
                            if let Some(count) = measure_count {
                                let actual_measures = parsed_measures.len();
                                if actual_measures < count {
                                    let padding_needed = count - actual_measures;
                                    let time_sig =
                                        self.time_signature.unwrap_or(TimeSignature::common_time());

                                    for _ in 0..padding_needed {
                                        // Create a "space chord" for padding
                                        // This is a ChordInstance with symbol "s" representing
                                        // an empty measure that will be filled with slashes
                                        let space_duration = MusicalDuration::new(1, 0, 0); // 1 measure
                                        let note = crate::primitives::MusicalNote::from_string("C")
                                            .unwrap();
                                        let root_notation = RootNotation::from_note_name(note);
                                        let space_chord = crate::chart::types::ChordInstance::new(
                                            root_notation.clone(),
                                            "s".to_string(),
                                            crate::chord::Chord::new(
                                                root_notation,
                                                crate::chord::ChordQuality::Major,
                                            ),
                                            ChordRhythm::space(
                                                LilySyntax::Whole,
                                                false,
                                                false,
                                                None,
                                            ),
                                            "s".to_string(),
                                            space_duration,
                                            AbsolutePosition::at_beginning(),
                                        );

                                        let mut space_measure = Measure::new();
                                        space_measure.time_signature =
                                            (time_sig.numerator as u8, time_sig.denominator as u8);
                                        // Add to rhythm_elements (post-processor will sync to chords)
                                        space_measure
                                            .rhythm_elements
                                            .push(RhythmElement::Chord(space_chord));
                                        parsed_measures.push(space_measure);
                                    }
                                } else if actual_measures > count {
                                    return Err(format!(
                                        "Section {:?} has {} parsed measures but header specifies {}. \
                                         Add explicit durations or split the content so the parsed measure count matches the section header.",
                                        section_type, actual_measures, count
                                    ));
                                }
                            }

                            // Save as template if not Intro/Outro/Pre/Post
                            if !matches!(
                                section_type,
                                SectionType::Intro
                                    | SectionType::Outro
                                    | SectionType::Pre(_)
                                    | SectionType::Post(_)
                            ) {
                                let current_key = self.current_key.clone();
                                self.templates.store(
                                    &section_type,
                                    &parsed_measures,
                                    current_key.as_ref(),
                                );
                            }
                        }

                        let mut track = if track_type == TrackType::Chords {
                            Track::chords(parsed_measures)
                        } else {
                            Track::rhythm(parsed_measures)
                        };

                        if let Some(name) = track_name {
                            track = track.with_name(name);
                        }
                        tracks.push(track);
                    }
                    TrackType::Melody => {
                        // Parse melody content - join all lines and parse as melody
                        let combined = track_lines.join(" ");
                        if let Ok((_, melody)) = Melody::parse_block(&combined) {
                            let mut track = Track::melody(melody);
                            if let Some(name) = track_name {
                                track = track.with_name(name);
                            }
                            tracks.push(track);
                        }
                    }
                    TrackType::Lyrics => {
                        // Parse lyrics content - join all lines
                        let combined = track_lines.join(" ");
                        let trimmed = combined.trim();
                        if !trimmed.is_empty() {
                            // Try {Chord}syllable format first
                            let lyric_line = if trimmed.contains('{') && trimmed.contains('}') {
                                let parser = crate::chart::LyricChordParser::new();
                                parser.parse(trimmed).unwrap_or_else(|_| {
                                    crate::chart::LyricLine::parse_simple(trimmed)
                                })
                            } else if trimmed.contains('[') && trimmed.contains(']') {
                                // Try [Chord] inline format via SyllableParser
                                crate::chart::SyllableParser::new().parse(trimmed)
                            } else {
                                // Plain text fallback
                                crate::chart::LyricLine::parse_simple(trimmed)
                            };
                            let mut track = Track::lyrics(lyric_line);
                            if let Some(name) = track_name {
                                track = track.with_name(name);
                            }
                            tracks.push(track);
                        }
                    }
                }
            }

            // Complete first section if this was the first section
            if was_first_section {
                self.chord_memory.complete_first_section();
            }
        }

        // Create chart section with tracks
        let mut chart_section = ChartSection::new(section);
        chart_section.tracks = tracks;
        self.sections.push(chart_section);

        // Restore settings to pre-section state if any section settings were applied
        // This ensures settings like /push=4 inside a section don't leak to subsequent sections
        if has_section_settings {
            self.settings.restore(settings_checkpoint);
        }

        Ok(idx)
    }

    /// Parse "pre" section (pre-chorus, pre-verse, etc.)
    pub(super) fn parse_pre_section(
        &mut self,
        lines: &[&str],
        start_idx: usize,
    ) -> Result<usize, String> {
        let line = lines[start_idx];
        let parts: Vec<&str> = line.split_whitespace().collect();

        // Default to Pre-Chorus
        let section_type = SectionType::Pre(Box::new(SectionType::Chorus));
        let measure_count = if parts.len() > 1 {
            parts[1].parse::<usize>().ok()
        } else {
            None
        };

        self.parse_section_content(lines, start_idx, section_type, measure_count, false, None)
    }

    /// Parse "post" section (post-chorus, post-verse, etc.)
    pub(super) fn parse_post_section(
        &mut self,
        lines: &[&str],
        start_idx: usize,
    ) -> Result<usize, String> {
        let line = lines[start_idx];
        let parts: Vec<&str> = line.split_whitespace().collect();

        // Default to Post-Chorus
        let section_type = SectionType::Post(Box::new(SectionType::Chorus));
        let measure_count = if parts.len() > 1 {
            parts[1].parse::<usize>().ok()
        } else {
            None
        };

        self.parse_section_content(lines, start_idx, section_type, measure_count, false, None)
    }
}

// endregion: --- Section Parsing
