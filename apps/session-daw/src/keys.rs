//! The keyboard, through the FTS keybind profile.
//!
//! The profile is the one REAPER runs on (`reaper-input`'s
//! `fasttrackstudio` — `transport.styx`, `editing.styx`,
//! `navigation.styx`, …), read by `input-keybinds` into an
//! `input::InputProcessor`, so a key means the same thing in this window
//! as it does in REAPER with the extension loaded. The bindings name
//! REAPER actions by number; [`Action`] is the subset this window can
//! do, and [`action_of`] is the table from one to the other. A bound
//! action the window cannot do yet comes back as [`Action::Unbound`]
//! with its id, so it is logged rather than silently dropped.
//!
//! Where the profile is found: `FTS_INPUT_PROFILE` (a profile
//! directory), else the daw checkout beside this repo, else a built-in
//! copy of the bindings this window needs — the same ids, so the two
//! never disagree about what a key does.

use std::path::{Path, PathBuf};

use input::{ActionContext, InputCommand, InputEvent, InputProcessor, KeyCode, KeymapConfig, Modifiers};

/// What a key asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    PlayStop,
    PlayPause,
    GoToStart,
    /// Split the selected items at the edit cursor — or, with nothing
    /// selected, every item under it.
    SplitAtCursor,
    DeleteSelectedItems,
    SelectAllItems,
    /// Escape: no time selection, nothing selected.
    ClearSelection,
    /// The edit cursor by a bar, or by a beat.
    CursorBar(i32),
    CursorBeat(i32),
    /// Track selection, up or down the panel; `extend` keeps the rest.
    TrackStep { by: i32, extend: bool },
    /// The edit cursor to the previous or next marker.
    Marker(i32),
    ToggleRecord,
    /// A binding the window does not do yet, by its REAPER id.
    Unbound(String),
}

/// REAPER's action ids, as the profile binds them, to what this window
/// does about them.
#[must_use]
pub fn action_of(id: &str) -> Action {
    match id {
        "40044" => Action::PlayStop,
        "40073" => Action::PlayPause,
        "40042" => Action::GoToStart,
        "40757" | "40012" => Action::SplitAtCursor,
        "40006" | "40697" => Action::DeleteSelectedItems,
        "40035" | "40182" => Action::SelectAllItems,
        "40020" => Action::ClearSelection,
        "40838" => Action::CursorBar(-1),
        "40837" => Action::CursorBar(1),
        "40646" => Action::CursorBeat(-1),
        "40647" => Action::CursorBeat(1),
        "40285" => Action::TrackStep { by: 1, extend: false },
        "40286" => Action::TrackStep { by: -1, extend: false },
        "40421" => Action::TrackStep { by: 1, extend: true },
        "40420" => Action::TrackStep { by: -1, extend: true },
        "40172" => Action::Marker(-1),
        "40173" => Action::Marker(1),
        "1013" => Action::ToggleRecord,
        other => Action::Unbound(other.to_owned()),
    }
}

/// The keyboard: the processor and where its map came from.
pub struct Keys {
    processor: InputProcessor,
    context: ActionContext,
    pub source: String,
}

impl Keys {
    /// Load the FTS profile, or fall back to the built-in bindings.
    #[must_use]
    pub fn load() -> Self {
        let (config, source) = profile_dir()
            .and_then(|dir| input_keybinds::load_profile_keymap(&dir).map(|c| (c, dir.display().to_string())))
            .or_else(|| KeymapConfig::from_json_str(BUILTIN).ok().map(|c| (c, "built-in".to_owned())))
            .unwrap_or_else(|| (KeymapConfig::default(), "none".to_owned()));
        let processor = InputProcessor::from_config(config).unwrap_or_else(|_| InputProcessor::new());
        Self {
            processor,
            context: ActionContext::new(),
            source,
        }
    }

    /// A key went down. The actions it asked for, in order — empty
    /// when the key is not bound, so the window can keep its own.
    pub fn press(&mut self, key: KeyCode, modifiers: Modifiers) -> Vec<Action> {
        let event = InputEvent::Key(input::KeyEvent { key, modifiers });
        self.processor
            .process(event, &self.context)
            .into_iter()
            .filter_map(|command| match command {
                InputCommand::Action(id) => Some(action_of(id.as_str())),
                InputCommand::ActionWithArgs { action, .. } => Some(action_of(action.as_str())),
                _ => None,
            })
            .collect()
    }

    /// What is buffered, waiting for the rest of a sequence — for a
    /// which-key hint.
    #[must_use]
    pub fn pending(&self) -> Option<String> {
        self.processor.pending_display()
    }
}

/// The profile directory: from the environment, else the daw checkout
/// beside this repo (the layout every FTS clone has).
fn profile_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("FTS_INPUT_PROFILE") {
        let dir = PathBuf::from(dir);
        return dir.join("profile.styx").exists().then_some(dir);
    }
    let candidates = [
        "../daw/features/reaper/reaper-input/config/config/fasttrackstudio",
        "../../daw/features/reaper/reaper-input/config/config/fasttrackstudio",
        "../../../daw/features/reaper/reaper-input/config/config/fasttrackstudio",
    ];
    candidates
        .iter()
        .map(Path::new)
        .find(|dir| dir.join("profile.styx").exists())
        .map(Path::to_path_buf)
}

/// A winit key as the processor's, with the shift folded out of the
/// character: the profile writes `Shift+e`, and the keyboard gives an
/// `E` — the chord is the letter and the modifier, not the capital.
#[must_use]
pub fn key_code(named: Option<&str>, text: Option<&str>) -> Option<KeyCode> {
    match named {
        Some("Enter") => return Some(KeyCode::Enter),
        Some("Escape") => return Some(KeyCode::Escape),
        Some("Tab") => return Some(KeyCode::Tab),
        Some("Backspace") => return Some(KeyCode::Backspace),
        Some("Delete") => return Some(KeyCode::Delete),
        Some("ArrowUp") => return Some(KeyCode::ArrowUp),
        Some("ArrowDown") => return Some(KeyCode::ArrowDown),
        Some("ArrowLeft") => return Some(KeyCode::ArrowLeft),
        Some("ArrowRight") => return Some(KeyCode::ArrowRight),
        Some(f) if f.len() <= 3 && f.starts_with('F') => {
            return f.get(1..).and_then(|n| n.parse::<u8>().ok()).map(KeyCode::F);
        }
        _ => {}
    }
    let text = text?;
    let mut chars = text.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(KeyCode::Character(first.to_lowercase().collect()))
}

/// The bindings this window needs, in the profile's own ids, for a
/// checkout without the profile beside it. Kept in step with
/// `fasttrackstudio/*.styx` by hand; the profile wins when it is there.
const BUILTIN: &str = r#"{
  "modes": { "normal": { "display_name": "NORMAL" } },
  "keymap": {
    "normal": {
      "Space": "40044",
      "Shift+Space": "40073",
      "Enter": "40042",
      "0": "40042",
      "s": "40757",
      "d": "40006",
      "Delete": "40697",
      "Backspace": "40697",
      "Escape": "40020",
      "Ctrl+a": "40035",
      "ArrowLeft": "40838",
      "ArrowRight": "40837",
      "h": "40838",
      "l": "40837",
      "Ctrl+h": "40646",
      "Ctrl+l": "40647",
      "ArrowUp": "40286",
      "ArrowDown": "40285",
      "j": "40285",
      "k": "40286",
      "Shift+j": "40421",
      "Shift+k": "40420",
      ",": "40172",
      ".": "40173",
      "r": "1013"
    }
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in map is the profile's ids: space plays, s splits,
    /// d deletes, and the two agree because they are one table.
    #[test]
    fn the_builtin_bindings_reach_the_actions() {
        let config = KeymapConfig::from_json_str(BUILTIN).expect("the built-in map parses");
        let mut keys = Keys {
            processor: InputProcessor::from_config(config).expect("a processor"),
            context: ActionContext::new(),
            source: "test".into(),
        };
        assert_eq!(keys.press(KeyCode::Character(" ".into()), Modifiers::NONE), vec![Action::PlayStop]);
        assert_eq!(keys.press(KeyCode::Character("s".into()), Modifiers::NONE), vec![Action::SplitAtCursor]);
        assert_eq!(keys.press(KeyCode::Delete, Modifiers::NONE), vec![Action::DeleteSelectedItems]);
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(keys.press(KeyCode::Character("a".into()), ctrl), vec![Action::SelectAllItems]);
        assert!(keys.press(KeyCode::Character("q".into()), Modifiers::NONE).is_empty());
    }

    /// A capital is the letter with shift, not another key.
    #[test]
    fn a_capital_is_the_letter_and_shift() {
        assert_eq!(key_code(None, Some("E")), Some(KeyCode::Character("e".into())));
        assert_eq!(key_code(Some("Delete"), None), Some(KeyCode::Delete));
        assert_eq!(key_code(Some("F3"), None), Some(KeyCode::F(3)));
    }

    #[test]
    fn an_unknown_id_is_reported_not_dropped() {
        assert_eq!(action_of("_FTS_SMART_DUPLICATE"), Action::Unbound("_FTS_SMART_DUPLICATE".into()));
    }
}
