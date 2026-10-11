//! Leveling the kit: each drum's strokes brought toward one another, as
//! a volume curve on the recordings themselves.
//!
//! An engineer evens a kick out before mixing it — clip gain stroke by
//! stroke, or a compressor — so the compressor later shapes a steady
//! drum rather than chasing an uneven one. Here it is the leveler's drum
//! ride ([`level_dsp::drums`]): one gain a stroke, read off its hit
//! marker's level, set a few milliseconds before the attack and held
//! through the ring. A ghost note, bleed or a flam's grace is no stroke
//! and rides under the stroke before it.
//!
//! The curve goes on each of the drum's close mics as its take's volume
//! envelope — the same gain on every mic, so they stay in balance — on
//! the comp the edit is cut from. It is done before the edit: a take's
//! envelope rides with its audio through every split, slip and move, so
//! an edit made after carries the leveling with it, and the waveforms
//! drawn are the leveled ones.

#![cfg(feature = "native")]

use daw::service::{Items as _, ProjectContext, TakeRef, Takes as _, TrackRef, Tracks};
use daw_proto::automation::{AddPointParams, Automation as _, EnvelopeShape};
use daw_proto::{EnvelopeLocation, EnvelopeRef, ItemRef, TakeEnvelopeKind};
use facet::Facet;

/// What leveling a drum did, or would.
#[derive(Facet, Clone, Debug, Default)]
pub struct Leveled {
    pub piece: String,
    /// Its strokes, leveled.
    pub strokes: u32,
    /// Their median level, dBFS — each stroke is brought toward the
    /// median of the strokes around it.
    pub target_db: Option<f64>,
    /// How many were raised and lowered, and the most either way, dB.
    pub raised: u32,
    pub lowered: u32,
    pub most_db: f64,
    /// The takes whose volume curve was written.
    pub takes: u32,
}

/// One drum's hits on a take, as the leveler reads them: where in the
/// take (seconds from its start), how loud, a stroke or not.
fn hits_on(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    item: &daw_proto::Item,
    piece: &str,
) -> Vec<level_dsp::drums::Hit> {
    let markers = daw.get_take_markers(
        project.clone(),
        ItemRef::Guid(item.guid.clone()),
        TakeRef::Active,
    );
    let from = item.start_offset.as_seconds();
    let to = from + item.length.as_seconds();
    let peaks: std::collections::HashMap<&str, (f64, f64)> = markers
        .iter()
        .filter_map(|m| {
            let h = m.hit.as_ref()?;
            Some((m.id.as_str(), (h.peak_db, m.source_position_seconds)))
        })
        .collect();
    let mut hits: Vec<level_dsp::drums::Hit> = markers
        .iter()
        .filter(|m| m.source_position_seconds >= from && m.source_position_seconds < to)
        .filter_map(|m| {
            let h = m.hit.as_ref().filter(|h| h.piece == piece)?;
            // A flam's grace — quieter, and before or after its stroke —
            // rides with the stroke.
            let grace = h
                .flam_with
                .as_deref()
                .and_then(|other| peaks.get(other))
                .is_some_and(|(peak, _)| *peak > h.peak_db);
            Some(level_dsp::drums::Hit {
                t_s: m.source_position_seconds - from,
                level_db: h.peak_db,
                stroke: !(h.ghost || h.bleed || grace),
            })
        })
        .collect();
    hits.sort_by(|a, b| a.t_s.total_cmp(&b.t_s));
    hits
}

/// The ride for a drum by its name.
fn ride_for(piece: &str, amount: Option<f64>) -> level_dsp::drums::DrumRide {
    let name = piece.to_lowercase();
    let ride = if name.contains("kick") {
        level_dsp::drums::DrumRide::KICK
    } else if name.contains("snare") {
        level_dsp::drums::DrumRide::SNARE
    } else {
        level_dsp::drums::DrumRide::TOM
    };
    amount.map_or(ride, |amount| level_dsp::drums::DrumRide { amount, ..ride })
}

/// The items a drum's mic plays its comp from: its COMP lane's, once the
/// kit is set up to be edited — else every item it has.
fn comp_items(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: &str,
) -> Vec<daw_proto::Item> {
    let Some(t) = Tracks::get(daw, project.clone(), TrackRef::Guid(track.to_owned())) else {
        return Vec::new();
    };
    let comp = crate::drum_lanes::lane_named(&t, crate::drum_lanes::COMP);
    daw.get_items(project.clone(), TrackRef::Guid(track.to_owned()))
        .into_iter()
        .filter(|i| comp.is_none_or(|lane| i.fixed_lane.unwrap_or(0) == lane))
        .collect()
}

/// Level `pieces` (every drum of the kit when empty): see the module.
/// `amount` overrides each drum's own; `dry` only says.
///
/// # Errors
///
/// No hit markers to level by.
pub fn level(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    pieces: &[String],
    (amount, dry): (Option<f64>, bool),
) -> Result<Vec<Leveled>, String> {
    let (kit, _) = crate::hits::kit(daw, project);
    if kit.is_empty() {
        return Err("no hit markers: find the hits first (HITS_DETECT)".to_owned());
    }
    let label = "Level the drums";
    if !dry {
        daw::service::Projects::begin_undo_block(daw, project.clone(), label);
    }
    let mut out = Vec::new();
    for piece in kit
        .iter()
        .filter(|p| pieces.is_empty() || pieces.contains(&p.name))
    {
        let ride = ride_for(&piece.name, amount);
        // The target from the drum's first mic: one target, so every mic
        // takes the same gain at each stroke.
        let Some(first) = piece.mics.first() else {
            continue;
        };
        // Every hit on the song's timeline: what each stroke is held up to
        // is the playing around it there.
        let mut all: Vec<level_dsp::drums::Hit> = comp_items(daw, project, first)
            .iter()
            .flat_map(|item| {
                let at = item.position.as_seconds();
                hits_on(daw, project, item, &piece.name)
                    .into_iter()
                    .map(move |h| level_dsp::drums::Hit {
                        t_s: h.t_s + at,
                        ..h
                    })
            })
            .collect();
        all.sort_by(|a, b| a.t_s.total_cmp(&b.t_s));
        let Some(target) = level_dsp::drums::target(&all) else {
            continue;
        };
        let around = |t: f64| level_dsp::drums::target_at(&all, t).unwrap_or(target);
        let mut said = Leveled {
            piece: piece.name.clone(),
            target_db: Some(target),
            ..Leveled::default()
        };
        for hit in all.iter().filter(|h| h.stroke) {
            let g = level_dsp::drums::gain(hit.level_db, around(hit.t_s), &ride);
            said.strokes = said.strokes.saturating_add(1);
            if g > ride.least_db {
                said.raised = said.raised.saturating_add(1);
            } else if g < -ride.least_db {
                said.lowered = said.lowered.saturating_add(1);
            }
            said.most_db = if g.abs() > said.most_db.abs() {
                g
            } else {
                said.most_db
            };
        }
        if !dry {
            for mic in &piece.mics {
                for item in comp_items(daw, project, mic) {
                    let at = item.position.as_seconds();
                    let curve = level_dsp::drums::ride(
                        &hits_on(daw, project, &item, &piece.name),
                        |t| around(t + at),
                        &ride,
                    );
                    if write_curve(daw, project, &item, &curve) {
                        said.takes = said.takes.saturating_add(1);
                        crate::midi::mark_stale([item.guid.clone()]);
                    }
                }
            }
        }
        out.push(said);
    }
    if !dry {
        daw::service::Projects::end_undo_block(daw, project.clone(), label, None);
        crate::studio::request_resync();
    }
    Ok(out)
}

/// Write `curve` (dB, seconds from the item's start) as the item's take
/// volume envelope, in place of what was there — derived, so never merged
/// with an earlier pass. False with no take.
fn write_curve(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    item: &daw_proto::Item,
    curve: &[level_dsp::envelope::EnvPoint],
) -> bool {
    let Some(take) = daw.get_active_take(project.clone(), ItemRef::Guid(item.guid.clone())) else {
        return false;
    };
    let location = EnvelopeLocation {
        // Ignored for a take's envelope: the item and take say where.
        track: TrackRef::Index(0),
        envelope: EnvelopeRef::Take {
            item_guid: item.guid.clone(),
            take_guid: take.guid,
            kind: TakeEnvelopeKind::Volume,
        },
    };
    let had = daw.points(project.clone(), location.clone()).len();
    for i in (0..had).rev() {
        daw.delete_point(
            project.clone(),
            location.clone(),
            u32::try_from(i).unwrap_or(u32::MAX),
        );
    }
    for p in curve {
        daw.add_point(
            project.clone(),
            location.clone(),
            AddPointParams::new(
                daw_proto::PositionInSeconds::from_seconds(p.t_s),
                10f64.powf(p.db / 20.0),
                EnvelopeShape::Linear,
            ),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::ride_for;

    #[test]
    fn each_drum_has_its_own_ride() {
        assert_eq!(ride_for("Kick", None), level_dsp::drums::DrumRide::KICK);
        assert_eq!(ride_for("Snare", None), level_dsp::drums::DrumRide::SNARE);
        assert_eq!(ride_for("Tom 3", None), level_dsp::drums::DrumRide::TOM);
        assert!((ride_for("Kick", Some(0.3)).amount - 0.3).abs() < 1e-12);
    }
}
