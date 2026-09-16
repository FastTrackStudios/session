//! The hit list across a piece's mics, driven and read back.
//!
//! The rule these prove is not "hits are stored": it is that **a piece's
//! mics never drift apart**. A kick is a kick-in, a kick-out and often a
//! trigger, all hearing the same stick, and moving the hit on one of
//! them smears it in a way no later edit repairs.

use daw_proto::primitives::{Duration, PositionInSeconds};
use daw_proto::stretch_marker::{StretchMarkers, StretchTakeRef};
use daw_proto::take::Takes;
use daw_proto::{ItemRef, Items, ProjectContext, ProjectInfo, TakeRef, TrackRef, Tracks};
use daw_standalone::sync::Standalone;
use session::hits::{Hit, read, slip, write};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

/// The kick, heard three ways.
const MICS: [&str; 3] = ["Kick In", "Kick Out", "Kick Trig"];

struct Kick {
    daw: Standalone,
    project: ProjectContext,
    takes: Vec<StretchTakeRef>,
}

fn kick() -> Kick {
    let daw = Standalone::new();
    let guid = daw.seed_project(ProjectInfo {
        guid: "kick".into(),
        name: "kick".into(),
        path: String::new(),
    });
    let project = ProjectContext::Project(guid);
    let takes = MICS
        .iter()
        .map(|name| {
            let track =
                TrackRef::Guid(Tracks::add(&daw, project.clone(), name, None).expect("a track"));
            let item = daw
                .add_item(
                    project.clone(),
                    track,
                    PositionInSeconds::from_seconds(0.0),
                    Duration::from_seconds(8.0),
                )
                .expect("an item to carry the take");
            let item = ItemRef::Guid(item);
            // A fresh item has no take; the markers live on one, so the
            // fixture makes the take the recorder would have made.
            let take = daw
                .add_take(project.clone(), item.clone())
                .expect("a take on the item");
            StretchTakeRef::new(project.clone(), item, TakeRef::Guid(take))
        })
        .collect();
    Kick {
        daw,
        project,
        takes,
    }
}

fn hits() -> Vec<Hit> {
    vec![Hit { at: 0.5 }, Hit { at: 1.0 }, Hit { at: 1.5 }]
}

/// Detection writes the same list to every mic of the piece.
///
/// r[verify flow.drums.editing.stack]
#[test]
fn every_mic_of_the_piece_carries_the_same_hits() -> Result {
    let kick = kick();
    let out = write(&kick.daw, &kick.project, &kick.takes, &hits())?;
    assert_eq!(out.mics, MICS.len(), "a mic missed the detection");

    let first = read(&kick.daw, &kick.takes[0])?;
    assert_eq!(first.len(), 3);
    for take in &kick.takes[1..] {
        assert_eq!(
            read(&kick.daw, take)?,
            first,
            "a mic carries different hits"
        );
    }
    Ok(())
}

/// **The rule.** Slipping a hit moves it on every mic at once, so the
/// piece stays in phase with itself.
///
/// r[verify flow.drums.editing.hands]
#[test]
fn slipping_a_hit_moves_it_on_every_mic() -> Result {
    let kick = kick();
    write(&kick.daw, &kick.project, &kick.takes, &hits())?;
    slip(&kick.daw, &kick.project, &kick.takes, 1, 1.2)?;

    for take in &kick.takes {
        let moved = read(&kick.daw, take)?;
        assert!(
            (moved[1].at - 1.2).abs() < 1e-9,
            "a mic kept the old position: {moved:?}"
        );
        assert!((moved[0].at - 0.5).abs() < 1e-9, "an untouched hit moved");
    }
    Ok(())
}

/// Re-detecting **replaces** the list rather than merging into it: a
/// detection describes the whole take, and merging would leave the hits
/// of a previous, worse detection behind with nothing to tell them
/// apart.
///
/// r[verify flow.drums.editing.stack]
#[test]
fn re_detecting_replaces_rather_than_merges() -> Result {
    let kick = kick();
    write(&kick.daw, &kick.project, &kick.takes, &hits())?;
    let fewer = vec![Hit { at: 2.0 }];
    write(&kick.daw, &kick.project, &kick.takes, &fewer)?;

    let after = read(&kick.daw, &kick.takes[0])?;
    assert_eq!(after.len(), 1, "the old detection survived: {after:?}");
    assert!((after[0].at - 2.0).abs() < 1e-9);
    Ok(())
}

/// A fresh detection maps each hit's own place in the source to its own
/// place in the take, so writing one does not warp the audio. Only a
/// slip should stretch anything.
#[test]
fn an_undisturbed_hit_does_not_warp_the_take() -> Result {
    let kick = kick();
    write(&kick.daw, &kick.project, &kick.takes, &hits())?;
    let markers = kick.daw.get_stretch_markers(
        kick.takes[0].project.clone(),
        kick.takes[0].item.clone(),
        kick.takes[0].take.clone(),
    );
    for marker in &markers {
        assert!(
            (marker.position - marker.source_position).abs() < 1e-9,
            "detection warped the take: {marker:?}"
        );
    }
    Ok(())
}
