//! Action definitions for Auto Color.
//!
//! Each action maps to a color operation on DAW tracks. The engine crate
//! (`dynamic-template`) provides the classification and application logic;
//! the reaper-extension wires these action IDs to actual handlers.

actions_proto::define_actions! {
    pub auto_color_actions {
        prefix: "fts.auto_color",
        title: "Auto Color",

        COLOR_ALL = "color_all" {
            name: "Auto Color All Tracks",
            description: "Classify all tracks by instrument group and apply colors",
            category: General,
        }
        COLOR_SELECTED = "color_selected" {
            name: "Auto Color Selected",
            description: "Classify selected tracks by instrument group and apply colors",
            category: General,
        }
        TOGGLE = "toggle" {
            name: "Toggle Auto Color",
            description: "Toggle auto-color on/off (applies or clears all track colors)",
            category: General,
        }
        CLEAR_ALL = "clear_all" {
            name: "Clear All Track Colors",
            description: "Reset colors on all tracks to default",
            category: General,
        }
        CLEAR_SELECTED = "clear_selected" {
            name: "Clear Selected Track Colors",
            description: "Reset colors on selected tracks to default",
            category: General,
        }
    }
}
