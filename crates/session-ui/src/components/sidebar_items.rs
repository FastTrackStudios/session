//! Sidebar Item Components
//!
//! Components for displaying songs and sections in the sidebar navigator.
//! Copied from FastTrackStudio for exact styling match.

use crate::components::progress::CompactProgressBar;
use crate::prelude::*;

/// Vertical progress bar component for sidebar
///
/// A simple vertical progress indicator.
#[component]
pub fn VerticalProgressBar(
    progress: f64,
    height: String,
    bright_color: String,
    muted_color: String,
) -> Element {
    rsx! {
        div {
            class: "relative w-1 flex-shrink-0 mr-2",
            style: format!("height: {};", height),
            // Background (unfilled)
            div {
                class: "absolute inset-0 rounded-full",
                style: format!(
                    "background-color: {}; opacity: 0.3;",
                    muted_color
                ),
            }
            // Filled portion
            div {
                class: "absolute top-0 left-0 right-0 rounded-full transition-all duration-300 ease-in-out",
                style: format!(
                    "height: {}%; background-color: {}; opacity: 0.8;",
                    progress.clamp(0.0, 100.0),
                    bright_color
                ),
            }
        }
    }
}

/// Section item data for sidebar
#[derive(Clone, Debug, PartialEq)]
pub struct SectionItem {
    pub label: String,
    pub progress: f64,
    pub bright_color: String,
    pub muted_color: String,
    /// Optional comment/descriptor shown in italic after the label
    pub comment: Option<String>,
}

/// Song item data for sidebar
#[derive(Clone, Debug, PartialEq)]
pub struct SongItemData {
    pub label: String,
    pub progress: f64,
    pub bright_color: String,
    pub muted_color: String,
    pub sections: Vec<SectionItem>,
}

/// Song item component for sidebar
///
/// A generic component that displays a song with its sections.
/// All data is passed as props, keeping it domain-agnostic.
#[component]
pub fn SongItem(
    song_data: SongItemData,
    index: usize,
    is_expanded: bool,
    is_playing: bool,
    current_section_index: Option<usize>,
    on_song_click: Callback<()>,
    on_section_click: Callback<usize>,
) -> Element {
    // Show progress if the song is expanded OR if it's actively playing
    let show_progress = is_expanded || is_playing;

    rsx! {
        div {
            key: "{index}",
            class: "relative",
            div {
                class: "flex",
                // Vertical progress bar (only show if expanded)
                if is_expanded {
                    VerticalProgressBar {
                        progress: song_data.progress,
                        height: format!(
                            "calc(3rem + {} * 3rem + {} * 0.25rem);",
                            song_data.sections.len(),
                            song_data.sections.len()
                        ),
                        bright_color: song_data.bright_color.clone(),
                        muted_color: song_data.muted_color.clone(),
                    }
                }
                div {
                    class: "flex-1 space-y-1",
                    // Song progress bar
                    CompactProgressBar {
                        label: song_data.label.clone(),
                        progress: song_data.progress,
                        bright_color: song_data.bright_color.clone(),
                        muted_color: song_data.muted_color.clone(),
                        is_selected: is_expanded,
                        is_inactive: !show_progress,
                        always_black_bg: true,
                        on_click: Some(Callback::new(move |_| {
                            on_song_click.call(());
                        })),
                    }
                    // Sections (only if expanded)
                    if is_expanded {
                        for (section_idx, section) in song_data.sections.iter().enumerate() {
                            div {
                                key: "{index}-{section_idx}",
                                class: "pl-4",
                                CompactProgressBar {
                                    label: section.label.clone(),
                                    progress: section.progress,
                                    bright_color: section.bright_color.clone(),
                                    muted_color: section.muted_color.clone(),
                                    is_selected: current_section_index == Some(section_idx),
                                    is_inactive: !show_progress,
                                    on_click: {
                                        let section_idx = section_idx;
                                        Some(Callback::new(move |_| {
                                            on_section_click.call(section_idx);
                                        }))
                                    },
                                    comment: section.comment.clone(),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
