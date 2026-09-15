//! The hit list: where a drummer actually hit, kept as stretch markers.
//!
//! # Why stretch markers, and not a list of our own
//!
//! A hit is a position in a take that the editor can move. REAPER
//! already has exactly that — a stretch marker — and it survives a save,
//! travels with the take, and is what the editor's own slip gesture
//! already writes. Keeping a parallel hit list beside it would mean two
//! things that must agree about every take in the session, and they
//! would stop agreeing the first time anyone edited in REAPER directly.
//!
//! So a detected hit and a hand-placed one are **the same thing**, and
//! slip, nudge and quantize are ordinary marker moves.
//!
//! # Why every mic of the piece, always
//!
//! A kick is a kick-in mic and a kick-out mic and often a trigger, all
//! hearing the same stick. Moving the hit on one of them and not the
//! others does not "edit the kick" — it smears it, and the damage is
//! phase, which no later edit repairs. So the list is written to every
//! mic at the same positions, in **one undo step**, because an edit an
//! engineer has to undo six times is an edit they will get half of.
//!
//! # Why it lands on EDIT
//!
//! The markers go on the take of each mic's **EDIT** comp, never on the
//! take lanes and never on COMP (see [`crate::comping`]). Detection is
//! destructive to a hit list — re-detecting replaces it — and that is
//! only safe because the thing it replaces is a derived stage that can
//! be made again.

use daw_proto::stretch_marker::{StretchMarker, StretchMarkers, StretchTakeRef};
use daw_proto::{DawResult, ProjectContext, Projects};

/// A hit: where in the take the stick landed.
///
/// One number, because that is all a hit is. Velocity is *measured from
/// the audio* when a trigger is rendered rather than stored here — a
/// marker carries no level, and a stored one would go stale the moment
/// the hit moved.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Hit {
    /// Position in the take, seconds.
    pub at: f64,
}

impl Hit {
    /// The marker a hit becomes.
    ///
    /// Position and source position are the same until something slips
    /// it: an undisturbed hit maps its own place in the source to its
    /// own place in the take, which is the identity that makes writing
    /// a fresh detection a no-op on the audio.
    #[must_use]
    pub const fn marker(self) -> StretchMarker {
        StretchMarker {
            position: self.at,
            source_position: self.at,
            slope: 0.0,
        }
    }
}

/// What a hit-list write touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wrote {
    /// Mics that took the list.
    pub mics: usize,
    /// Hits written to each.
    pub hits: usize,
}

/// Replace the hit list on every mic of a piece, as one undo step.
///
/// Replace rather than merge: a detection describes the whole take, so
/// merging would leave the hits of a previous, worse detection behind
/// with nothing to distinguish them.
///
/// r[impl flow.drums.editing.stack]
pub fn write<D>(
    daw: &D,
    project: &ProjectContext,
    mics: &[StretchTakeRef],
    hits: &[Hit],
) -> DawResult<Wrote>
where
    D: StretchMarkers + Projects + ?Sized,
{
    let label = "Detect hits";
    daw.begin_undo_block(project.clone(), label);
    let mut out = Wrote {
        mics: 0,
        hits: hits.len(),
    };
    let markers: Vec<StretchMarker> = hits.iter().copied().map(Hit::marker).collect();
    let result = (|| -> DawResult<()> {
        for mic in mics {
            daw.set_stretch_markers(mic.clone(), markers.clone())?;
            out.mics += 1;
        }
        Ok(())
    })();
    daw.end_undo_block(project.clone(), label, None);
    result.map(|()| out)
}

/// Move one hit on every mic of the piece, as one undo step.
///
/// The index is the hit's place in the list, which is the same on every
/// mic because the list was written to all of them at once. Moving by
/// index rather than by searching near a time is what keeps the mics
/// from drifting apart when two hits are close together.
///
/// r[impl flow.drums.editing.hands]
pub fn slip<D>(
    daw: &D,
    project: &ProjectContext,
    mics: &[StretchTakeRef],
    index: u32,
    to: f64,
) -> DawResult<Wrote>
where
    D: StretchMarkers + Projects + ?Sized,
{
    let label = "Slip hit";
    daw.begin_undo_block(project.clone(), label);
    let mut out = Wrote { mics: 0, hits: 1 };
    let result = (|| -> DawResult<()> {
        for mic in mics {
            let mut markers =
                daw.get_stretch_markers(mic.project.clone(), mic.item.clone(), mic.take.clone());
            let Some(marker) = markers.get_mut(index as usize) else {
                continue;
            };
            marker.position = to;
            daw.set_stretch_markers(mic.clone(), markers)?;
            out.mics += 1;
        }
        Ok(())
    })();
    daw.end_undo_block(project.clone(), label, None);
    result.map(|()| out)
}

/// The hit list as read back from one mic.
///
/// # Errors
///
/// When the take's markers cannot be read.
pub fn read<D: StretchMarkers + ?Sized>(daw: &D, mic: &StretchTakeRef) -> DawResult<Vec<Hit>> {
    Ok(daw
        .get_stretch_markers(mic.project.clone(), mic.item.clone(), mic.take.clone())
        .into_iter()
        .map(|marker| Hit {
            at: marker.position,
        })
        .collect())
}
