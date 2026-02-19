use crate::prelude::*;

/// Song title component displaying the currently active song
///
/// A pure UI component that displays song information.
/// Accepts the song name as a prop.
#[component]
pub fn SongTitle(song_name: String) -> Element {
    rsx! {
        div {
            class: "text-center py-8",
            h1 {
                class: "text-6xl font-bold text-foreground mb-2",
                "{song_name}"
            }
        }
    }
}

/// Faded song title component for displaying the next song
///
/// A pure UI component that displays a faded song title with custom color.
#[component]
pub fn FadedSongTitle(song_name: String, color: String) -> Element {
    rsx! {
        div {
            class: "text-center py-4",
                p {
                class: "text-sm font-medium text-muted-foreground mb-2",
                "Next:"
            }
            h2 {
                class: "text-4xl font-bold",
                style: format!("color: {}; opacity: 0.4;", color),
                "{song_name}"
            }
        }
    }
}
