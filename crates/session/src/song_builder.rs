//! SongBuilder - Extract song structure from DAW projects
//!
//! Analyzes markers, regions, and tempo maps to build Song domain objects.
//!
//! ## Song Name Convention
//! Project names follow the format: "Title - Artist.rpp"
//! - The ".rpp" extension is stripped
//! - "Title - Artist" is parsed to extract song name and artist
//!
//! ## Section Detection
//! Sections can be built from either:
//! 1. **Regions**: Regions contained within a song region (preferred)
//! 2. **Markers**: When no regions exist, consecutive markers are used to define sections
//!    Each marker defines the start of a section, ending at the next marker.

use daw_control::Project;
use daw_proto::{Marker, Region};
use session_proto::{Comment, Section, SectionType, Song};
use tracing::{Level, debug, warn};
use uuid::Uuid;

/// Builder for extracting Song structure from DAW projects
pub struct SongBuilder;

/// Helper to get seconds from Position
fn position_to_seconds(pos: &daw_proto::Position) -> f64 {
    pos.time.as_ref().map(|t| t.as_seconds()).unwrap_or(0.0)
}

impl SongBuilder {
    /// Build one or more Songs from a DAW project.
    ///
    /// When the project contains multiple parent regions (each with ≥2 child regions),
    /// each parent region is treated as a separate song (multi-song mode).
    /// Otherwise the entire project is treated as a single song (backward-compatible).
    pub async fn build(project: &Project) -> eyre::Result<Vec<Song>> {
        debug!("SongBuilder::build for project {}", project.guid());

        let markers_api = project.markers();
        let regions_api = project.regions();
        let (project_info, markers, regions) =
            tokio::try_join!(project.info(), markers_api.all(), regions_api.all())?;
        let tempo_map = project.tempo_map();

        // Check for multi-song layout: multiple parent regions each containing ≥2 children
        let song_regions = Self::find_song_regions(&regions);
        if song_regions.len() >= 2 {
            debug!(
                "Multi-song mode: found {} song regions in project {}",
                song_regions.len(),
                project.guid()
            );
            let project_guid = project.guid().to_string();
            let mut songs = Vec::with_capacity(song_regions.len());
            for song_region in &song_regions {
                match Self::build_song_from_region(
                    &project_guid,
                    song_region,
                    &regions,
                    &markers,
                    &tempo_map,
                )
                .await
                {
                    Ok(song) => songs.push(song),
                    Err(e) => {
                        warn!(
                            "Failed to build song from region '{}': {}",
                            song_region.name, e
                        );
                    }
                }
            }
            Ok(songs)
        } else {
            // Single-song mode: preserve exact backward compatibility
            let song =
                Self::build_single_song(project, &project_info.name, &markers, &regions, &tempo_map)
                    .await?;
            Ok(vec![song])
        }
    }

    /// Build a single Song from the entire project (original build logic).
    ///
    /// Preserves exact backward compatibility for projects with zero or one song region.
    async fn build_single_song(
        project: &Project,
        project_name: &str,
        markers: &[Marker],
        regions: &[Region],
        tempo_map: &daw_control::TempoMap,
    ) -> eyre::Result<Song> {
        // Parse song name and artist from project name
        let (song_name, _artist) = Self::parse_project_name(project_name);

        // Find special markers
        let count_in_marker = markers.iter().find(|m| Self::is_count_in_marker(&m.name));
        let absolute_start_marker = markers.iter().find(|m| m.name == "=START");
        let songstart_marker = markers.iter().find(|m| Self::is_songstart_marker(&m.name));
        let songend_marker = markers.iter().find(|m| Self::is_songend_marker(&m.name));
        let absolute_end_marker = markers.iter().find(|m| m.name == "=END");

        if tracing::enabled!(Level::DEBUG) {
            // Debug: log found markers with positions
            debug!(
                "All markers: {:?}",
                markers
                    .iter()
                    .map(|m| (&m.name, position_to_seconds(&m.position)))
                    .collect::<Vec<_>>()
            );
            debug!(
                "count_in_marker: {:?}",
                count_in_marker.map(|m| (&m.name, position_to_seconds(&m.position)))
            );
            debug!(
                "absolute_start_marker (=START): {:?}",
                absolute_start_marker.map(|m| (&m.name, position_to_seconds(&m.position)))
            );
            debug!(
                "songstart_marker: {:?}",
                songstart_marker.map(|m| (&m.name, position_to_seconds(&m.position)))
            );
            debug!(
                "songend_marker: {:?}",
                songend_marker.map(|m| (&m.name, position_to_seconds(&m.position)))
            );
            debug!(
                "absolute_end_marker (=END): {:?}",
                absolute_end_marker.map(|m| (&m.name, position_to_seconds(&m.position)))
            );
        }

        // Legacy marker support
        // Note: =START is the absolute start (including count-in), not SONGSTART
        // Only use =SONGSTART as a fallback, not =START
        let start_marker =
            songstart_marker.or_else(|| markers.iter().find(|m| m.name.starts_with("=SONGSTART")));

        // Note: =END is the absolute end (including outro), not SONGEND
        // Only use =SONGEND as a fallback, not =END
        let end_marker =
            songend_marker.or_else(|| markers.iter().find(|m| m.name.starts_with("=SONGEND")));

        // Find the song region (if regions exist)
        let song_region = Self::find_song_region(regions);

        // Determine song bounds
        let (start_seconds, songend_seconds, end_seconds, count_in_seconds) =
            if let (Some(start), Some(end)) = (start_marker, end_marker) {
                let song_start = position_to_seconds(&start.position);
                let song_end = position_to_seconds(&end.position);
                let absolute_end = absolute_end_marker
                    .map(|m| position_to_seconds(&m.position))
                    .unwrap_or(song_end);

                // Use COUNT-IN marker first, fall back to =START
                // COUNT-IN is more explicit about the count-in position
                let count_in = if let Some(ci_marker) = count_in_marker.or(absolute_start_marker) {
                    let ci_time = position_to_seconds(&ci_marker.position);
                    debug!(
                        "Count-in calculation: ci_marker={}, ci_time={}, song_start={}",
                        ci_marker.name, ci_time, song_start
                    );
                    if ci_time < song_start {
                        let duration = song_start - ci_time;
                        debug!("Count-in duration: {}", duration);
                        Some(duration)
                    } else {
                        debug!("Count-in marker is NOT before song_start, no count-in");
                        None
                    }
                } else {
                    debug!("No count-in or =START marker found");
                    None
                };

                debug!(
                    "Song bounds: start={}, songend={}, end={}, count_in={:?}",
                    song_start, song_end, absolute_end, count_in
                );
                (song_start, song_end, absolute_end, count_in)
            } else if let Some(ref song_region) = song_region {
                let end = song_region.time_range.end_seconds();
                let start = song_region.time_range.start_seconds();
                let count_in = count_in_marker.and_then(|m| {
                    let marker_time = position_to_seconds(&m.position);
                    if marker_time < start {
                        Some(start - marker_time)
                    } else {
                        None
                    }
                });
                (start, end, end, count_in)
            } else {
                // Fallback to entire project
                let start = markers
                    .iter()
                    .map(|m| position_to_seconds(&m.position))
                    .chain(regions.iter().map(|r| r.time_range.start_seconds()))
                    .min_by(|a: &f64, b: &f64| a.partial_cmp(b).unwrap())
                    .unwrap_or(0.0);

                let end = markers
                    .iter()
                    .map(|m| position_to_seconds(&m.position))
                    .chain(regions.iter().map(|r| r.time_range.end_seconds()))
                    .max_by(|a: &f64, b: &f64| a.partial_cmp(b).unwrap())
                    .unwrap_or(60.0);

                (start, end, end, None)
            };

        // Extract sections - prefer regions, fall back to markers
        let mut sections = if let Some(ref song_region) = song_region {
            Self::extract_sections_from_song_region(regions, song_region)?
        } else if !regions.is_empty() {
            Self::extract_sections_from_regions(regions, start_seconds, songend_seconds)?
        } else {
            // No regions - build sections from markers
            debug!("No regions found, building sections from markers");
            Self::build_sections_from_markers(markers, start_seconds, songend_seconds)?
        };

        debug!("Extracted {} sections", sections.len());
        if tracing::enabled!(Level::DEBUG) {
            for (i, section) in sections.iter().enumerate() {
                debug!(
                    "  Section[{}]: '{}' type={:?} start={:.3} end={:.3} duration={:.3}",
                    i,
                    section.name,
                    section.section_type,
                    section.start_seconds,
                    section.end_seconds,
                    section.end_seconds - section.start_seconds
                );
            }
        }

        // Add Count-In section at the beginning if there's a count-in
        // IMPORTANT: Use the first section's start time as Count-In's end time to ensure
        // continuity (no gaps between Count-In and the first section). This handles cases
        // where markers and regions don't perfectly align due to tempo quantization.
        let song_start_seconds = if let Some(count_in_duration) = count_in_seconds {
            if count_in_duration > 0.0 {
                let count_in_start = start_seconds - count_in_duration;
                // Use first section's start time as count-in end (ensures no gap)
                let count_in_end = sections
                    .first()
                    .map(|s| s.start_seconds)
                    .unwrap_or(start_seconds);
                debug!(
                    "Adding Count-In section: start={:.3} end={:.3} (first_section_start={:.3}, marker_start={:.3})",
                    count_in_start, count_in_end, count_in_end, start_seconds
                );
                sections.insert(
                    0,
                    Section {
                        id: None,
                        name: "Count-In".to_string(),
                        comment: None,
                        section_type: SectionType::CountIn,
                        start_seconds: count_in_start,
                        end_seconds: count_in_end,
                        number: None,
                        color: None,
                    },
                );
                count_in_start
            } else {
                start_seconds
            }
        } else {
            start_seconds
        };

        // Add END section if there's a gap between SONGEND and =END
        // IMPORTANT: Use the last section's end time as END's start time to ensure
        // continuity (no gaps between the last content section and END). This handles
        // cases where markers and regions don't perfectly align due to tempo quantization.
        if end_seconds > songend_seconds + 0.01 {
            // Use last section's end time as END start (ensures no gap)
            let end_section_start = sections
                .last()
                .map(|s| s.end_seconds)
                .unwrap_or(songend_seconds);
            debug!(
                "Adding END section: start={:.3} end={:.3} (last_section_end={:.3}, marker_songend={:.3})",
                end_section_start, end_seconds, end_section_start, songend_seconds
            );
            sections.push(Section {
                id: None,
                name: "End".to_string(),
                comment: None,
                section_type: SectionType::End,
                start_seconds: end_section_start,
                end_seconds,
                number: None,
                color: None,
            });
        }

        // Log final sections list
        debug!("Final sections after adding Count-In/End:");
        if tracing::enabled!(Level::DEBUG) {
            for (i, section) in sections.iter().enumerate() {
                debug!(
                    "  Final[{}]: '{}' start={:.3} end={:.3}",
                    i, section.name, section.start_seconds, section.end_seconds
                );
            }
        }

        // Get tempo and time signature at song start
        let tempo = tempo_map.tempo_at(start_seconds).await.ok();
        let time_sig = tempo_map
            .time_signature_at(start_seconds)
            .await
            .ok()
            .map(|(num, denom)| daw_proto::TimeSignature::new(num as u32, denom as u32));

        // Build measure positions if we have tempo and time signature
        let measure_positions = if let (Some(bpm), Some(ts)) = (tempo, time_sig) {
            Self::calculate_measure_positions(song_start_seconds, end_seconds, bpm, ts)
        } else {
            Vec::new()
        };

        // Extract comment markers (non-structural markers within song bounds)
        // Also handle COUNT-IN marker as a comment if it's after the first section starts
        // (for songs where count-in happens mid-song, e.g., keys-only intro)
        let first_section_start = sections
            .first()
            .map(|s| s.start_seconds)
            .unwrap_or(song_start_seconds);
        let count_in_marker_pos = count_in_marker.map(|m| position_to_seconds(&m.position));
        let count_in_is_mid_song = count_in_marker_pos
            .map(|pos| pos > first_section_start + 0.01)
            .unwrap_or(false);

        let mut comments: Vec<Comment> = markers
            .iter()
            .filter(|m| {
                let pos = position_to_seconds(&m.position);
                // Must be within song bounds
                let in_bounds = pos >= song_start_seconds && pos <= end_seconds;
                // Must not be a structural marker (unless it's a mid-song count-in)
                let is_structural = Self::is_structural_marker(&m.name);
                // Include if: in bounds AND (not structural OR is mid-song count-in)
                let is_mid_song_count_in =
                    Self::is_count_in_marker(&m.name) && count_in_is_mid_song;
                in_bounds && (!is_structural || is_mid_song_count_in)
            })
            .map(|m| {
                let is_count_in = Self::is_count_in_marker(&m.name);
                // Check for section-only prefix (>)
                let (text, section_only) = if m.name.trim().starts_with('>') {
                    (
                        m.name
                            .trim()
                            .strip_prefix('>')
                            .unwrap_or(&m.name)
                            .trim()
                            .to_string(),
                        true,
                    )
                } else {
                    (m.name.clone(), false)
                };
                Comment {
                    id: m.id,
                    text,
                    position_seconds: position_to_seconds(&m.position),
                    color: m.color,
                    is_count_in,
                    section_only,
                }
            })
            .collect();

        // Sort comments by position
        comments.sort_by(|a, b| {
            a.position_seconds
                .partial_cmp(&b.position_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if tracing::enabled!(Level::DEBUG) {
            debug!("Extracted {} comments", comments.len());
            for comment in &comments {
                debug!(
                    "  Comment: '{}' at {:.3}s{}",
                    comment.text,
                    comment.position_seconds,
                    if comment.is_count_in {
                        " (count-in)"
                    } else {
                        ""
                    }
                );
            }
        }

        Ok(Song {
            id: Uuid::new_v4().to_string(),
            name: song_name,
            project_guid: project.guid().to_string(),
            start_seconds: song_start_seconds,
            end_seconds,
            count_in_seconds,
            sections,
            comments,
            tempo,
            time_signature: time_sig,
            measure_positions,
            chart_text: None,
            parsed_chart: None,
            detected_chords: Vec::new(),
            chart_fingerprint: None,
            advance_mode: None,
        })
    }

    /// Build a Song from a specific parent region within a multi-song project.
    ///
    /// In multi-song mode, each parent region defines a song. Song name is parsed
    /// from the region name using the "Title - Artist" convention. SONGSTART/SONGEND
    /// markers are ignored; bounds come from the region's time range.
    async fn build_song_from_region(
        project_guid: &str,
        song_region: &Region,
        all_regions: &[Region],
        all_markers: &[Marker],
        tempo_map: &daw_control::TempoMap,
    ) -> eyre::Result<Song> {
        let (song_name, _artist) = Self::parse_project_name(&song_region.name);
        let start_seconds = song_region.time_range.start_seconds();
        let end_seconds = song_region.time_range.end_seconds();

        debug!(
            "build_song_from_region: '{}' start={:.3} end={:.3}",
            song_name, start_seconds, end_seconds
        );

        // Extract sections from child regions within this song region
        let mut sections = Self::extract_sections_from_song_region(all_regions, song_region)?;

        // Check for COUNT-IN marker before this song region
        let count_in_marker = all_markers.iter().find(|m| {
            Self::is_count_in_marker(&m.name) && {
                let pos = position_to_seconds(&m.position);
                pos < start_seconds && pos >= start_seconds - 30.0 // reasonable max count-in
            }
        });

        let count_in_seconds = count_in_marker.and_then(|m| {
            let marker_time = position_to_seconds(&m.position);
            if marker_time < start_seconds {
                Some(start_seconds - marker_time)
            } else {
                None
            }
        });

        // Add Count-In section if present
        let song_start_seconds = if let Some(count_in_duration) = count_in_seconds {
            if count_in_duration > 0.0 {
                let count_in_start = start_seconds - count_in_duration;
                let count_in_end = sections
                    .first()
                    .map(|s| s.start_seconds)
                    .unwrap_or(start_seconds);
                sections.insert(
                    0,
                    Section {
                        id: None,
                        name: "Count-In".to_string(),
                        comment: None,
                        section_type: SectionType::CountIn,
                        start_seconds: count_in_start,
                        end_seconds: count_in_end,
                        number: None,
                        color: None,
                    },
                );
                count_in_start
            } else {
                start_seconds
            }
        } else {
            start_seconds
        };

        // Get tempo and time signature at song start
        let tempo = tempo_map.tempo_at(start_seconds).await.ok();
        let time_sig = tempo_map
            .time_signature_at(start_seconds)
            .await
            .ok()
            .map(|(num, denom)| daw_proto::TimeSignature::new(num as u32, denom as u32));

        let measure_positions = if let (Some(bpm), Some(ts)) = (tempo, time_sig) {
            Self::calculate_measure_positions(song_start_seconds, end_seconds, bpm, ts)
        } else {
            Vec::new()
        };

        // Extract comment markers within this song region's bounds
        let comments = Self::extract_comments_in_range(
            all_markers,
            song_start_seconds,
            end_seconds,
        );

        Ok(Song {
            id: Uuid::new_v4().to_string(),
            name: song_name,
            project_guid: project_guid.to_string(),
            start_seconds: song_start_seconds,
            end_seconds,
            count_in_seconds,
            sections,
            comments,
            tempo,
            time_signature: time_sig,
            measure_positions,
            chart_text: None,
            parsed_chart: None,
            detected_chords: Vec::new(),
            chart_fingerprint: None,
            advance_mode: None,
        })
    }

    /// Extract non-structural comment markers within a time range.
    fn extract_comments_in_range(
        markers: &[Marker],
        start: f64,
        end: f64,
    ) -> Vec<Comment> {
        let mut comments: Vec<Comment> = markers
            .iter()
            .filter(|m| {
                let pos = position_to_seconds(&m.position);
                let in_bounds = pos >= start && pos <= end;
                let is_structural = Self::is_structural_marker(&m.name);
                in_bounds && !is_structural
            })
            .map(|m| {
                let is_count_in = Self::is_count_in_marker(&m.name);
                let (text, section_only) = if m.name.trim().starts_with('>') {
                    (
                        m.name
                            .trim()
                            .strip_prefix('>')
                            .unwrap_or(&m.name)
                            .trim()
                            .to_string(),
                        true,
                    )
                } else {
                    (m.name.clone(), false)
                };
                Comment {
                    id: m.id,
                    text,
                    position_seconds: position_to_seconds(&m.position),
                    color: m.color,
                    is_count_in,
                    section_only,
                }
            })
            .collect();

        comments.sort_by(|a, b| {
            a.position_seconds
                .partial_cmp(&b.position_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        comments
    }

    /// Parse project name to extract song title and artist
    ///
    /// Format: "Title - Artist.rpp" or "Title - Artist"
    /// Returns: (song_name, Option<artist>)
    fn parse_project_name(name: &str) -> (String, Option<String>) {
        // Remove .rpp extension (case insensitive)
        let name = name
            .trim()
            .trim_end_matches(".rpp")
            .trim_end_matches(".RPP")
            .trim_end_matches(".Rpp");

        // Look for " - " separator (with spaces around dash)
        if let Some(sep_pos) = name.find(" - ") {
            let title = name[..sep_pos].trim();
            let artist = name[sep_pos + 3..].trim();

            if artist.is_empty() {
                (title.to_string(), None)
            } else {
                (title.to_string(), Some(artist.to_string()))
            }
        } else {
            (name.to_string(), None)
        }
    }

    /// Check if a marker name indicates a count-in marker
    /// Supports: COUNTIN, COUNT-IN, COUNT IN, count in, count-in, COUNT_IN, etc.
    fn is_count_in_marker(name: &str) -> bool {
        let normalized = name.to_uppercase().replace(['-', ' ', '_'], "");
        normalized == "COUNTIN"
    }

    /// Check if a marker name indicates a SONGSTART marker
    fn is_songstart_marker(name: &str) -> bool {
        let upper = name.to_uppercase();
        upper == "SONGSTART"
            || upper.starts_with("SONGSTART ")
            || upper == "SONG START"
            || upper.starts_with("SONG START ")
    }

    /// Check if a marker name indicates a SONGEND marker
    fn is_songend_marker(name: &str) -> bool {
        let upper = name.to_uppercase();
        upper == "SONGEND"
            || upper.starts_with("SONGEND ")
            || upper == "SONG END"
            || upper.starts_with("SONG END ")
    }

    /// Check if a marker is a special marker (not a section marker)
    fn is_special_marker(name: &str) -> bool {
        Self::is_count_in_marker(name)
            || Self::is_songstart_marker(name)
            || Self::is_songend_marker(name)
            || name == "=START"
            || name == "=END"
            || name.starts_with("=SONGSTART")
            || name.starts_with("=SONGEND")
    }

    /// Check if a marker is a structural marker (used for song bounds, not for comments)
    /// This is similar to is_special_marker but includes COUNT-IN since it can
    /// appear as a comment when it's mid-song
    fn is_structural_marker(name: &str) -> bool {
        Self::is_count_in_marker(name)
            || Self::is_songstart_marker(name)
            || Self::is_songend_marker(name)
            || name == "=START"
            || name == "=END"
            || name.starts_with("=SONGSTART")
            || name.starts_with("=SONGEND")
    }

    /// Find all regions that qualify as song regions (contain ≥2 child regions).
    ///
    /// Returns leaf-level parent regions only — a region that contains another
    /// song-candidate region is considered a "grandparent" (e.g., a setlist wrapper)
    /// and is filtered out. Results are sorted by start time.
    fn find_song_regions(regions: &[Region]) -> Vec<&Region> {
        // Step 1: Find all regions containing ≥2 children
        let mut candidates: Vec<&Region> = regions
            .iter()
            .filter(|region| {
                let contained_count = regions
                    .iter()
                    .filter(|r| {
                        r.id != region.id
                            && r.time_range.start_seconds() >= region.time_range.start_seconds()
                            && r.time_range.end_seconds() <= region.time_range.end_seconds()
                    })
                    .count();
                contained_count >= 2
            })
            .collect();

        // Step 2: Remove grandparents — any candidate that contains another candidate.
        // Collect IDs of grandparents first to avoid borrow conflict with retain.
        let grandparent_ids: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                candidates.iter().any(|other| {
                    other.id != candidate.id
                        && other.time_range.start_seconds()
                            >= candidate.time_range.start_seconds()
                        && other.time_range.end_seconds() <= candidate.time_range.end_seconds()
                        && (other.time_range.start_seconds()
                            > candidate.time_range.start_seconds()
                            || other.time_range.end_seconds()
                                < candidate.time_range.end_seconds())
                })
            })
            .map(|r| r.id)
            .collect();
        candidates.retain(|r| !grandparent_ids.contains(&r.id));

        // Step 3: Sort by start time
        candidates.sort_by(|a, b| {
            a.time_range
                .start_seconds()
                .partial_cmp(&b.time_range.start_seconds())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
    }

    /// Find the song region - the region that contains other regions (sections)
    fn find_song_region(regions: &[Region]) -> Option<&Region> {
        let mut best_region: Option<&Region> = None;
        let mut best_count = 0;

        for region in regions {
            let contained_count = regions
                .iter()
                .filter(|r| {
                    r.id != region.id
                        && r.time_range.start_seconds() >= region.time_range.start_seconds()
                        && r.time_range.end_seconds() <= region.time_range.end_seconds()
                })
                .count();

            if contained_count > best_count {
                best_count = contained_count;
                best_region = Some(region);
            }
        }

        if best_count > 0 { best_region } else { None }
    }

    /// Build sections from markers (when no regions exist)
    ///
    /// Each marker defines the start of a section, ending at the next marker.
    fn build_sections_from_markers(
        markers: &[Marker],
        song_start: f64,
        song_end: f64,
    ) -> eyre::Result<Vec<Section>> {
        // Filter to section markers within song bounds (excluding special markers)
        let mut section_markers: Vec<&Marker> = markers
            .iter()
            .filter(|m| {
                let pos = position_to_seconds(&m.position);
                pos >= song_start && pos < song_end && !Self::is_special_marker(&m.name)
            })
            .collect();

        // Sort by position
        section_markers.sort_by(|a, b| {
            position_to_seconds(&a.position)
                .partial_cmp(&position_to_seconds(&b.position))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut sections = Vec::new();

        for (idx, marker) in section_markers.iter().enumerate() {
            let start = position_to_seconds(&marker.position);

            // End is at the next marker or song end
            let end = if idx + 1 < section_markers.len() {
                position_to_seconds(&section_markers[idx + 1].position)
            } else {
                song_end
            };

            let (section_type, number, clean_name, comment) =
                Self::parse_section_name(&marker.name);

            sections.push(Section {
                id: marker.id,
                name: clean_name,
                comment,
                section_type,
                start_seconds: start,
                end_seconds: end,
                number,
                color: marker.color,
            });
        }

        Ok(sections)
    }

    /// Extract sections from regions contained within the song region
    fn extract_sections_from_song_region(
        regions: &[Region],
        song_region: &Region,
    ) -> eyre::Result<Vec<Section>> {
        let song_start = song_region.time_range.start_seconds();
        let song_end = song_region.time_range.end_seconds();

        let mut sections: Vec<Section> = regions
            .iter()
            .filter(|r| {
                r.id != song_region.id
                    && r.time_range.start_seconds() >= song_start
                    && r.time_range.end_seconds() <= song_end
            })
            .map(|r| {
                let (section_type, number, clean_name, comment) = Self::parse_section_name(&r.name);
                Section {
                    id: r.id,
                    name: clean_name,
                    comment,
                    section_type,
                    start_seconds: r.time_range.start_seconds(),
                    end_seconds: r.time_range.end_seconds(),
                    number,
                    color: r.color,
                }
            })
            .collect();

        sections.sort_by(|a, b| {
            a.start_seconds
                .partial_cmp(&b.start_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(sections)
    }

    /// Extract sections from regions within song bounds (fallback when no song region)
    fn extract_sections_from_regions(
        regions: &[Region],
        start: f64,
        end: f64,
    ) -> eyre::Result<Vec<Section>> {
        let mut sections: Vec<Section> = regions
            .iter()
            .filter(|r| r.time_range.start_seconds() >= start && r.time_range.end_seconds() <= end)
            .filter(|r| !r.name.starts_with("SONG:") && r.name.to_uppercase() != "SONG")
            .map(|r| {
                let (section_type, number, clean_name, comment) = Self::parse_section_name(&r.name);
                Section {
                    id: r.id,
                    name: clean_name,
                    comment,
                    section_type,
                    start_seconds: r.time_range.start_seconds(),
                    end_seconds: r.time_range.end_seconds(),
                    number,
                    color: r.color,
                }
            })
            .collect();

        sections.sort_by(|a, b| {
            a.start_seconds
                .partial_cmp(&b.start_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(sections)
    }

    /// Calculate measure positions for the song
    fn calculate_measure_positions(
        start_seconds: f64,
        end_seconds: f64,
        bpm: f64,
        ts: daw_proto::TimeSignature,
    ) -> Vec<daw_proto::Position> {
        let beats_per_measure = ts.numerator() as f64;
        let seconds_per_beat = 60.0 / bpm;
        let measure_duration = beats_per_measure * seconds_per_beat;

        let song_duration = end_seconds - start_seconds;
        let measure_count = (song_duration / measure_duration).ceil() as i32;

        (0..measure_count)
            .map(|idx| {
                let time_seconds = start_seconds + (idx as f64 * measure_duration);
                daw_proto::Position::from_time(daw_proto::TimePosition::from_seconds(time_seconds))
            })
            .collect()
    }

    /// Parse section type, number, name (without comment), and optional comment from region/marker name
    ///
    /// Supports formats like:
    /// - "Verse 1" -> (Verse, Some(1), "Verse 1", None)
    /// - "Interlude C" -> (Instrumental, Some(3), "Interlude C", None) // C=3rd variant
    /// - `Interlude C "Woodwinds"` -> (Instrumental, Some(3), "Interlude C", Some("Woodwinds"))
    /// - `Chorus 2 "Big Build"` -> (Chorus, Some(2), "Chorus 2", Some("Big Build"))
    fn parse_section_name(name: &str) -> (SectionType, Option<u32>, String, Option<String>) {
        // First, extract any quoted/bracketed comment
        let (name_without_comment, comment) = Self::extract_comment(name);

        let name_upper = name_without_comment.to_uppercase();
        let name_trimmed = name_upper.trim();

        let (type_part, number) = Self::extract_type_and_number(name_trimmed);
        // Pass the original case name part for Custom types
        let original_type_part = Self::extract_original_type_part(name_without_comment.trim());
        let section_type = Self::parse_section_type_with_original(type_part, &original_type_part);

        // Use the original name (without comment) preserving case
        let clean_name = name_without_comment.trim().to_string();

        (section_type, number, clean_name, comment)
    }

    /// Extract the type part from the original (non-uppercased) name
    /// This preserves case for Custom section types
    fn extract_original_type_part(name: &str) -> String {
        // Try "Type Number" format (e.g., "Verse 1", "CH 2")
        if let Some(last_space) = name.rfind(' ') {
            let potential_suffix = &name[last_space + 1..];
            // Check if it's a number or single letter variant
            if potential_suffix.parse::<u32>().is_ok()
                || (potential_suffix.len() == 1
                    && potential_suffix
                        .chars()
                        .next()
                        .unwrap()
                        .is_ascii_uppercase())
            {
                return name[..last_space].trim().to_string();
            }
        }

        // Try concatenated format (e.g., "V1", "CH2")
        let mut num_start = name.len();
        for (i, c) in name.chars().rev().enumerate() {
            let pos = name.len() - 1 - i;
            if c.is_ascii_digit() {
                num_start = pos;
            } else if num_start != name.len() {
                break;
            }
        }

        if num_start < name.len() {
            name[..num_start].trim().to_string()
        } else {
            name.to_string()
        }
    }

    /// Extract a comment/descriptor from a section name
    ///
    /// Supports multiple delimiter styles:
    /// - Double quotes: `Interlude C "Woodwinds"` -> ("Interlude C", "Woodwinds")
    /// - Curly braces: `Riff {Back In}` -> ("Riff", "Back In")
    /// - Parentheses: `Verse 1 (Acoustic)` -> ("Verse 1", "Acoustic")
    ///
    /// Returns (name_without_comment, optional_comment)
    fn extract_comment(name: &str) -> (&str, Option<String>) {
        let name = name.trim();

        // Try double quotes first: `Something "Comment"`
        if let Some(last_quote) = name.rfind('"') {
            if let Some(open_quote) = name[..last_quote].rfind('"') {
                let comment = name[open_quote + 1..last_quote].trim();
                let name_part = name[..open_quote].trim();

                if !comment.is_empty() {
                    return (name_part, Some(comment.to_string()));
                }
            }
        }

        // Try curly braces: `Something {Comment}`
        if let Some(close_brace) = name.rfind('}') {
            if let Some(open_brace) = name[..close_brace].rfind('{') {
                let comment = name[open_brace + 1..close_brace].trim();
                let name_part = name[..open_brace].trim();

                if !comment.is_empty() {
                    return (name_part, Some(comment.to_string()));
                }
            }
        }

        // Try parentheses: `Something (Comment)`
        // Only if it looks like a descriptor (not a number like "Verse (1)")
        if let Some(close_paren) = name.rfind(')') {
            if let Some(open_paren) = name[..close_paren].rfind('(') {
                let comment = name[open_paren + 1..close_paren].trim();
                let name_part = name[..open_paren].trim();

                // Only treat as comment if it's not just a number
                if !comment.is_empty() && !comment.chars().all(|c| c.is_ascii_digit()) {
                    return (name_part, Some(comment.to_string()));
                }
            }
        }

        (name, None)
    }

    /// Extract the type part and optional number from a section name
    fn extract_type_and_number(name: &str) -> (&str, Option<u32>) {
        // Try "Type Number" format (e.g., "Verse 1", "CH 2")
        if let Some(last_space) = name.rfind(' ') {
            let potential_num = &name[last_space + 1..];
            if let Ok(num) = potential_num.parse::<u32>() {
                return (&name[..last_space], Some(num));
            }
            // Try single letter variant (A=1, B=2, C=3, etc.) for cases like "Interlude C"
            if potential_num.len() == 1 {
                let c = potential_num.chars().next().unwrap();
                if c.is_ascii_uppercase() {
                    // A=1, B=2, C=3, etc.
                    let num = (c as u32) - ('A' as u32) + 1;
                    return (&name[..last_space], Some(num));
                }
            }
        }

        // Try concatenated format (e.g., "V1", "CH2", "VS1A")
        // Handle letter suffix after number (e.g., "1A" -> 1)
        let mut num_start = name.len();
        let mut num_end = name.len();

        for (i, c) in name.chars().rev().enumerate() {
            let pos = name.len() - 1 - i;
            if c.is_ascii_digit() {
                num_start = pos;
                if num_end == name.len() {
                    num_end = pos + 1;
                }
            } else if num_end != name.len() {
                // Found non-digit after finding digits, stop
                break;
            }
        }

        if num_start < num_end {
            let num_str = &name[num_start..num_end];
            if let Ok(num) = num_str.parse::<u32>() {
                return (&name[..num_start], Some(num));
            }
        }

        (name, None)
    }

    /// Parse section type from the type part of the name, preserving original case for Custom
    ///
    /// Uses keyflow-proto's SectionType parsing with fallback for session-specific patterns.
    fn parse_section_type_with_original(type_part: &str, original_type_part: &str) -> SectionType {
        let s = type_part.trim().to_lowercase();

        // Handle pre/post modifiers first
        if s.starts_with("pre-") || s.starts_with("pre ") {
            let rest = s
                .trim_start_matches("pre-")
                .trim_start_matches("pre ")
                .trim();
            // Try to parse the inner type
            if let Ok(inner) = SectionType::parse(rest) {
                return SectionType::Pre(Box::new(inner));
            }
            // Default pre-chorus for ambiguous cases
            if rest == "chorus" || rest == "ch" || rest == "c" || rest.is_empty() {
                return SectionType::Pre(Box::new(SectionType::Chorus));
            }
        }

        if s.starts_with("post-") || s.starts_with("post ") {
            let rest = s
                .trim_start_matches("post-")
                .trim_start_matches("post ")
                .trim();
            if let Ok(inner) = SectionType::parse(rest) {
                return SectionType::Post(Box::new(inner));
            }
            if rest == "chorus" || rest == "ch" || rest == "c" || rest.is_empty() {
                return SectionType::Post(Box::new(SectionType::Chorus));
            }
        }

        // Try keyflow's parser first (handles most cases including fuzzy matching)
        if let Ok(section_type) = SectionType::parse(&s) {
            return section_type;
        }

        // Handle session-specific variations not in keyflow
        match s.as_str() {
            // Pre-chorus shorthand
            "prechorus" | "pre-chorus" | "pre chorus" | "pc" => {
                SectionType::Pre(Box::new(SectionType::Chorus))
            }

            // Build is sometimes used as breakdown
            "build" => SectionType::Breakdown,

            // Unknown - use Custom with original case preserved
            _ => SectionType::Custom(original_type_part.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_project_name() {
        let (name, artist) = SongBuilder::parse_project_name("Cryin' - Mateus Asato.rpp");
        assert_eq!(name, "Cryin'");
        assert_eq!(artist, Some("Mateus Asato".to_string()));

        let (name, artist) = SongBuilder::parse_project_name("My Song.rpp");
        assert_eq!(name, "My Song");
        assert_eq!(artist, None);

        let (name, artist) = SongBuilder::parse_project_name("Another Song - The Artist");
        assert_eq!(name, "Another Song");
        assert_eq!(artist, Some("The Artist".to_string()));
    }

    #[test]
    fn test_parse_section_name() {
        // Basic section types with numbers
        let (section_type, number, name, comment) = SongBuilder::parse_section_name("Verse 1");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1));
        assert_eq!(name, "Verse 1");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("V2");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(2));
        assert_eq!(name, "V2");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("VS 1A");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1));
        assert_eq!(name, "VS 1A");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("VS 1B");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1));
        assert_eq!(name, "VS 1B");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("CH 1");
        assert_eq!(section_type, SectionType::Chorus);
        assert_eq!(number, Some(1));
        assert_eq!(name, "CH 1");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("INST");
        assert_eq!(section_type, SectionType::Instrumental);
        assert_eq!(number, None);
        assert_eq!(name, "INST");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("GTR SOLO");
        assert_eq!(section_type, SectionType::Custom("GTR SOLO".to_string()));
        assert_eq!(number, None);
        assert_eq!(name, "GTR SOLO");
        assert_eq!(comment, None);

        let (section_type, number, name, comment) = SongBuilder::parse_section_name("SYNTH SOLO");
        assert_eq!(section_type, SectionType::Custom("SYNTH SOLO".to_string()));
        assert_eq!(number, None);
        assert_eq!(name, "SYNTH SOLO");
        assert_eq!(comment, None);
    }

    #[test]
    fn test_parse_section_name_with_comment() {
        // Section with quoted comment - "C" is interpreted as variant 3 (A=1, B=2, C=3)
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name(r#"Interlude C "Woodwinds""#);
        assert_eq!(section_type, SectionType::Interlude);
        assert_eq!(number, Some(3)); // C = 3rd variant
        assert_eq!(name, "Interlude C");
        assert_eq!(comment, Some("Woodwinds".to_string()));

        // Chorus with comment
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name(r#"Chorus 2 "Big Build""#);
        assert_eq!(section_type, SectionType::Chorus);
        assert_eq!(number, Some(2));
        assert_eq!(name, "Chorus 2");
        assert_eq!(comment, Some("Big Build".to_string()));

        // Bridge with descriptive comment
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name(r#"Bridge "Key Change to Eb""#);
        assert_eq!(section_type, SectionType::Bridge);
        assert_eq!(number, None);
        assert_eq!(name, "Bridge");
        assert_eq!(comment, Some("Key Change to Eb".to_string()));

        // Verse with instrument indication
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name(r#"Verse 1 "Guitar Solo""#);
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1));
        assert_eq!(name, "Verse 1");
        assert_eq!(comment, Some("Guitar Solo".to_string()));
    }

    #[test]
    fn test_parse_section_name_with_curly_braces() {
        // Curly braces descriptor: Riff {Back In}
        // "Riff" is not a standard section type, so it becomes Custom
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name("Riff {Back In}");
        assert_eq!(section_type, SectionType::Custom("Riff".to_string()));
        assert_eq!(number, None);
        assert_eq!(name, "Riff");
        assert_eq!(comment, Some("Back In".to_string()));

        // Verse with curly braces
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name("Verse 1 {Acoustic}");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1));
        assert_eq!(name, "Verse 1");
        assert_eq!(comment, Some("Acoustic".to_string()));

        // Outro with descriptor
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name("Outro {Fade Out}");
        assert_eq!(section_type, SectionType::Outro);
        assert_eq!(number, None);
        assert_eq!(name, "Outro");
        assert_eq!(comment, Some("Fade Out".to_string()));
    }

    #[test]
    fn test_parse_section_name_with_parentheses() {
        // Parentheses descriptor
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name("Intro (Keys Only)");
        assert_eq!(section_type, SectionType::Intro);
        assert_eq!(number, None);
        assert_eq!(name, "Intro");
        assert_eq!(comment, Some("Keys Only".to_string()));

        // Verse with number in parentheses - number is still extracted from name
        // The (1) is not treated as a comment since it's numeric-only
        // But the number extraction logic still finds the 1
        let (section_type, number, name, comment) = SongBuilder::parse_section_name("Verse (1)");
        assert_eq!(section_type, SectionType::Verse);
        assert_eq!(number, Some(1)); // Number extracted from (1)
        assert_eq!(name, "Verse (1)"); // Name preserved (no comment extracted)
        assert_eq!(comment, None); // Numeric-only parentheses are not comments

        // Bridge with descriptive parentheses
        let (section_type, number, name, comment) =
            SongBuilder::parse_section_name("Bridge 2 (Half Time)");
        assert_eq!(section_type, SectionType::Bridge);
        assert_eq!(number, Some(2));
        assert_eq!(name, "Bridge 2");
        assert_eq!(comment, Some("Half Time".to_string()));
    }

    #[test]
    fn test_is_special_marker() {
        assert!(SongBuilder::is_special_marker("COUNT-IN"));
        assert!(SongBuilder::is_special_marker("SONGSTART"));
        assert!(SongBuilder::is_special_marker("SONGEND"));
        assert!(SongBuilder::is_special_marker("=START"));
        assert!(SongBuilder::is_special_marker("=END"));
        assert!(!SongBuilder::is_special_marker("Intro"));
        assert!(!SongBuilder::is_special_marker("VS 1A"));
        assert!(!SongBuilder::is_special_marker("CH 1"));
    }
}
