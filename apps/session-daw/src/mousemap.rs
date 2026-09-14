//! What a gesture means where it lands.
//!
//! REAPER's mouse behaviour is a table: for each CONTEXT — a media
//! item, its left edge, a fade, the ruler, the empty arrange area —
//! and each modifier combination, an action. [`crate::hit`] answers
//! "what did you touch, and in which context"; this answers "and what
//! does that do", so the two stay separately configurable: a hit test
//! is geometry, a mouse map is preference.
//!
//! The table here is the FTS default. A mouse profile
//! (`input_config_proto::MouseProfileConfig`) is the same shape — a
//! context, a modifier set, a behaviour — and is where a user's
//! overrides will load into once the profile system reaches this
//! window; the contexts are already its contexts.

use input_config_proto::MouseModifierContext as Context;

/// The keys held during a gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// The gesture itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Click,
    DoubleClick,
    Drag,
}

/// What a gesture does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Move the edit cursor (and the transport) to the time.
    SetEditCursor,
    /// Drag out a time selection.
    TimeSelection,
    /// Drag the fade-in's length.
    FadeIn,
    /// Drag the fade-out's length.
    FadeOut,
    /// Change the fade's shape rather than its length.
    FadeShape,
    /// Move the item along its lane.
    MoveItem,
    /// Trim the item's left edge.
    TrimLeft,
    /// Trim the item's right edge.
    TrimRight,
    /// Select the item.
    SelectItem,
    /// Nothing bound here.
    Nothing,
}

/// The FTS default: what each context does under each gesture.
///
/// Stated as a table so it reads like the preference page it will
/// become, and so a profile override is a row that wins over one of
/// these rather than a branch somewhere in the window.
#[must_use]
pub fn resolve(context: Context, gesture: Gesture, mods: Mods) -> Action {
    use Gesture as G;
    match (context, gesture) {
        // The ruler: a click places the cursor, a drag selects time.
        (Context::Ruler, G::Click) => Action::SetEditCursor,
        (Context::Ruler, G::Drag) => Action::TimeSelection,
        // The empty arrange area does the same — the ruler is just
        // where the numbers are.
        (Context::ArrangeView, G::Click) => Action::SetEditCursor,
        (Context::ArrangeView, G::Drag) => Action::TimeSelection,
        // A fade's handle: drag its length; with Shift, its shape.
        (Context::MediaItemFade, G::Drag) if mods.shift => Action::FadeShape,
        (Context::MediaItemFade, G::Drag) => Action::FadeIn,
        // An item (REAPER calls the body its bottom half): click selects,
        // drag moves, its edges trim. The
        // moves and trims are bound here and not yet built — the
        // window says so rather than doing something else.
        (Context::MediaItemBottomHalf, G::Click) => Action::SelectItem,
        (Context::MediaItemBottomHalf, G::Drag) => Action::MoveItem,
        (Context::MediaItemLeftEdge, G::Drag) => Action::TrimLeft,
        (Context::MediaItemRightEdge, G::Drag) => Action::TrimRight,
        _ => Action::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_handle_drags_the_fade_and_shift_drags_its_shape() {
        assert_eq!(
            resolve(Context::MediaItemFade, Gesture::Drag, Mods::default()),
            Action::FadeIn
        );
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(
            resolve(Context::MediaItemFade, Gesture::Drag, shift),
            Action::FadeShape
        );
        assert_eq!(
            resolve(Context::MediaItemFade, Gesture::Click, Mods::default()),
            Action::Nothing
        );
    }

    #[test]
    fn the_ruler_and_the_empty_area_agree() {
        for context in [Context::Ruler, Context::ArrangeView] {
            assert_eq!(
                resolve(context, Gesture::Click, Mods::default()),
                Action::SetEditCursor
            );
            assert_eq!(
                resolve(context, Gesture::Drag, Mods::default()),
                Action::TimeSelection
            );
        }
    }

    #[test]
    fn a_mixer_strip_is_not_in_the_map() {
        assert_eq!(
            resolve(Context::MixerStrip, Gesture::Drag, Mods::default()),
            Action::Nothing
        );
    }
}
