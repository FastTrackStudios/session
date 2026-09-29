//! Which folders are open: the window's own choice over the layout's.
//!
//! The scene decides a first layout (`plan::apply_scene`); a person then
//! opens and shuts folders — from a folder's button in the arrangement or
//! at the foot of its mixer strip — and can fold every top-level folder at
//! once ([`set_tops`]): the whole session as its families, the drums one
//! row and one strip, to see everything that is going on and open only
//! what is being worked on.
//!
//! One state for the window, so the arrangement and the mixer always show
//! the same folders; applied where rows are planned
//! (`studio::plan_rows_with`), so a folder shut here gets its children's
//! items folded onto its row, as a folder the scene shut does. Every
//! change bumps [`generation`], which the views compare each frame to plan
//! their rows again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

struct State {
    /// A folder's own choice, by guid: open (`true`) or shut.
    chosen: HashMap<String, bool>,
    /// Every top-level folder shut unless chosen open.
    tops: bool,
}

impl Default for State {
    /// Every top-level folder folded: a session opens as its families,
    /// one row and one strip each, opened one by one. Not under test,
    /// where rows are what they are planned as.
    fn default() -> Self {
        Self {
            chosen: HashMap::new(),
            tops: !cfg!(test),
        }
    }
}

#[cfg(not(test))]
static STATE: std::sync::LazyLock<Mutex<State>> =
    std::sync::LazyLock::new(|| Mutex::new(State::default()));
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(not(test))]
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// Under test, each thread's own: a test folding folders must not fold the
// rows another test is planning beside it.
#[cfg(test)]
thread_local! {
    static STATE: &'static Mutex<State> = Box::leak(Box::default());
}

#[cfg(test)]
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .with(|state| *state)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn changed() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// How many times the folds have changed: a view planning rows compares
/// this with the value it planned at.
#[must_use]
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Open the folder `guid` if it is shut (`open` = whether it shows open
/// now), shut it if it is open.
pub fn toggle(guid: &str, open: bool) {
    state().chosen.insert(guid.to_owned(), !open);
    changed();
}

/// Whether every top-level folder is folded.
#[must_use]
pub fn tops() -> bool {
    state().tops
}

/// Fold every top-level folder (`true`), or open them again (`false`).
/// Either way the folders chosen one by one start over: the switch is a
/// fresh overview, not a filter over earlier choices.
pub fn set_tops(on: bool) {
    let mut state = state();
    state.tops = on;
    state.chosen.clear();
    drop(state);
    changed();
}

/// `rows` (tracks and their depths, in order) with the shut folders'
/// contents left out: each folder keeps its row.
#[must_use]
pub fn apply(rows: Vec<(daw_proto::Track, u32)>) -> Vec<(daw_proto::Track, u32)> {
    let state = state();
    if !state.tops && state.chosen.is_empty() {
        return rows;
    }
    let mut out = Vec::with_capacity(rows.len());
    // The depth of the shut folder whose contents are being skipped.
    let mut skipping: Option<u32> = None;
    for (track, depth) in rows {
        if let Some(shut) = skipping {
            if depth > shut {
                continue;
            }
            skipping = None;
        }
        if track.is_folder {
            let open = state
                .chosen
                .get(&track.guid)
                .copied()
                .unwrap_or(!(state.tops && depth == 0));
            if !open {
                skipping = Some(depth);
            }
        }
        out.push((track, depth));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, folder: bool, depth: u32) -> (daw_proto::Track, u32) {
        let track = daw_proto::Track {
            guid: name.to_owned(),
            name: name.to_owned(),
            is_folder: folder,
            ..daw_proto::Track::default()
        };
        (track, depth)
    }

    fn names(rows: &[(daw_proto::Track, u32)]) -> Vec<&str> {
        rows.iter().map(|(t, _)| t.name.as_str()).collect()
    }

    #[test]
    fn folders_fold_by_choice_and_all_at_once() {
        let rows = vec![
            row("Click", false, 0),
            row("Drums", true, 0),
            row("Kick", false, 1),
            row("Toms", true, 1),
            row("Tom 1", false, 2),
            row("Bass", true, 0),
            row("DI", false, 1),
        ];
        assert_eq!(
            names(&apply(rows.clone())).len(),
            7,
            "nothing chosen, nothing folded"
        );

        toggle("Drums", true);
        assert_eq!(
            names(&apply(rows.clone())),
            ["Click", "Drums", "Bass", "DI"],
            "a shut folder keeps its row and hides what is in it, however deep"
        );

        set_tops(true);
        assert_eq!(names(&apply(rows.clone())), ["Click", "Drums", "Bass"]);
        toggle("Bass", false);
        assert_eq!(
            names(&apply(rows.clone())),
            ["Click", "Drums", "Bass", "DI"],
            "one folder opened out of the folded whole"
        );

        set_tops(false);
        assert_eq!(
            names(&apply(rows)).len(),
            7,
            "and back, every choice forgotten"
        );
    }
}
