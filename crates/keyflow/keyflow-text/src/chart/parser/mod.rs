//! Chart Parser
//!
//! Main parsing logic for charts
//!
//! Three-phase parsing:
//! 1. Metadata phase: Parse title, tempo, time signature, key
//! 2. Content phase: Parse sections and their measures
//! 3. Post-processing: Auto-number sections, finalize positions

// region:    --- Modules

pub mod block;
pub mod chordpro;
mod chordpro_integration;
pub mod chords;
pub mod document;
pub mod helpers;
pub mod metadata;
pub mod post_process;
pub mod sections;

pub use chordpro::parse_chordpro;
pub use chordpro_integration::merge_chordpro_into_chart;
pub use document::parse_kf_document;
pub use helpers::{LineOffsetMap, PushPullModifier, RepeatCount};

// endregion: --- Modules

// region:    --- Parser Entry Point

use super::ChartParser;
use crate::chart::types::Measure;
use crate::chart::{Chart, ChartSection};
use crate::sections::{MeasureExpression, SectionType};
use crate::time::TimeSignatureExt;

impl<'a> ChartParser<'a> {
    /// Strip comments from a line (everything after ;)
    fn strip_comment(line: &str) -> &str {
        let trimmed = line.trim();
        if trimmed.ends_with(';') || trimmed.contains(" ; ") {
            return line;
        }
        if let Some(pos) = line.find(';') {
            line[..pos].trim()
        } else {
            line
        }
    }

    /// Resolve a measure expression and update section memory
    ///
    /// Returns the resolved measure count, or None if the expression can't be resolved.
    /// Only absolute expressions update the memory - relative expressions (+N, -N) use
    /// memory but don't change it, so subsequent sections still use the original base.
    pub(crate) fn resolve_measure_expression(
        &mut self,
        section_type: &crate::sections::SectionType,
        expr: &MeasureExpression,
    ) -> Option<usize> {
        let memory = self.section_measure_memory.get(section_type).copied();
        let resolved = expr.resolve(memory);

        // Only update memory for absolute expressions (not relative +N or -N)
        if let Some(count) = resolved {
            if expr.updates_memory() {
                self.section_measure_memory
                    .insert(section_type.clone(), count);
            }
        }

        resolved
    }

    /// Parse a chart from input string
    pub fn parse(&mut self, input: &str) -> Result<(), String> {
        use crate::key::Key;
        use crate::primitives::MusicalNote;
        use crate::time::TimeSignature;

        // Block-lane format (`sections {}/rhythm {}/lyrics {}`) is transpiled to
        // classic section text first, so the rest of the parser is unchanged.
        // `voice {}` lanes (melody + per-note lyrics) are attached after parsing.
        let is_block = block::is_block_format(input);
        let block_src: Option<&str> = if is_block { Some(input) } else { None };
        let transpiled;
        let input: &str = if is_block {
            transpiled = block::transpile(input)?;
            &transpiled
        } else {
            input
        };

        let raw_lines: Vec<&str> = input
            .lines()
            .map(|l| Self::strip_comment(l.trim()))
            .collect();
        let logical_lines = Self::join_multiline_parallel_containers(&raw_lines);
        let lines: Vec<&str> = logical_lines.iter().map(String::as_str).collect();

        if lines.is_empty() {
            return Err("Empty input".to_string());
        }

        // Phase 1: Parse metadata at the beginning
        let mut line_idx = self.parse_metadata(&lines, 0)?;

        // Phase 1.5: Apply defaults for unspecified metadata
        // Default time signature: 4/4 (common time)
        if self.time_signature.is_none() {
            self.time_signature = Some(TimeSignature::common_time());
            self.initial_time_signature = Some(TimeSignature::common_time());
        }
        // Default key: C major
        if self.current_key.is_none() {
            let c_major = Key::major(MusicalNote::c());
            self.current_key = Some(c_major.clone());
            self.initial_key = Some(c_major);
        }

        // Phase 2: Parse sections and content
        if !self.try_parse_sectioned_lane_parallel(&lines[line_idx..])? {
            line_idx = self.parse_sections(&lines, line_idx)?;
        }

        // Phase 2.5: attach `voice {}` lanes (melody + per-note lyrics) to the
        // sections built above, matching by Custom label.
        if let Some(src) = block_src {
            block::attach_voices(&mut self.sections, src);
        }

        // Phase 3: Post-processing
        self.post_process();

        // Phase 3.5: assign document-absolute source spans to every chord.
        self.assign_chord_source_spans(input);

        // Phase 3.6: assign document-absolute source spans to every explicitly
        // written section header, so the editor can badge it with the resolved
        // name (`VS` → `Verse 1a`).
        self.assign_section_source_spans(input);

        let _ = line_idx; // Suppress unused warning

        Ok(())
    }

    /// Assign **document-absolute** `source_span`s to every parsed chord.
    ///
    /// The line-by-line parser emits each chord's `source_span` relative to its
    /// own chord line — `parse_chord_line_inner`'s `line_byte_offset` is `0` in
    /// production (only the `#[cfg(test)]` path shifts it). Line-relative spans
    /// are useless for mapping a document offset back to a chord, which is
    /// exactly what click-to-highlight (engraver), editor inlay overlays, and
    /// hover all need.
    ///
    /// Rather than thread a doc offset through the whole line pipeline (a large,
    /// risky refactor of the core parser), we recover absolute spans in one pass
    /// here: chords occur in the same order in the chart and in the source, so a
    /// single forward scan that matches each chord's `original_token` against the
    /// document's whitespace/`|`-delimited tokens lines them up. A chord whose
    /// token can't be relocated keeps its prior span and the scan stays aligned
    /// for the rest. (CM6's block parser instead carries an absolute `lineStart`
    /// cursor — `lineStart += line.length + 1` — and emits `lineStart + col`; the
    /// equivalent here would be threading that offset through `parse_sections` →
    /// `parse_section_measures` → `parse_chord_line_inner`.)
    fn assign_chord_source_spans(&mut self, input: &str) {
        use crate::parsing::TextSpan;

        // Tokenize the document on whitespace + measure bars — the boundaries
        // chords sit between.
        let bytes = input.as_bytes();
        let mut tokens: Vec<(usize, &str)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_whitespace() || bytes[i] == b'|' {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'|' {
                i += 1;
            }
            tokens.push((start, &input[start..i]));
        }

        let line_col = |off: usize| -> (u32, u32) {
            let upto = &input[..off];
            let line = upto.bytes().filter(|&b| b == b'\n').count() as u32 + 1;
            let col = (off - upto.rfind('\n').map_or(0, |n| n + 1)) as u32 + 1;
            (line, col)
        };

        let mut ti = 0;
        for section in &mut self.sections {
            for measure in section.measures_mut() {
                for ci in &mut measure.chords {
                    let want = ci.original_token.trim();
                    if want.is_empty() {
                        continue;
                    }
                    while ti < tokens.len() && tokens[ti].1 != want {
                        ti += 1;
                    }
                    let Some(&(off, tok)) = tokens.get(ti) else {
                        return; // ran out of source tokens to match
                    };
                    let (line, col) = line_col(off);
                    ci.source_span = Some(TextSpan::with_location(off, tok.len(), line, col));
                    ti += 1;
                }
            }
        }
    }

    /// Assign **document-absolute** `source_span`s to every explicitly-written
    /// section header (`VS`, `CH 8 #G`, `^BR`, ...).
    ///
    /// Same problem and approach as [`assign_chord_source_spans`]: the line
    /// pipeline trims each line, so by the time sections exist their header
    /// offsets are gone. We recover them by scanning the raw `input` for header
    /// lines in document order and lining each up with the next chart section
    /// of the same type. Sections with no written header — implicit Intros
    /// synthesized from bare chord content — are skipped (they keep
    /// `source_span = None`, so no name badge is drawn). Header lines that don't
    /// correspond to a section (or whose type doesn't match) are skipped over,
    /// so an extra/recalled header never desyncs the rest.
    fn assign_section_source_spans(&mut self, input: &str) {
        use crate::parsing::TextSpan;
        use crate::sections::SectionType;

        // Collect (start, len, line, col, type_key) for each section-header line.
        let mut headers: Vec<(usize, usize, u32, u32, String)> = Vec::new();
        let mut offset = 0usize;
        let mut line_no = 1u32;
        for line in input.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = content.strip_suffix('\r').unwrap_or(content);
            let trimmed = content.trim();
            // The marker is everything before a `,` comment; strip a `^`
            // subsection prefix the way the section parser does.
            let marker = match trimmed.find(',') {
                Some(c) => trimmed[..c].trim(),
                None => trimmed,
            };
            let marker = marker.strip_prefix('^').unwrap_or(marker);
            if let Some(parsed) = (!marker.is_empty())
                .then(|| SectionType::parse_with_measure_count(marker))
                .flatten()
            {
                // Span the whole trimmed header line (marker + any comment/key),
                // so the badge lands at end of line rather than mid-header.
                let lead = content.len() - content.trim_start().len();
                let start = offset + lead;
                let end = offset + content.trim_end().len();
                let col = lead as u32 + 1;
                headers.push((start, end - start, line_no, col, parsed.section_type.key()));
            }
            offset += line.len();
            line_no += 1;
        }

        let mut hi = 0;
        for section in &mut self.sections {
            if section.implicit {
                continue;
            }
            let want = section.section.section_type.key();
            while hi < headers.len() && headers[hi].4 != want {
                hi += 1;
            }
            let Some(h) = headers.get(hi) else { break };
            section.source_span = Some(TextSpan::with_location(h.0, h.1, h.2, h.3));
            hi += 1;
        }
    }

    fn try_parse_sectioned_lane_parallel(&mut self, lines: &[&str]) -> Result<bool, String> {
        let content_lines = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let Some(parallel_idx) = content_lines
            .iter()
            .position(|line| line.starts_with("<<") && line.ends_with(">>"))
        else {
            return Ok(false);
        };
        if parallel_idx + 1 != content_lines.len() {
            return Ok(false);
        }
        let mut toc_lines = Vec::new();
        for line in &content_lines[..parallel_idx] {
            if let Some((name, value)) = Self::parse_alias_declaration(line) {
                self.aliases.insert(name, value);
            } else {
                toc_lines.push(*line);
            }
        }

        let trimmed = content_lines[parallel_idx];
        if !trimmed.starts_with("<<") || !trimmed.ends_with(">>") {
            return Ok(false);
        }

        let inner = trimmed
            .strip_prefix("<<")
            .and_then(|s| s.strip_suffix(">>"))
            .unwrap_or("")
            .trim();
        let branches = Self::split_top_level_parallel_branches_spanned(inner);
        // A chords-only chart joins a single lane (`<< <chords> >>`); multi-lane
        // joins add `; <melody>` etc. An empty join is never a lane join. The
        // alias-ref validation below still rejects inline-content parallels like
        // `<< C#m ; m { ... } >>`, so single literal branches fall back to
        // section parsing rather than being mistaken for a one-lane join.
        if branches.is_empty() {
            return Ok(false);
        }

        // Validate that every branch is an `<alias>` reference to a known alias
        // BEFORE committing to the lane-parallel interpretation. An inline
        // parallel block like `<< C#m B/C# ; m { ... } >>` has literal content
        // branches, not alias refs — that is a normal measure, not a whole-chart
        // lane join, so fall back to section parsing. Doing this before
        // `section_plan_from_toc` avoids surfacing a confusing TOC error for a
        // section that merely contains an inline parallel (e.g. after
        // `/Duration`).
        let mut resolved_branches = Vec::with_capacity(branches.len());
        for (_, branch) in &branches {
            let branch = branch.trim();
            let Some(alias_name) = branch
                .strip_prefix('<')
                .and_then(|token| token.strip_suffix('>'))
            else {
                return Ok(false);
            };
            let Some(content) = self.aliases.get(alias_name).cloned() else {
                return Ok(false);
            };
            resolved_branches.push((alias_name.to_string(), content));
        }

        // The section plan (table of contents) owns the measure counts. It can
        // be written explicitly above the lane join, or — when omitted — derived
        // from the section headers of the first lane, which must then carry the
        // counts itself.
        let section_plan = if toc_lines.is_empty() {
            Self::section_plan_from_lane(&resolved_branches[0].1)?
        } else {
            Self::section_plan_from_toc(&toc_lines)?
        };

        let mut merged_sections: Option<Vec<ChartSection>> = None;
        for (alias_name, content) in &resolved_branches {
            let alias_name = alias_name.as_str();
            let lane_sections =
                self.parse_sectioned_lane_alias(alias_name, content, &section_plan)?;
            if let Some(existing) = merged_sections.as_mut() {
                Self::merge_section_lanes(existing, lane_sections, alias_name, false)?;
            } else {
                merged_sections = Some(lane_sections);
            }
        }

        self.sections = merged_sections.unwrap_or_default();
        Ok(true)
    }

    fn parse_sectioned_lane_alias(
        &self,
        alias_name: &str,
        content: &str,
        section_plan: &[String],
    ) -> Result<Vec<ChartSection>, String> {
        let content = Self::apply_section_plan_to_lane(content, section_plan)?;
        let content = if alias_name == "melody" {
            Self::melody_mode_content(&content)
        } else {
            content
        };
        let mut lane_chart = Chart::new();
        lane_chart.metadata = self.metadata.clone();
        lane_chart.tempo = self.tempo.clone();
        lane_chart.time_signature = self.time_signature;
        lane_chart.initial_time_signature = self.initial_time_signature;
        lane_chart.current_key = self.current_key.clone();
        lane_chart.initial_key = self.initial_key.clone();
        lane_chart.settings = self.settings.clone();

        let raw_lines = content.lines().map(str::trim).collect::<Vec<_>>();
        let logical_lines = Self::join_multiline_parallel_containers(&raw_lines);
        let lines = logical_lines.iter().map(String::as_str).collect::<Vec<_>>();

        let mut parser = ChartParser::new(&mut lane_chart);
        parser.aliases = self.aliases.clone();
        parser.melody_octave_memory = self.melody_octave_memory;
        parser.default_duration = self.default_duration.clone();
        parser.parse_sections(&lines, 0)?;
        parser.post_process();

        if parser.sections.is_empty() {
            return Err(format!(
                "sectioned lane alias <{alias_name}> has no sections"
            ));
        }

        Ok(lane_chart.sections)
    }

    /// Derive a section plan from a lane's own content when no explicit
    /// table of contents was written above the lane join. Each section header
    /// found in the lane must include a measure count, since there is no TOC to
    /// supply it.
    fn section_plan_from_lane(content: &str) -> Result<Vec<String>, String> {
        let mut plan = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(parsed) = SectionType::parse_with_measure_count(trimmed) {
                if parsed.measure_expr.is_none() {
                    return Err(format!(
                        "Lane section header '{trimmed}' must include a measure count \
                         when there is no table of contents above the lane join"
                    ));
                }
                plan.push(trimmed.to_string());
            }
        }
        if plan.is_empty() {
            return Err(
                "Lane parallel has no section headers and no table of contents".to_string(),
            );
        }
        Ok(plan)
    }

    fn section_plan_from_toc(lines: &[&str]) -> Result<Vec<String>, String> {
        let mut plan = Vec::new();
        for line in lines {
            let Some(parsed) = SectionType::parse_with_measure_count(line) else {
                return Err(format!(
                    "Expected section table-of-contents entry before lane parallel, got '{line}'"
                ));
            };
            if parsed.measure_expr.is_none() {
                return Err(format!(
                    "Section table-of-contents entry '{line}' must include a measure count"
                ));
            }
            plan.push((*line).to_string());
        }
        Ok(plan)
    }

    fn apply_section_plan_to_lane(
        content: &str,
        section_plan: &[String],
    ) -> Result<String, String> {
        let mut section_idx = 0usize;
        let mut output = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                output.push(String::new());
                continue;
            }
            if SectionType::parse_with_measure_count(trimmed).is_some() {
                let Some(planned) = section_plan.get(section_idx) else {
                    return Err(format!(
                        "Lane has extra section '{trimmed}' beyond table of contents"
                    ));
                };
                let planned_type = SectionType::parse_with_measure_count(planned)
                    .map(|parsed| parsed.section_type);
                let lane_type = SectionType::parse_with_measure_count(trimmed)
                    .map(|parsed| parsed.section_type);
                if planned_type != lane_type {
                    return Err(format!(
                        "Lane section '{trimmed}' does not match table-of-contents section '{}'",
                        planned
                    ));
                }
                output.push(planned.clone());
                section_idx += 1;
            } else {
                output.push(trimmed.to_string());
            }
        }
        if section_idx != section_plan.len() {
            return Err(format!(
                "Lane has {} sections, expected {} from table of contents",
                section_idx,
                section_plan.len()
            ));
        }
        Ok(output.join("\n"))
    }

    fn melody_mode_content(content: &str) -> String {
        content
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty()
                    || trimmed.starts_with('/')
                    || trimmed.starts_with("dyn ")
                    || trimmed.starts_with("dynamic ")
                    || trimmed.starts_with("hairpin ")
                    || trimmed.starts_with('@')
                    || trimmed.starts_with('"')
                    || trimmed.starts_with("^\"")
                    || trimmed.starts_with("_\"")
                    || trimmed.starts_with("m{")
                    || trimmed.starts_with("m {")
                    || trimmed.chars().all(|ch| ch == '|' || ch.is_whitespace())
                    || crate::sections::SectionType::parse_with_measure_count(trimmed).is_some()
                {
                    trimmed.to_string()
                } else {
                    format!("m {{ {trimmed} }}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn merge_section_lanes(
        base: &mut [ChartSection],
        lane: Vec<ChartSection>,
        alias_name: &str,
        merge_chords_and_rhythm: bool,
    ) -> Result<(), String> {
        if base.len() != lane.len() {
            return Err(format!(
                "sectioned lane <{alias_name}> has {} sections, expected {}",
                lane.len(),
                base.len()
            ));
        }

        for (idx, (base_section, lane_section)) in base.iter_mut().zip(lane).enumerate() {
            if base_section.section.section_type != lane_section.section.section_type
                || base_section.section.measure_count != lane_section.section.measure_count
            {
                return Err(format!(
                    "sectioned lane <{alias_name}> section {} does not match: {:?} {:?} vs {:?} {:?}",
                    idx + 1,
                    lane_section.section.section_type,
                    lane_section.section.measure_count,
                    base_section.section.section_type,
                    base_section.section.measure_count
                ));
            }

            let base_measures = base_section.measures_mut();
            let lane_measures = lane_section.measures();
            if base_measures.len() != lane_measures.len() {
                return Err(format!(
                    "sectioned lane <{alias_name}> section {} has {} measures, expected {}",
                    idx + 1,
                    lane_measures.len(),
                    base_measures.len()
                ));
            }
            for (base_measure, lane_measure) in base_measures.iter_mut().zip(lane_measures) {
                Self::merge_lane_measure(base_measure, lane_measure, merge_chords_and_rhythm);
            }
        }

        Ok(())
    }

    fn merge_lane_measure(base: &mut Measure, lane: &Measure, merge_chords_and_rhythm: bool) {
        if merge_chords_and_rhythm {
            base.chords.extend(lane.chords.clone());
            base.rhythm_elements.extend(lane.rhythm_elements.clone());
            base.rhythm_slashes.extend(lane.rhythm_slashes.clone());
        }
        base.text_cues.extend(lane.text_cues.clone());
        base.staff_text.extend(lane.staff_text.clone());
        base.figured_bass.extend(lane.figured_bass.clone());
        base.dynamics.extend(lane.dynamics.clone());
        base.classical_dynamics
            .extend(lane.classical_dynamics.clone());
        base.hairpins.extend(lane.hairpins.clone());
        base.melodies.extend(lane.melodies.clone());
    }
}

// endregion: --- Parser Entry Point

// region:    --- Tests

#[cfg(test)]
mod tests {
    use crate::chart::parse_chart;
    use crate::chord::ChordRhythm;
    use crate::key::Key;
    use crate::primitives::{MusicalNote, Note, RootNotation};
    use crate::sections::SectionType;
    use crate::time::MusicalDuration;
    use crate::time::MusicalPositionExt;

    #[test]
    fn sub_labelled_first_section_after_a_title_is_not_eaten() {
        // Regression: with a title line present, a sub-labelled first-section
        // header (`CH 3A 4`) used to be swallowed as a "part name" and the chords
        // became a default Intro. It must parse as the Chorus it names.
        let input = "My Song\n4/4 #C\n\nCH 3A 4\nF C G Am\n";
        let chart = parse_chart(input).expect("Failed to parse chart");
        assert_eq!(chart.metadata.title, Some("My Song".to_string()));
        assert_eq!(chart.sections.len(), 1);
        assert_eq!(chart.sections[0].section.section_type, SectionType::Chorus);

        // A quoted comment and a key change on the first header behave the same.
        let chart = parse_chart("My Song\n4/4 #C\n\nBR 8 #G\nEm D\n").unwrap();
        assert_eq!(chart.sections[0].section.section_type, SectionType::Bridge);
    }

    #[test]
    fn chord_source_spans_are_document_absolute() {
        // The two `5` tokens sit at their literal document offsets (20 and 27),
        // not the line-relative offsets the parser emits internally (2 and 9).
        let text = "120bpm 4/4 #C\n\nvs\n1 5 #G 1 5\n";
        let chart = parse_chart(text).expect("parse");
        let mut starts = Vec::new();
        for s in &chart.sections {
            for m in s.measures() {
                for c in &m.chords {
                    if let Some(span) = c.source_span {
                        starts.push(span.start);
                        // The span covers exactly its source token.
                        assert_eq!(&text[span.start..span.end()], c.original_token.trim());
                    }
                }
            }
        }
        assert!(
            starts.contains(&text.find('5').unwrap()),
            "first 5 should be at its doc offset; spans = {starts:?}"
        );
        assert!(
            starts.contains(&text.rfind('5').unwrap()),
            "second 5 should be at its doc offset; spans = {starts:?}"
        );
    }

    #[test]
    fn test_parse_simple_chord_line() {
        let input = r#"
My Song - Artist Name

120bpm 4/4 #C

vs
cmaj7 Dm7 g7 Cmaj7
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Verify metadata
        assert_eq!(chart.metadata.title, Some("My Song".to_string()));
        assert_eq!(chart.metadata.artist, Some("Artist Name".to_string()));
        assert_eq!(chart.tempo.unwrap().bpm, 120.0);
        assert_eq!(chart.time_signature.unwrap().numerator, 4);
        assert_eq!(chart.time_signature.unwrap().denominator, 4);

        // Verify key
        let c_major = Key::major(MusicalNote::c());
        assert_eq!(chart.initial_key, Some(c_major.clone()));
        assert_eq!(chart.current_key, Some(c_major.clone()));

        // Verify sections
        assert_eq!(chart.sections.len(), 1);
        assert_eq!(chart.sections[0].section.section_type, SectionType::Verse);

        // Verify chords parsed (lowercase and uppercase work)
        let measures = &chart.sections[0].measures();
        assert_eq!(measures.len(), 4); // Each chord creates a measure for now

        // Check first chord (lowercase input)
        assert_eq!(measures[0].chords.len(), 1);
        assert_eq!(measures[0].chords[0].full_symbol, "Cmaj7");
    }

    #[test]
    fn test_parse_with_key_change() {
        let input = r#"
Key Change Test

120bpm 4/4 #C

vs
cmaj7 dm7 #G gmaj7 am7
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Verify initial key
        let c_major = Key::major(MusicalNote::c());
        assert_eq!(chart.initial_key, Some(c_major));

        // Verify ending key is G major
        let g_major = Key::major(MusicalNote::g());
        assert_eq!(chart.ending_key, Some(g_major.clone()));
        assert_eq!(chart.current_key, Some(g_major));

        // Verify key change was recorded
        assert_eq!(chart.key_changes.len(), 1);
        assert_eq!(chart.key_changes[0].to_key.root.name(), "G");

        // Verify chords - should have 4 chords (key change token is skipped)
        let measures = &chart.sections[0].measures();
        assert_eq!(measures.len(), 4);
    }

    #[test]
    fn test_parse_multiple_sections_with_key_change() {
        let input = r#"
Multi Section Test

120bpm 4/4 #C

vs
cmaj7 dm7

ch
Gmaj7 am7

br
#D dmaj7 Em7
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 3 sections
        assert_eq!(chart.sections.len(), 3);

        // Verify section types
        assert_eq!(chart.sections[0].section.section_type, SectionType::Verse);
        assert_eq!(chart.sections[1].section.section_type, SectionType::Chorus);
        assert_eq!(chart.sections[2].section.section_type, SectionType::Bridge);

        // Verify ending key is D major
        let d_major = Key::major(MusicalNote::d());
        assert_eq!(chart.ending_key, Some(d_major));

        // Verify key change was recorded
        assert_eq!(chart.key_changes.len(), 1);
        assert_eq!(chart.key_changes[0].to_key.root.name(), "D");
    }

    #[test]
    fn test_chord_memory_across_sections() {
        let input = r#"
Memory Test

120bpm 4/4 #C

vs
cmaj7 dm7 g7

ch
c d g
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Verify verse has full chord symbols
        let verse_measures = &chart.sections[0].measures();
        assert_eq!(verse_measures.len(), 3);

        // Verify chorus chords recall memory
        let chorus_measures = &chart.sections[1].measures();
        assert_eq!(chorus_measures.len(), 3);

        // The chord memory should have remembered qualities from verse
        // Note: The current implementation stores normalized forms
        // We can verify chords were parsed successfully
        assert!(chorus_measures[0].chords[0].full_symbol.contains("C"));
        assert!(chorus_measures[1].chords[0].full_symbol.contains("D"));
        assert!(chorus_measures[2].chords[0].full_symbol.contains("G"));
    }

    #[test]
    fn test_time_signature_in_metadata() {
        let input = r#"
Time Sig Test

140bpm 6/8 #G

vs
gmaj7
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Verify time signature
        assert_eq!(chart.time_signature.unwrap().numerator, 6);
        assert_eq!(chart.time_signature.unwrap().denominator, 8);

        // Verify key
        let g_major = Key::major(MusicalNote::g());
        assert_eq!(chart.initial_key, Some(g_major));
    }

    #[test]
    fn test_section_numbering() {
        let input = r#"
Numbering Test

120bpm 4/4 #C

vs
cmaj7

ch
gmaj7

vs
dm7

ch
am7
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 4 sections
        assert_eq!(chart.sections.len(), 4);

        // Verify auto-numbering
        assert_eq!(chart.sections[0].section.number, Some(1)); // Verse 1
        assert_eq!(chart.sections[1].section.number, Some(1)); // Chorus 1
        assert_eq!(chart.sections[2].section.number, Some(2)); // Verse 2
        assert_eq!(chart.sections[3].section.number, Some(2)); // Chorus 2
    }

    #[test]
    fn test_empty_section_with_measure_count() {
        let input = r#"
Empty Section Test

120bpm 4/4 #C

vs 4
cmaj7 dm7 em7 fmaj7

ch 4
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 2 sections
        assert_eq!(chart.sections.len(), 2);

        // Verse should have 4 measures with chords
        assert_eq!(chart.sections[0].measures().len(), 4);

        // Chorus should have 4 empty measures (template recall placeholder)
        assert_eq!(chart.sections[1].measures().len(), 4);
        assert_eq!(chart.sections[1].section.measure_count, Some(4));
    }

    #[test]
    fn test_parse_section_with_inline_chords() {
        let input = r#"
Inline Chords Test

120bpm 4/4 #C

vs 4, cmaj7 dm7 g7 cmaj7
"#;
        let chart = parse_chart(input).expect("Failed to parse chart");

        assert_eq!(chart.sections.len(), 1);
        let section = &chart.sections[0];
        assert_eq!(section.section.section_type, SectionType::Verse);
        assert_eq!(section.section.measure_count, Some(4));
        assert_eq!(section.measures().len(), 4);
        assert_eq!(section.measures()[0].chords[0].full_symbol, "Cmaj7");
    }

    #[test]
    fn test_sectionless_chord_content() {
        // Content without a section header should still be parsed as an Intro section
        let input = r#"
Sectionless Test

120bpm 4/4 #C

Cm | Fm | Gm | Cm
"#;
        let chart = parse_chart(input).expect("Failed to parse chart");

        assert_eq!(chart.sections.len(), 1);
        let section = &chart.sections[0];
        assert_eq!(section.section.section_type, SectionType::Intro);
        assert_eq!(section.measures().len(), 4);
        assert_eq!(section.measures()[0].chords[0].full_symbol, "Cm");
        assert_eq!(section.measures()[1].chords[0].full_symbol, "Fm");
    }

    #[test]
    fn test_measure_separator() {
        let input = r#"
Measure Separator Test

120bpm 4/4 #C

vs
cmaj7_2 dm7_2 | em7_2 fmaj7_2
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 2 measures (separated by |)
        // Each chord is 2 beats (half note), so 2 chords per measure
        assert_eq!(chart.sections.len(), 1);
        let measures = &chart.sections[0].measures();
        assert_eq!(measures.len(), 2);

        // First measure should have cmaj7 and dm7
        assert_eq!(measures[0].chords.len(), 2);
        assert!(measures[0].chords[0].full_symbol.contains("C"));
        assert!(measures[0].chords[1].full_symbol.contains("D"));

        // Second measure should have em7 and fmaj7
        assert_eq!(measures[1].chords.len(), 2);
        assert!(measures[1].chords[0].full_symbol.contains("E"));
        assert!(measures[1].chords[1].full_symbol.contains("F"));
    }

    #[test]
    fn test_auto_duration_two_chords_between_separators() {
        let input = r#"
Auto Duration Test

120bpm 4/4 #C

vs
| G C |
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 1 measure with 2 chords, each 2 beats (half notes)
        assert_eq!(chart.sections.len(), 1);
        let measures = &chart.sections[0].measures();
        assert_eq!(measures.len(), 1);
        assert_eq!(measures[0].chords.len(), 2);

        // Each chord should be 2 beats (half note in 4/4)
        let time_sig = chart.time_signature.unwrap();
        assert!((measures[0].chords[0].duration.to_beats(time_sig) - 2.0).abs() < 0.001);
        assert!((measures[0].chords[1].duration.to_beats(time_sig) - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_smart_repeat_syntax() {
        let input = r#"
Smart Repeat Test

120bpm 4/4 #C

VS 16
cmaj7 dm7 x^
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        // Should have 16 measures total (2-bar phrase repeated 8 times)
        assert_eq!(chart.sections.len(), 1);
        let measures = &chart.sections[0].measures();

        // The phrase is 2 bars (cmaj7 = 1 bar, dm7 = 1 bar in 4/4)
        // Section is 16 bars, so we need 8 repeats
        // Total: 2 bars * 8 = 16 bars
        assert_eq!(measures.len(), 16);

        // Verify the repeat count is stored on the last measure of the pattern
        // The pattern is 2 measures, so measure[1] (second measure) should have repeat_count = 8
        assert_eq!(
            measures[1].repeat_count,
            8,
            "Repeat count should be 8 on measure 1, but got {}. All measures: {:?}",
            measures[1].repeat_count,
            measures.iter().map(|m| m.repeat_count).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_chord_instance_with_source_span() {
        // Verify ChordInstance can store source_span
        use crate::chord::{Chord, ChordQuality};
        use crate::parsing::TextSpan;

        let root = RootNotation::from_note_name(MusicalNote::c());
        let chord = Chord::new(root.clone(), ChordQuality::Major);
        let span = TextSpan::with_location(10, 5, 3, 8);

        let instance = crate::chart::types::ChordInstance::new(
            root,
            "Cmaj7".to_string(),
            chord,
            ChordRhythm::Default,
            "Cmaj7".to_string(),
            MusicalDuration::new(0, 4, 0),
            crate::time::AbsolutePosition::at_beginning(),
        )
        .with_source_span(span);

        // Verify span is stored
        assert!(instance.source_span.is_some());
        let stored_span = instance.source_span.unwrap();
        assert_eq!(stored_span.start, 10);
        assert_eq!(stored_span.len, 5);
        assert_eq!(stored_span.line, 3);
        assert_eq!(stored_span.column, 8);
    }

    #[test]
    fn test_dot_repeat_with_bar_lines() {
        // With explicit bar lines, each segment is ONE measure
        // "F/C . | Cm ." should be 2 measures, each with 2 half-note chords
        let input = r#"
Dot Repeat Test
120bpm 4/4 #C

VS
F/C . | Cm .
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        let section = &chart.sections[0];
        let measures = section.measures();
        let time_sig = chart.time_signature.unwrap();

        // Should have 2 measures: (F/C, F/C repeat), (Cm, Cm repeat)
        assert_eq!(
            measures.len(),
            2,
            "Expected 2 measures (F/C + dot, Cm + dot), got {}. Chords per measure: {:?}",
            measures.len(),
            measures.iter().map(|m| m.chords.len()).collect::<Vec<_>>()
        );

        // Each measure should have 2 chords of 2 beats each
        for (i, measure) in measures.iter().enumerate() {
            assert_eq!(
                measure.chords.len(),
                2,
                "Measure {} should have 2 chords, got {}",
                i,
                measure.chords.len()
            );
            for (j, chord) in measure.chords.iter().enumerate() {
                let beats = chord.duration.to_beats(time_sig);
                assert!(
                    (beats - 2.0).abs() < 0.001,
                    "Measure {} chord {} should be 2 beats, got {}",
                    i,
                    j,
                    beats
                );
            }
        }

        // Verify chord symbols
        assert_eq!(measures[0].chords[0].full_symbol, "F/C");
        assert_eq!(measures[0].chords[1].full_symbol, "F/C"); // repeat
        assert_eq!(measures[1].chords[0].full_symbol, "Cm");
        assert_eq!(measures[1].chords[1].full_symbol, "Cm"); // repeat
    }

    #[test]
    fn test_dot_repeat_without_bar_lines() {
        // Without explicit bar lines, each chord/dot takes a full measure
        // "F/C ." should be 2 measures, each with 1 whole-note chord
        let input = r#"
Dot Repeat Test
120bpm 4/4 #C

VS
F/C .
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        let section = &chart.sections[0];
        let measures = section.measures();
        let time_sig = chart.time_signature.unwrap();

        // Should have 2 measures: F/C, F/C repeat
        assert_eq!(
            measures.len(),
            2,
            "Expected 2 measures (F/C, then dot repeat), got {}. Chords per measure: {:?}",
            measures.len(),
            measures.iter().map(|m| m.chords.len()).collect::<Vec<_>>()
        );

        // Each measure should have 1 chord of 4 beats
        for (i, measure) in measures.iter().enumerate() {
            assert_eq!(
                measure.chords.len(),
                1,
                "Measure {} should have 1 chord, got {}",
                i,
                measure.chords.len()
            );
            let beats = measure.chords[0].duration.to_beats(time_sig);
            assert!(
                (beats - 4.0).abs() < 0.001,
                "Measure {} chord should be 4 beats, got {}",
                i,
                beats
            );
        }

        // Verify chord symbols
        assert_eq!(measures[0].chords[0].full_symbol, "F/C");
        assert_eq!(measures[1].chords[0].full_symbol, "F/C"); // repeat
    }

    #[test]
    fn test_dot_repeat_clears_push_modifier() {
        // "'F/C ." should be 2 measures:
        // - Measure 0: F/C with push
        // - Measure 1: F/C WITHOUT push (dot repeat clears timing modifiers)
        let input = r#"
Dot Push Test
120bpm 4/4 #C
/push = triplet

VS
'F/C .
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        let section = &chart.sections[0];
        let measures = section.measures();

        // Should have 2 measures
        assert_eq!(
            measures.len(),
            2,
            "Expected 2 measures, got {}. Chords per measure: {:?}",
            measures.len(),
            measures.iter().map(|m| m.chords.len()).collect::<Vec<_>>()
        );

        // Filter out space placeholders ("s" chords added by post-processor)
        let m0_real_chords: Vec<_> = measures[0]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();
        let m1_real_chords: Vec<_> = measures[1]
            .chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .collect();

        // Each measure should have 1 real chord (excluding "s" placeholders)
        assert_eq!(
            m0_real_chords.len(),
            1,
            "Measure 0 should have 1 real chord, got {}",
            m0_real_chords.len()
        );
        assert_eq!(
            m1_real_chords.len(),
            1,
            "Measure 1 should have 1 real chord, got {}",
            m1_real_chords.len()
        );

        // Measure 0 chord SHOULD have push_pull set
        let chord0 = m0_real_chords[0];
        assert!(
            chord0.push_pull.is_some(),
            "Measure 0 chord should have push_pull set"
        );
        let (is_push, _) = chord0.push_pull.as_ref().unwrap();
        assert!(*is_push, "Measure 0 chord should be a push");

        // Measure 1 chord should NOT have push_pull (cleared by dot repeat)
        let chord1 = m1_real_chords[0];
        assert!(
            chord1.push_pull.is_none(),
            "Measure 1 chord (dot repeat) should NOT have push_pull, but got {:?}",
            chord1.push_pull
        );

        // Both should have same symbol
        assert_eq!(chord0.full_symbol, "F/C");
        assert_eq!(chord1.full_symbol, "F/C");
    }

    #[test]
    fn test_slash_rhythm_same_measure() {
        // "Cm/Eb / Eb ///" should be ONE measure with:
        // - Cm/Eb taking 1 beat (from the / slash)
        // - Eb taking 3 beats (from the /// slashes)
        let input = r#"
Slash Rhythm Test
120bpm 4/4 #C

CH
Cm/Eb / Eb ///
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        let section = &chart.sections[0];
        let measures = section.measures();
        let time_sig = chart.time_signature.unwrap();

        // Should have 1 measure with 2 chords
        assert_eq!(
            measures.len(),
            1,
            "Expected 1 measure, got {}. Chords: {:?}",
            measures.len(),
            measures
                .iter()
                .map(|m| m.chords.iter().map(|c| &c.full_symbol).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );

        let measure = &measures[0];
        assert_eq!(
            measure.chords.len(),
            2,
            "Expected 2 chords in measure, got {}",
            measure.chords.len()
        );

        // Cm/Eb should be 1 beat
        let cm_eb_beats = measure.chords[0].duration.to_beats(time_sig);
        assert!(
            (cm_eb_beats - 1.0).abs() < 0.001,
            "Cm/Eb should be 1 beat, got {} (rhythm: {:?})",
            cm_eb_beats,
            measure.chords[0].rhythm
        );

        // Eb should be 3 beats
        let eb_beats = measure.chords[1].duration.to_beats(time_sig);
        assert!(
            (eb_beats - 3.0).abs() < 0.001,
            "Eb should be 3 beats, got {} (rhythm: {:?})",
            eb_beats,
            measure.chords[1].rhythm
        );
    }

    #[test]
    fn test_slash_rhythm_cross_barline() {
        // "Abmaj9 //// | //" should make Abmaj9 span 1.5 bars visually.
        // The slashes after | create SPACES (continuation) in the second measure.
        // - Measure 1: Abmaj9 with 4 beats (full measure)
        // - Measure 2: 2 spaces (continuation slashes) + Cm (2 beats) = 4 beats
        let input = r#"
Cross Barline Test
120bpm 4/4 #C

CH
Abmaj9 //// | // Cm
"#;

        let chart = parse_chart(input).expect("Failed to parse chart");

        let section = &chart.sections[0];
        let measures = section.measures();
        let time_sig = chart.time_signature.unwrap();

        // Should have 2 measures
        assert_eq!(measures.len(), 2, "Expected 2 measures");

        // First measure should have Abmaj9 with 4 beats (full measure)
        let abmaj9 = &measures[0].chords[0];
        let abmaj9_beats = abmaj9.duration.to_beats(time_sig);
        assert!(
            (abmaj9_beats - 4.0).abs() < 0.001,
            "Abmaj9 should be 4 beats (1 bar), got {} (rhythm: {:?})",
            abmaj9_beats,
            abmaj9.rhythm
        );

        // Second measure should have:
        // - 2 spaces (continuation from //)
        // - 1 chord (Cm)
        assert_eq!(
            measures[1].rhythm_elements.len(),
            3,
            "Second measure should have 3 rhythm elements (2 spaces + 1 chord)"
        );
        assert_eq!(
            measures[1].chords.len(),
            1,
            "Second measure should have 1 chord (Cm)"
        );
        assert_eq!(measures[1].chords[0].full_symbol, "Cm");

        // The continuation spaces should total 2 beats
        use crate::chart::types::RhythmElement;
        let space_count = measures[1]
            .rhythm_elements
            .iter()
            .filter(|e| matches!(e, RhythmElement::Space(_)))
            .count();
        assert_eq!(space_count, 2, "Should have 2 continuation spaces");
    }

    #[test]
    fn test_defaults_applied_for_titleless_snippet() {
        // Minimal snippet - just chords, no metadata at all
        let input = r#"
VS
C G Am F
"#;

        let chart = parse_chart(input).expect("Should parse successfully");

        // Default time signature should be 4/4
        assert_eq!(
            chart.time_signature.unwrap().numerator,
            4,
            "Default time signature numerator should be 4"
        );
        assert_eq!(
            chart.time_signature.unwrap().denominator,
            4,
            "Default time signature denominator should be 4"
        );
        assert_eq!(
            chart.initial_time_signature.unwrap().numerator,
            4,
            "Initial time signature should also be 4/4"
        );

        // Default key should be C major
        let key = chart.initial_key.as_ref().expect("Should have initial key");
        assert_eq!(key.root().name(), "C", "Default key root should be C");

        let current_key = chart.current_key.as_ref().expect("Should have current key");
        assert_eq!(
            current_key.root().name(),
            "C",
            "Current key root should be C"
        );

        // No title should be set
        assert!(
            chart.metadata.title.is_none(),
            "Title should be None for titleless snippet"
        );
    }

    #[test]
    fn test_explicit_key_overrides_default() {
        // Snippet with only key specified, no time signature
        let input = r#"
#G

VS
G D Em C
"#;

        let chart = parse_chart(input).expect("Should parse successfully");

        // Time signature should still be 4/4 (default)
        assert_eq!(chart.time_signature.unwrap().numerator, 4);
        assert_eq!(chart.time_signature.unwrap().denominator, 4);

        // Key should be G major (specified)
        let key = chart.initial_key.as_ref().expect("Should have initial key");
        assert_eq!(key.root().name(), "G", "Key should be G major as specified");
    }

    #[test]
    fn test_explicit_time_sig_overrides_default() {
        // Snippet with only time signature specified
        let input = r#"
6/8

VS
G D Em
"#;

        let chart = parse_chart(input).expect("Should parse successfully");

        // Time signature should be 6/8 (specified)
        assert_eq!(chart.time_signature.unwrap().numerator, 6);
        assert_eq!(chart.time_signature.unwrap().denominator, 8);

        // Key should be C major (default)
        let key = chart.initial_key.as_ref().expect("Should have initial key");
        assert_eq!(key.root().name(), "C", "Key should default to C major");
    }
}

// endregion: --- Tests
