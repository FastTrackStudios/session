//! The picture cache: one recorded scene per folded view.
//!
//! The same pattern as the docked stack's `stack_cache: Option<(ViewKey,
//! Scene)>` in `expression.rs` — a key that names everything the picture
//! depends on, a recorded `anyrender::Scene` beside it, and a frame that
//! replays rather than rebuilds when the key has not moved.
//!
//! The one thing that differs from the arrangement's own rectangles: the
//! **zoom is in the key**. The arrangement records its lanes once at one
//! pixel per second and scales them at replay, which works because a
//! rectangle scaled is the same rectangle. A fold is not — it lives on a
//! column grid, and a grid at a different zoom is a different fold,
//! which is the same reason `.reapeaks` keeps three mipmap levels rather
//! than one.

use anyrender::Scene;

use super::fold::GroupBy;

/// Everything the folded picture depends on.
///
/// The shape the decision on #27 settled: `(folder revision, mute mask,
/// take, zoom bucket, style, rule)`. Two of those six are narrower here
/// than in the prototype, which was comparing variants:
///
/// - **style** is the grouping. The prototype drew seven; one — "pieces"
///   — won, and the surviving choice is whether the groups are roles or
///   sides.
/// - **rule** is fixed at `min(min)`/`max(max)` and so is not a field.
///   The mean was the thing being compared against and it lost; there is
///   no way to ask for it, which is why there is nothing to key on.
///
/// `mute_mask` is carried even though [`revision`](Self::revision)
/// already moves when a mute does: the mask is what a reader of a cache
/// dump needs to see to know *why* two pictures of one folder differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PictureKey {
    /// The folder's revision — moves when any child's peaks or mute
    /// change, and does not move when one is hidden.
    pub revision: u64,
    /// A bit per child, set when it is out of the sum.
    pub mute_mask: u64,
    /// Which take the item is of.
    pub take: usize,
    /// The zoom, bucketed to whole columns per second.
    pub columns_per_sec: i32,
    /// Roles or sides.
    pub group_by: GroupBy,
}

/// The pictures kept until what they depend on changes.
#[derive(Default)]
pub struct PictureCache {
    entries: Vec<(PictureKey, Scene)>,
    hits: usize,
    misses: usize,
}

impl PictureCache {
    /// How many keys are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether anything is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replays served, and pictures built.
    #[must_use]
    pub const fn counts(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    /// The scene for `key`, built by `build` when it is not already
    /// held.
    ///
    /// r[impl flow.drums.comping.folder-items]
    pub fn get_or_build(&mut self, key: PictureKey, build: impl FnOnce() -> Scene) -> Option<&Scene> {
        let found = self.entries.iter().position(|(k, _)| *k == key);
        let at = match found {
            Some(at) => {
                self.hits = self.hits.saturating_add(1);
                at
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                self.entries.push((key, build()));
                self.entries.len().saturating_sub(1)
            }
        };
        self.entries.get(at).map(|(_, scene)| scene)
    }

    /// Drop every picture of a folder whose children changed, keeping
    /// the ones at `revision`.
    pub fn retain_revision(&mut self, revision: u64) {
        self.entries.retain(|(k, _)| k.revision == revision);
    }
}
