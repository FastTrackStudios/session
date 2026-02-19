//! Performance View Layout
//!
//! Complete performance view layout with sidebar navigation and main progress display.
//! This is a reusable layout component that apps can import and use directly.
//!
//! Based on the FastTrackStudio desktop app performance view.
//! Uses Tailwind CSS for styling.
//!
//! Components call `SetlistService` methods directly via `Session::get().setlist()`.
//! The consuming app must call `Session::init()` during startup.

use crate::components::*;
use crate::prelude::*;
use crate::signals::*;

/// Performance view layout
///
/// Complete performance view with:
/// - Large progress visualization with transport controls
///
/// This component subscribes to global signals and provides a complete
/// performance view interface that apps can use directly.
///
/// # Example
///
/// ```rust,no_run
/// use session_ui::PerformanceLayout;
/// use dioxus::prelude::*;
///
/// fn App() -> Element {
///     rsx! {
///         PerformanceLayout {}
///     }
/// }
/// ```
#[component]
pub fn PerformanceLayout() -> Element {
    // PERFORMANCE: This top-level component should NOT subscribe to any high-frequency signals.
    // Child components handle their own subscriptions to minimize re-renders.
    //
    // Architecture:
    // - PerformanceMainContent: Subscribes to transport signals for progress display
    // - TransportControlBar: Subscribes to PLAYBACK_STATE for play/pause button

    rsx! {
        div {
            class: "flex h-full w-full bg-background text-foreground",
            PerformanceMainContent {}
        }
    }
}

/// Static song structure for sidebar - memoized to avoid recalculation
/// This only contains data that changes when the setlist structure changes,
/// NOT transport/progress data which updates at 60Hz
#[derive(Clone, PartialEq)]
struct SidebarSongStructure {
    name: String,
    bright_color: String,
    muted_color: String,
    sections: Vec<SidebarSectionStructure>,
}

#[derive(Clone, PartialEq)]
struct SidebarSectionStructure {
    label: String,
    bright_color: String,
    muted_color: String,
    comment: Option<String>,
    start_seconds: f64,
    end_seconds: f64,
}

/// Performance sidebar component
///
/// Displays song list with expandable sections and progress bars.
/// Shows progress for ALL songs that have transport state (independent playback).
///
/// PERFORMANCE: This component only subscribes to ACTIVE_INDICES for selection state.
/// Transport updates (60Hz) are handled by individual SidebarSongItem components
/// to avoid re-rendering the entire sidebar.
#[component]
pub fn PerformanceSidebar() -> Element {
    // Read active indices - updates when song/section selection changes
    let indices = ACTIVE_INDICES.read();
    let active_song_index = indices.song_index;
    let active_section_index = indices.section_index;
    drop(indices);

    // Memoize the static song structure - only recalculates when setlist changes
    let song_structures = use_memo(move || {
        let setlist = SETLIST_STRUCTURE.read();
        setlist
            .songs
            .iter()
            .map(|song| SidebarSongStructure {
                name: song.name.clone(),
                bright_color: song.bright_color(),
                muted_color: song.muted_color(),
                sections: song
                    .sections
                    .iter()
                    .map(|section| SidebarSectionStructure {
                        label: section.display_name(),
                        bright_color: section.bright_color(),
                        muted_color: section.muted_color(),
                        comment: section.comment.clone(),
                        start_seconds: section.start_seconds,
                        end_seconds: section.end_seconds,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            class: "h-full w-full bg-sidebar overflow-y-auto",

            // Header
            div {
                class: "p-4",
                h2 {
                    class: "text-xl font-bold text-sidebar-foreground mb-4",
                    "Navigator"
                }
            }

            // Song list - each song item handles its own transport subscription
            div {
                class: "space-y-1 pr-4 pb-4",

                for (song_idx, song_structure) in song_structures.read().iter().enumerate() {
                    SidebarSongItemReactive {
                        key: "{song_idx}",
                        song_idx: song_idx,
                        song_structure: song_structure.clone(),
                        is_expanded: active_song_index == Some(song_idx),
                        current_section_index: if active_song_index == Some(song_idx) {
                            active_section_index
                        } else {
                            None
                        },
                    }
                }
            }
        }
    }
}

/// Reactive song item that subscribes to its own transport updates
///
/// PERFORMANCE: Only subscribes to transport if song is expanded or playing.
/// Collapsed/inactive songs use peek() to avoid 60Hz re-renders.
/// Section progress is only calculated for expanded songs.
#[component]
fn SidebarSongItemReactive(
    song_idx: usize,
    song_structure: SidebarSongStructure,
    is_expanded: bool,
    current_section_index: Option<usize>,
) -> Element {
    // First, peek to check if this song is playing (without subscribing)
    let is_song_playing = SONG_TRANSPORT
        .peek()
        .get(&song_idx)
        .map(|t| t.is_playing)
        .unwrap_or(false);

    // PERFORMANCE: Only subscribe to transport updates if we need to show progress
    // (song is expanded or actively playing). Otherwise, use static 0% progress.
    let needs_transport = is_expanded || is_song_playing;

    let (song_progress, position_seconds) = if needs_transport {
        // Subscribe to transport - this component will re-render on updates
        let transport = SONG_TRANSPORT.read().get(&song_idx).cloned();

        let pos = transport
            .as_ref()
            .and_then(|t| t.position.time.map(|time| time.as_seconds()))
            .unwrap_or(0.0);

        // Calculate song progress
        let song_start = song_structure
            .sections
            .first()
            .map(|s| s.start_seconds)
            .unwrap_or(0.0);
        let song_end = song_structure
            .sections
            .last()
            .map(|s| s.end_seconds)
            .unwrap_or(0.0);
        let song_duration = song_end - song_start;
        let progress = if song_duration > 0.0 && transport.is_some() {
            ((pos - song_start) / song_duration * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        (progress, pos)
    } else {
        // Inactive song - no transport subscription, static 0% progress
        (0.0, 0.0)
    };

    // PERFORMANCE: Only build section data if expanded (sections are hidden otherwise)
    let sections: Vec<SectionItem> = if is_expanded {
        song_structure
            .sections
            .iter()
            .map(|section| {
                let section_duration = section.end_seconds - section.start_seconds;
                let section_progress = if section_duration > 0.0
                    && position_seconds >= section.start_seconds
                    && position_seconds <= section.end_seconds
                {
                    ((position_seconds - section.start_seconds) / section_duration * 100.0)
                        .clamp(0.0, 100.0)
                } else if position_seconds > section.end_seconds {
                    100.0
                } else {
                    0.0
                };

                SectionItem {
                    label: section.label.clone(),
                    progress: section_progress,
                    bright_color: section.bright_color.clone(),
                    muted_color: section.muted_color.clone(),
                    comment: section.comment.clone(),
                }
            })
            .collect()
    } else {
        // Collapsed song - no sections needed (won't be rendered)
        Vec::new()
    };

    let song_data = SongItemData {
        label: song_structure.name.clone(),
        progress: song_progress,
        bright_color: song_structure.bright_color.clone(),
        muted_color: song_structure.muted_color.clone(),
        sections,
    };

    rsx! {
        SongItem {
            song_data: song_data,
            index: song_idx,
            is_expanded: is_expanded,
            is_playing: is_song_playing,
            current_section_index: current_section_index,
            on_song_click: Callback::new(move |_| {
                tracing::info!("on_song_click callback triggered for song_idx={}", song_idx);
                spawn(async move {
                    tracing::info!("Calling seek_to_song({})", song_idx);
                    match Session::get().setlist().seek_to_song(song_idx).await {
                        Ok(_) => tracing::info!("seek_to_song({}) completed successfully", song_idx),
                        Err(e) => tracing::error!("seek_to_song({}) failed: {}", song_idx, e),
                    }
                });
            }),
            on_section_click: Callback::new(move |section_idx| {
                spawn(async move {
                    let _ = Session::get().setlist().seek_to_section(song_idx, section_idx).await;
                });
            }),
        }
    }
}

/// Standalone transport control panel for the dock system.
///
/// Reads playback state and loop state from global signals, wires up
/// callbacks to `Session::get().setlist()` service methods.
#[component]
pub fn TransportPanel() -> Element {
    let indices = ACTIVE_INDICES.read();
    let is_looping = indices.looping;
    drop(indices);

    let playback_state = *PLAYBACK_STATE.read();
    let is_playing = matches!(
        playback_state,
        daw_proto::PlayState::Playing | daw_proto::PlayState::Recording
    );

    rsx! {
        div {
            class: "h-full w-full flex flex-col bg-background",
            TransportControlBar {
                is_playing: is_playing,
                is_looping: is_looping,
                on_play_pause: Callback::new(move |_| {
                    LATENCY_TRACKER.write().start_play_toggle();
                    spawn(async move {
                        let _ = Session::get().setlist().toggle_playback().await;
                    });
                }),
                on_loop_toggle: Callback::new(move |_| {
                    spawn(async move {
                        let _ = Session::get().setlist().toggle_song_loop().await;
                    });
                }),
                on_back: Callback::new(move |_| {
                    spawn(async move {
                        let _ = Session::get().setlist().previous_section().await;
                    });
                }),
                on_forward: Callback::new(move |_| {
                    spawn(async move {
                        let _ = Session::get().setlist().next_section().await;
                    });
                }),
            }
        }
    }
}

/// Main performance content area
///
/// Displays song title, both progress bars (song and section), transport info badges,
/// and transport controls at the bottom.
///
/// PERFORMANCE: This component subscribes to ACTIVE_INDICES and SONG_TRANSPORT.
/// Static song data (progress sections, markers) is memoized and only recalculates
/// when song selection changes.
#[component]
fn PerformanceMainContent() -> Element {
    // Read indices - we need song_index and section_index
    let indices = ACTIVE_INDICES.read();
    let song_index = indices.song_index;
    let _section_index = indices.section_index;
    let queued_target = indices.queued_target.clone();
    drop(indices);

    // Get transport state for current song
    let transport_state = song_index.and_then(|idx| SONG_TRANSPORT.read().get(&idx).cloned());

    // Memoize current song - reads ACTIVE_INDICES inside so it re-fires on song change
    let current_song = use_memo(move || {
        let indices = ACTIVE_INDICES.read();
        let idx = indices.song_index;
        let setlist = SETLIST_STRUCTURE.read();
        idx.and_then(|i| setlist.songs.get(i).cloned())
    });

    // Memoize current section - reads ACTIVE_INDICES inside so it re-fires on section change
    let current_section = use_memo(move || {
        let indices = ACTIVE_INDICES.read();
        let sec_idx = indices.section_index;
        current_song
            .read()
            .as_ref()
            .and_then(|s| sec_idx.and_then(|idx| s.sections.get(idx).cloned()))
    });

    // Memoize progress sections - reads ACTIVE_INDICES to ensure reactivity on song change
    let progress_sections = use_memo(move || {
        let _indices = ACTIVE_INDICES.read();
        current_song
            .read()
            .as_ref()
            .map(|song| {
                let sections_start = song
                    .sections
                    .first()
                    .map(|s| s.start_seconds)
                    .unwrap_or(song.start_seconds);
                let sections_end = song
                    .sections
                    .last()
                    .map(|s| s.end_seconds)
                    .unwrap_or(song.end_seconds);
                let sections_duration = sections_end - sections_start;

                if sections_duration <= 0.0 {
                    return Vec::new();
                }

                song.sections
                    .iter()
                    .map(|section| {
                        let start_percent =
                            ((section.start_seconds - sections_start) / sections_duration) * 100.0;
                        let end_percent =
                            ((section.end_seconds - sections_start) / sections_duration) * 100.0;

                        ProgressSection {
                            name: section.display_name(),
                            short_name: section.short_display(),
                            start_percent: start_percent.max(0.0),
                            end_percent: end_percent.min(100.0),
                            color: section.bright_color(),
                            comment: section.comment.clone(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    // Calculate progress values - these depend on transport_state which updates at 60Hz
    let song_progress = {
        let song = current_song.read();
        song.as_ref()
            .and_then(|song| {
                transport_state.as_ref().map(|t| {
                    let position_seconds =
                        t.position.time.map(|time| time.as_seconds()).unwrap_or(0.0);
                    let sections_start = song
                        .sections
                        .first()
                        .map(|s| s.start_seconds)
                        .unwrap_or(song.start_seconds);
                    let sections_end = song
                        .sections
                        .last()
                        .map(|s| s.end_seconds)
                        .unwrap_or(song.end_seconds);
                    let sections_duration = sections_end - sections_start;

                    if sections_duration <= 0.0 {
                        return 0.0;
                    }

                    let relative_pos = position_seconds - sections_start;
                    (relative_pos / sections_duration) * 100.0
                })
            })
            .unwrap_or(0.0)
            .clamp(0.0, 100.0)
    };

    let section_progress = {
        let section = current_section.read();
        section
            .as_ref()
            .and_then(|section| {
                transport_state.as_ref().map(|t| {
                    let position_seconds =
                        t.position.time.map(|time| time.as_seconds()).unwrap_or(0.0);
                    let section_duration = section.end_seconds - section.start_seconds;
                    if section_duration > 0.0
                        && position_seconds >= section.start_seconds
                        && position_seconds <= section.end_seconds
                    {
                        ((position_seconds - section.start_seconds) / section_duration) * 100.0
                    } else if position_seconds > section.end_seconds {
                        100.0
                    } else {
                        0.0
                    }
                })
            })
            .unwrap_or(0.0)
    };

    // Get next song info - use peek() since we don't need reactivity for this
    let next_song_info = {
        let setlist = SETLIST_STRUCTURE.peek();
        song_index.and_then(|idx| {
            setlist
                .songs
                .get(idx + 1)
                .map(|song| (song.name.clone(), song.bright_color()))
        })
    };

    // Song key for animation detection (changes when song changes)
    let song_key = song_index.map(|i| i.to_string());

    // Memoize tempo markers - reads ACTIVE_INDICES to ensure reactivity on song change
    let tempo_markers = use_memo(move || {
        let _indices = ACTIVE_INDICES.read();
        current_song
            .read()
            .as_ref()
            .map(|song| {
                let mut markers = Vec::new();

                if let Some(tempo) = song.tempo {
                    markers.push(TempoMarkerData {
                        position_percent: 0.0,
                        label: format!("{:.0} bpm", tempo),
                        is_tempo: true,
                        is_time_sig: false,
                        show_line_only: false,
                    });
                }

                if let Some(ref ts) = song.time_signature {
                    markers.push(TempoMarkerData {
                        position_percent: 0.0,
                        label: format!("{}/{}", ts.numerator, ts.denominator),
                        is_tempo: false,
                        is_time_sig: true,
                        show_line_only: false,
                    });
                }

                markers
            })
            .unwrap_or_default()
    });

    // Memoize comment markers - reads ACTIVE_INDICES to ensure reactivity on song change
    let comment_markers = use_memo(move || {
        let _indices = ACTIVE_INDICES.read();
        current_song
            .read()
            .as_ref()
            .map(|song| {
                let sections_start = song
                    .sections
                    .first()
                    .map(|s| s.start_seconds)
                    .unwrap_or(song.start_seconds);
                let sections_end = song
                    .sections
                    .last()
                    .map(|s| s.end_seconds)
                    .unwrap_or(song.end_seconds);
                let sections_duration = sections_end - sections_start;

                if sections_duration <= 0.0 {
                    return Vec::new();
                }

                song.comments
                    .iter()
                    .filter_map(|comment| {
                        if comment.position_seconds >= sections_start
                            && comment.position_seconds <= sections_end
                        {
                            let position_percent = ((comment.position_seconds - sections_start)
                                / sections_duration)
                                * 100.0;
                            Some(CommentMarker {
                                position_percent: position_percent.clamp(0.0, 100.0),
                                text: comment.text.clone(),
                                color: comment.color_hex(),
                                is_count_in: comment.is_count_in,
                                section_only: comment.section_only,
                                position_seconds: comment.position_seconds,
                            })
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    });

    // PERFORMANCE: Extract tempo/time sig values and memoize measure indicators
    // These only change when tempo/time signature actually changes, not every frame
    let current_bpm = transport_state.as_ref().map(|t| t.bpm).unwrap_or(120.0);
    let current_time_sig_num = transport_state
        .as_ref()
        .map(|t| t.time_sig_num)
        .unwrap_or(4);
    let current_time_sig_denom = transport_state
        .as_ref()
        .map(|t| t.time_sig_denom)
        .unwrap_or(4);

    // Memoize measure indicators - reads ACTIVE_INDICES to ensure reactivity on song/section change
    let measure_indicators = use_memo(move || {
        let _indices = ACTIVE_INDICES.read();
        let section = current_section.read();
        let song = current_song.read();
        section
            .as_ref()
            .map(|section| {
                let section_duration = section.end_seconds - section.start_seconds;
                let section_start = section.start_seconds;
                if section_duration <= 0.0 {
                    return Vec::new();
                }

                let bpm = current_bpm;
                let time_sig_num = current_time_sig_num;
                let time_sig_denom = current_time_sig_denom;

                let seconds_per_beat = 60.0 / bpm;
                let seconds_per_measure = seconds_per_beat * time_sig_num as f64;
                let num_measures = (section_duration / seconds_per_measure).ceil() as i32;

                // Determine the content start (SONGSTART marker position)
                let content_start = song
                    .as_ref()
                    .map(|song| song.start_seconds + song.count_in_seconds.unwrap_or(0.0))
                    .unwrap_or(0.0);

                // Calculate measure number relative to content start (SONGSTART = measure 1)
                // Measures before SONGSTART are 0, -1, -2, etc.
                let section_start_measure =
                    ((section_start - content_start) / seconds_per_measure).floor() as i32;

                (0..num_measures)
                    .map(|i| {
                        let position_percent =
                            (i as f64 * seconds_per_measure / section_duration) * 100.0;
                        let measure_from_content_start = section_start_measure + i;

                        let display_number = if measure_from_content_start >= 0 {
                            measure_from_content_start + 1
                        } else {
                            measure_from_content_start
                        };

                        MeasureIndicator {
                            position_percent: position_percent.min(100.0),
                            measure_number: display_number,
                            time_signature: Some((time_sig_num as u8, time_sig_denom as u8)),
                            musical_position: daw_proto::MusicalPosition::new(
                                measure_from_content_start,
                                0,
                                0,
                            ),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    });

    // Memoize section-specific comment markers - reads ACTIVE_INDICES to ensure reactivity
    let section_comment_markers = use_memo(move || {
        let _indices = ACTIVE_INDICES.read();
        let section = current_section.read();
        let song = current_song.read();
        section
            .as_ref()
            .and_then(|section| {
                song.as_ref().map(|song| {
                    let section_duration = section.end_seconds - section.start_seconds;
                    if section_duration <= 0.0 {
                        return Vec::new();
                    }

                    song.comments
                        .iter()
                        .filter_map(|comment| {
                            if comment.position_seconds >= section.start_seconds
                                && comment.position_seconds < section.end_seconds
                            {
                                let position_percent = ((comment.position_seconds
                                    - section.start_seconds)
                                    / section_duration)
                                    * 100.0;
                                Some(CommentMarker {
                                    position_percent: position_percent.clamp(0.0, 100.0),
                                    text: comment.text.clone(),
                                    color: comment.color_hex(),
                                    is_count_in: comment.is_count_in,
                                    section_only: comment.section_only,
                                    position_seconds: comment.position_seconds,
                                })
                            } else {
                                None
                            }
                        })
                        .collect()
                })
            })
            .unwrap_or_default()
    });

    // Build loop indicator data from transport state
    // The loop_region in TransportState is already in percentages (0.0-1.0)
    let loop_indicator = transport_state.as_ref().and_then(|t| {
        if t.is_looping {
            t.loop_region.map(|(start, end)| LoopIndicatorData {
                enabled: true,
                start_percent: start * 100.0, // Convert to percentage (0-100)
                end_percent: end * 100.0,
            })
        } else {
            None
        }
    });

    // Capture song_index for closures
    let song_index_for_section_click = song_index;

    rsx! {
        div {
            class: "flex-1 flex flex-col overflow-hidden bg-background",

            // Scrollable content area
            div {
                class: "flex-1 overflow-y-auto",
                div {
                    class: "p-6 relative flex items-center justify-center h-full",

                    // Song Title (positioned above progress bar)
                    div {
                        class: "absolute left-0 right-0",
                        style: "bottom: calc(50% + 4rem);",
                        if let Some(ref song) = *current_song.read() {
                            SongTitle {
                                song_name: song.name.clone(),
                            }
                        }
                    }

                    // Main Song Progress Bar (centered)
                    div {
                        key: "{song_key.clone().unwrap_or_else(|| \"none\".to_string())}",
                        class: "w-full px-4",
                        if !progress_sections.read().is_empty() {
                            SongProgressBar {
                                progress: song_progress,
                                sections: progress_sections.read().clone(),
                                on_section_click: Some(Callback::new(move |section_idx: usize| {
                                    if let Some(song_idx) = song_index_for_section_click {
                                        spawn(async move {
                                            let _ = Session::get().setlist().seek_to_section(song_idx, section_idx).await;
                                        });
                                    }
                                })),
                                on_comment_click: Some(Callback::new({
                                    move |position_seconds: f64| {
                                        if let Some(song_idx) = song_index {
                                            spawn(async move {
                                                tracing::info!(
                                                    "Seeking to comment at {:.2}s in song {}",
                                                    position_seconds,
                                                    song_idx
                                                );
                                                let _ = Session::get()
                                                    .setlist()
                                                    .seek_to_time(song_idx, position_seconds)
                                                    .await;
                                            });
                                        }
                                    }
                                })),
                                tempo_markers: tempo_markers.read().clone(),
                                comment_markers: comment_markers.read().clone(),
                                song_key: song_key.clone(),
                                loop_indicator: loop_indicator.clone(),
                                queued_target: queued_target.clone(),
                            }
                        }
                    }

                    // Section Progress Bar (positioned below song progress bar)
                    div {
                        class: "absolute left-0 right-0",
                        style: "top: calc(50% + 6.5rem);",
                        div {
                            class: "w-full px-4",
                            SectionProgressBar {
                                progress: section_progress,
                                sections: progress_sections.read().clone(),
                                measure_indicators: measure_indicators.read().clone(),
                                comment_markers: section_comment_markers.read().clone(),
                                song_key: song_key.clone(),
                                on_measure_click: Some(Callback::new({
                                    move |musical_position: daw_proto::MusicalPosition| {
                                        if let Some(song_idx) = song_index {
                                            // The measure field contains the absolute measure number
                                            let measure = musical_position.measure;
                                            spawn(async move {
                                                tracing::info!(
                                                    "Going to measure {} in song {}",
                                                    measure,
                                                    song_idx
                                                );
                                                let _ = Session::get()
                                                    .setlist()
                                                    .goto_measure(song_idx, measure)
                                                    .await;
                                            });
                                        }
                                    }
                                })),
                                on_comment_click: Some(Callback::new({
                                    move |position_seconds: f64| {
                                        if let Some(song_idx) = song_index {
                                            spawn(async move {
                                                tracing::info!(
                                                    "Seeking to comment at {:.2}s in song {}",
                                                    position_seconds,
                                                    song_idx
                                                );
                                                let _ = Session::get()
                                                    .setlist()
                                                    .seek_to_time(song_idx, position_seconds)
                                                    .await;
                                            });
                                        }
                                    }
                                })),
                                queued_target: queued_target.clone(),
                            }
                        }
                    }

                    // Detail badges and next song (positioned below section progress bar)
                    div {
                        class: "absolute left-0 right-0",
                        style: "top: calc(50% + 10.5rem);",
                        div {
                            class: "flex flex-col items-center gap-4",

                            // Transport info badges
                            if let Some(ref transport) = transport_state {
                                {
                                    // Get time position from transport Position struct
                                    let position_seconds = transport.position.time.map(|t| t.as_seconds()).unwrap_or(0.0);
                                    let minutes = (position_seconds / 60.0).floor() as i32;
                                    let seconds = (position_seconds % 60.0).floor() as i32;
                                    let millis = ((position_seconds % 1.0) * 1000.0).floor() as i32;
                                    let time_str = format!("{}:{:02}.{:03}", minutes, seconds, millis);

                                    // Get musical position from transport Position struct
                                    // This comes from REAPER's TimeMap2_timeToBeats and properly handles tempo changes
                                    let musical_str = if let Some(ref musical) = transport.position.musical {
                                        format!("{}.{}.{:03}", musical.measure, musical.beat, musical.subdivision)
                                    } else {
                                        "1.1.000".to_string()
                                    };

                                    rsx! {
                                        div {
                                            class: "flex flex-wrap gap-3 justify-center",

                                            // Musical position badge (Measure.Beat.Subdivision)
                                            div {
                                                class: "px-4 py-2 rounded-full bg-secondary text-secondary-foreground text-base font-medium flex items-center gap-2",
                                                // Ruler icon
                                                svg {
                                                    width: "18",
                                                    height: "18",
                                                    view_box: "0 0 24 24",
                                                    fill: "none",
                                                    stroke: "currentColor",
                                                    stroke_width: "2",
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    // Simple ruler icon
                                                    line { x1: "3", y1: "12", x2: "21", y2: "12" }
                                                    line { x1: "3", y1: "8", x2: "3", y2: "16" }
                                                    line { x1: "7", y1: "10", x2: "7", y2: "14" }
                                                    line { x1: "11", y1: "8", x2: "11", y2: "16" }
                                                    line { x1: "15", y1: "10", x2: "15", y2: "14" }
                                                    line { x1: "19", y1: "8", x2: "19", y2: "16" }
                                                    line { x1: "21", y1: "8", x2: "21", y2: "16" }
                                                }
                                                span {
                                                    class: "font-mono tabular-nums",
                                                    "{musical_str}"
                                                }
                                            }

                                            // Time position badge (MM:SS.mmm)
                                            div {
                                                class: "px-4 py-2 rounded-full bg-secondary text-secondary-foreground text-base font-medium flex items-center gap-2",
                                                // Clock icon
                                                svg {
                                                    width: "18",
                                                    height: "18",
                                                    view_box: "0 0 24 24",
                                                    fill: "none",
                                                    stroke: "currentColor",
                                                    stroke_width: "2",
                                                    stroke_linecap: "round",
                                                    stroke_linejoin: "round",
                                                    circle { cx: "12", cy: "12", r: "10" }
                                                    polyline { points: "12 6 12 12 16 14" }
                                                }
                                                span {
                                                    class: "font-mono tabular-nums",
                                                    "{time_str}"
                                                }
                                            }

                                            // BPM badge
                                            div {
                                                class: "px-4 py-2 rounded-full bg-secondary text-secondary-foreground text-base font-medium",
                                                "{transport.bpm:.0} BPM"
                                            }

                                            // Time signature badge
                                            div {
                                                class: "px-4 py-2 rounded-full bg-secondary text-secondary-foreground text-base font-medium",
                                                "{transport.time_sig_num}/{transport.time_sig_denom}"
                                            }

                                            // Loop indicator badge
                                            if transport.is_looping {
                                                div {
                                                    class: "px-4 py-2 rounded-full bg-yellow-500/20 text-yellow-500 text-base font-medium",
                                                    "Loop ON"
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Next song (faded)
                            if let Some((next_name, next_color)) = next_song_info {
                                FadedSongTitle {
                                    song_name: next_name,
                                    color: next_color,
                                }
                            }
                        }
                    }
                }
            }

        }
    }
}
