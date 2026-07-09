//! Action definitions for the Visibility Manager.
//!
//! Each action maps to a dynamic-template top-level group. The `LocalActionBinder`
//! trait is implemented by reaper-extension to toggle REAPER track visibility.

actions_proto::define_actions! {
    pub visibility_manager_actions {
        prefix: "fts.visibility_manager",
        title: "Visibility Manager",

        // ── Per-group toggles (matches dynamic-template default_config order) ──
        TOGGLE_DRUMS = "toggle_drums" {
            name: "Toggle Drums",
            description: "Toggle visibility of all Drums tracks",
            category: View,
        }
        TOGGLE_PERCUSSION = "toggle_percussion" {
            name: "Toggle Percussion",
            description: "Toggle visibility of all Percussion tracks",
            category: View,
        }
        TOGGLE_BASS = "toggle_bass" {
            name: "Toggle Bass",
            description: "Toggle visibility of all Bass tracks",
            category: View,
        }
        TOGGLE_GUITARS = "toggle_guitars" {
            name: "Toggle Guitars",
            description: "Toggle visibility of all Guitars tracks",
            category: View,
        }
        TOGGLE_KEYS = "toggle_keys" {
            name: "Toggle Keys",
            description: "Toggle visibility of all Keys tracks",
            category: View,
        }
        TOGGLE_SYNTHS = "toggle_synths" {
            name: "Toggle Synths",
            description: "Toggle visibility of all Synths tracks",
            category: View,
        }
        TOGGLE_HORNS = "toggle_horns" {
            name: "Toggle Horns",
            description: "Toggle visibility of all Horns tracks",
            category: View,
        }
        TOGGLE_HARMONICA = "toggle_harmonica" {
            name: "Toggle Harmonica",
            description: "Toggle visibility of all Harmonica tracks",
            category: View,
        }
        TOGGLE_STRINGS = "toggle_strings" {
            name: "Toggle Strings",
            description: "Toggle visibility of all Strings tracks",
            category: View,
        }
        TOGGLE_VOCALS = "toggle_vocals" {
            name: "Toggle Vocals",
            description: "Toggle visibility of all Vocals tracks",
            category: View,
        }
        TOGGLE_CHOIR = "toggle_choir" {
            name: "Toggle Choir",
            description: "Toggle visibility of all Choir tracks",
            category: View,
        }
        TOGGLE_ORCHESTRA = "toggle_orchestra" {
            name: "Toggle Orchestra",
            description: "Toggle visibility of all Orchestra tracks",
            category: View,
        }
        TOGGLE_SFX = "toggle_sfx" {
            name: "Toggle SFX",
            description: "Toggle visibility of all SFX tracks",
            category: View,
        }
        TOGGLE_GUIDE = "toggle_guide" {
            name: "Toggle Guide",
            description: "Toggle visibility of all Guide tracks",
            category: View,
        }
        TOGGLE_REFERENCE = "toggle_reference" {
            name: "Toggle Reference",
            description: "Toggle visibility of all Reference tracks",
            category: View,
        }
        TOGGLE_STEM_SPLIT = "toggle_stem_split" {
            name: "Toggle Stem Split",
            description: "Toggle visibility of all Stem Split tracks",
            category: View,
        }

        // ── Global operations ───────────────────────────────────────
        SHOW_ALL = "show_all" {
            name: "Show All",
            description: "Show all tracks (reset visibility)",
            category: View,
        }
        HIDE_ALL = "hide_all" {
            name: "Hide All",
            description: "Hide all group tracks",
            category: View,
        }
        PROFILE_DRUM_EDITING = "profile_drum_editing" {
            name: "Drum Editing Profile",
            description: "Show and size drum tracks for editing, hiding unrelated tracks",
            category: View,
        }
        PROFILE_MIDI_EDITING = "profile_midi_editing" {
            name: "MIDI Editing Profile",
            description: "Show and size MIDI-oriented template groups for editing",
            category: View,
        }
        // ── Per-mode visibility (rule-based, applied on mode switch) ─────────
        // One action per session Mode (slug matches session::Mode::slug()). The
        // fts-extensions mode-change listener fires the matching action; the
        // handler resolves that mode's rule set against the live session and
        // applies per-surface (arrange/mixer) visibility + folder-collapse.
        MODE_ORGANIZE = "mode_organize" {
            name: "Visibility: Organize Mode",
            description: "Apply the Organize mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_WRITE = "mode_write" {
            name: "Visibility: Write Mode",
            description: "Apply the Write mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_PRODUCE = "mode_produce" {
            name: "Visibility: Produce Mode",
            description: "Apply the Produce mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_RECORD = "mode_record" {
            name: "Visibility: Record Mode",
            description: "Apply the Record mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_EDIT = "mode_edit" {
            name: "Visibility: Edit Mode",
            description: "Apply the Edit mode visibility rules (mixer shows collapsed buses, arrange shows one audio track per instrument)",
            category: View,
            group: "Modes",
        }
        MODE_MIX = "mode_mix" {
            name: "Visibility: Mix Mode",
            description: "Apply the Mix mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_MASTER = "mode_master" {
            name: "Visibility: Master Mode",
            description: "Apply the Master mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_LIVE = "mode_live" {
            name: "Visibility: Live Mode",
            description: "Apply the Live mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_VIDEO = "mode_video" {
            name: "Visibility: Video Mode",
            description: "Apply the Video mode visibility rules",
            category: View,
            group: "Modes",
        }
        MODE_SCORING = "mode_scoring" {
            name: "Visibility: Scoring Mode",
            description: "Apply the Scoring mode visibility rules",
            category: View,
            group: "Modes",
        }

        REBUILD_CACHE = "rebuild_cache" {
            name: "Rebuild Cache",
            description: "Rebuild the track-to-group classification cache",
            category: Dev,
            group: "Dev",
        }
    }
}
