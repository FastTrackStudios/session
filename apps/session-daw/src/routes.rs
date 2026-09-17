//! What a track sends to, and what it receives from.
//!
//! The strip has always known WHETHER a track has sends — `send_count`
//! rides on the track and lights a lane in the corner. That is all a
//! mixer needs to draw itself, and it is deliberately all the track
//! stream carries: a bulk read of two hundred tracks must not also be a
//! bulk read of their routing.
//!
//! This is the other half, read only for the tracks something is
//! actually looking at. A route is a destination, a level, a pan, a
//! mute and a mode, and every one of those is a thing you change while
//! listening — so they arrive the way notes do: the window draws what
//! it has, asks for the rest, and redraws as the answers land.
//!
//! # Why a cache and not a fetch per frame
//!
//! Reading one track's sends is a round trip, and the panel is open
//! while you drag a send's level. Asking per frame would put the
//! network in the middle of a gesture. So the window holds the routes,
//! predicts its own edits into them, and lets the routing bus correct
//! it — the same shape the track list already uses, for the same
//! reason.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use daw_proto::routing::{RouteType, RoutingEvent, TrackRoute};

/// One track's routing, as the panel draws it.
///
/// No `PartialEq`: `TrackRoute` has none, and comparing two wirings is
/// not a question anything here asks — the cache is replaced by a read,
/// never diffed against one.
#[derive(Clone, Default, Debug)]
pub struct Wiring {
    /// Where this track sends, in REAPER's own order.
    pub sends: Vec<TrackRoute>,
    /// What sends to this track. Not editable from here: a receive is
    /// the far end of somebody else's send, and the thing that owns it
    /// is that send. Shown because "who is feeding this?" is the
    /// question you have when a track is louder than it should be.
    pub receives: Vec<TrackRoute>,
}

/// What a routing event did to the cache.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applied {
    /// A level, a pan, a mute. Redraw.
    Field,
    /// A route appeared or went away. The list itself is different, and
    /// so is the FAR track's — a send is a receive at the other end —
    /// so both have to be read again.
    Structure,
    /// Nothing this window is holding.
    Nothing,
}

/// The routes of the tracks that have been read so far.
///
/// Absent means "not asked yet"; a `Wiring` with two empty lists means
/// "read, and it has none". A track still loading must not draw as a
/// track with no sends — the panel would say something false and then
/// quietly correct itself, which is worse than saying nothing yet.
#[derive(Clone, Default)]
pub struct Routes {
    known: Arc<Mutex<HashMap<String, Wiring>>>,
    /// Set when something has landed, so the window redraws rather than
    /// comparing a map every frame.
    fresh: Arc<std::sync::atomic::AtomicBool>,
}

impl Routes {
    /// One track's routing, if it has been read.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<Wiring> {
        self.known.lock().ok()?.get(guid).cloned()
    }

    /// Has anything arrived since this was last asked? Asking clears it.
    pub fn take_fresh(&self) -> bool {
        self.fresh.swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    /// Forget a track's routing, so the next fetch reads it again.
    ///
    /// Forgetting rather than patching, for the structural changes: a
    /// send that appeared changed the numbering of nothing, but a send
    /// that went away renumbered every one after it at BOTH ends, and
    /// guessing at that is how a panel ends up editing the wrong send.
    pub fn forget(&self, guid: &str) {
        if let Ok(mut known) = self.known.lock() {
            known.remove(guid);
        }
    }

    /// Put one track's routing in, as read.
    pub fn put(&self, guid: &str, wiring: Wiring) {
        if let Ok(mut known) = self.known.lock() {
            known.insert(guid.to_owned(), wiring);
        }
        self.fresh.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Change one of a track's sends, before the engine confirms it.
    ///
    /// The prediction half of a drag: a fader that waits for a round
    /// trip is a fader that lags, and the correction is coming either
    /// way. `None` if that send is not held — nothing to predict into,
    /// and the read that lands will be right.
    pub fn predict(&self, guid: &str, index: u32, change: impl FnOnce(&mut TrackRoute)) {
        let Ok(mut known) = self.known.lock() else {
            return;
        };
        if let Some(route) = known
            .get_mut(guid)
            .and_then(|wiring| wiring.sends.iter_mut().find(|r| r.index == index))
        {
            change(route);
            self.fresh.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Apply one routing event from the bus.
    ///
    /// Field changes are written in place. Structural ones are reported
    /// rather than applied, because the caller is the one that can go
    /// and read both ends again.
    pub fn apply(&self, event: &RoutingEvent) -> Applied {
        let applied = {
            let Ok(mut known) = self.known.lock() else {
                return Applied::Nothing;
            };
            apply_to(&mut known, event)
        };
        if applied != Applied::Nothing {
            self.fresh.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        applied
    }

    /// Read the routing of every track in the list that is not held yet.
    ///
    /// Spawns and returns; the window keeps drawing.
    pub fn fetch(&self, wanted: Vec<String>) {
        let Some(runtime) = crate::open::runtime() else {
            return;
        };
        let missing: Vec<String> = {
            let Ok(known) = self.known.lock() else { return };
            wanted
                .into_iter()
                .filter(|guid| !known.contains_key(guid))
                .collect()
        };
        if missing.is_empty() {
            return;
        }
        let known = Arc::clone(&self.known);
        let fresh = Arc::clone(&self.fresh);
        std::thread::Builder::new()
            .name("session-daw-routes".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    let Ok(project) = daw.current_project().await else {
                        return;
                    };
                    for guid in missing {
                        let wiring = read(&project, &guid).await;
                        let Ok(mut known) = known.lock() else { return };
                        known.insert(guid, wiring);
                        fresh.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            })
            .ok();
    }
}

/// The cache half of [`Routes::apply`], with no lock and no thread.
///
/// Separated so what an event DOES is testable without a project: this
/// is the part that can be wrong, and the part that is wrong the day a
/// send is removed from under a numbering somebody was holding.
fn apply_to(known: &mut HashMap<String, Wiring>, event: &RoutingEvent) -> Applied {
    match event {
        // Both ends change, and neither is patchable from here: the
        // route arrives with a source but a receive is numbered at the
        // destination, which this event does not carry.
        RoutingEvent::RouteCreated {
            source_track_guid, ..
        }
        | RoutingEvent::RouteDeleted {
            source_track_guid, ..
        } => {
            // Structure either way, held or not: the far end of this
            // route is a track this event does not name, and only a
            // read can say which numbering changed.
            known.remove(source_track_guid);
            Applied::Structure
        }
        RoutingEvent::VolumeChanged {
            source_track_guid,
            route_type,
            route_index,
            volume,
            ..
        } => field(known, source_track_guid, *route_type, *route_index, |r| {
            r.volume = *volume;
        }),
        RoutingEvent::PanChanged {
            source_track_guid,
            route_type,
            route_index,
            pan,
            ..
        } => field(known, source_track_guid, *route_type, *route_index, |r| {
            r.pan = *pan;
        }),
        RoutingEvent::MuteChanged {
            source_track_guid,
            route_type,
            route_index,
            muted,
            ..
        } => field(known, source_track_guid, *route_type, *route_index, |r| {
            r.muted = *muted;
        }),
        // The track's own, not a route's — it rides on the track list
        // and the strip already draws it.
        RoutingEvent::ParentSendChanged { .. } => Applied::Nothing,
    }
}

/// Write one field of one route, if the window is holding it.
fn field(
    known: &mut HashMap<String, Wiring>,
    guid: &str,
    route_type: RouteType,
    index: u32,
    change: impl FnOnce(&mut TrackRoute),
) -> Applied {
    let Some(wiring) = known.get_mut(guid) else {
        return Applied::Nothing;
    };
    let list = match route_type {
        RouteType::Send => &mut wiring.sends,
        RouteType::Receive => &mut wiring.receives,
        // Hardware outputs are real routing, and nothing here draws
        // them yet. Reported as nothing rather than held, so the day
        // something does draw them it is a change to this line and not
        // a silent list that was being kept all along.
        RouteType::HardwareOutput => return Applied::Nothing,
    };
    match list.iter_mut().find(|r| r.index == index) {
        Some(route) => {
            change(route);
            Applied::Field
        }
        None => Applied::Nothing,
    }
}

/// Read one track's sends and receives.
///
/// An empty answer is a real answer: a track with no sends is the
/// common case, and holding that is what stops it being asked again.
async fn read(project: &daw_control::Project, guid: &str) -> Wiring {
    let Ok(Some(track)) = project.tracks().by_guid(guid).await else {
        return Wiring::default();
    };
    Wiring {
        sends: track.sends().all().await.unwrap_or_default(),
        receives: track.receives().all().await.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Applied, Routes, Wiring, apply_to};
    use daw_proto::routing::{RouteType, RoutingEvent, TrackRoute};
    use std::collections::HashMap;

    fn send(index: u32, to: &str) -> TrackRoute {
        TrackRoute {
            index,
            route_type: RouteType::Send,
            source_track_guid: "src".into(),
            dest_track_guid: Some(to.into()),
            dest_track_name: Some(to.to_uppercase()),
            ..TrackRoute::default()
        }
    }

    fn held() -> HashMap<String, Wiring> {
        let mut known = HashMap::new();
        known.insert(
            "src".to_owned(),
            Wiring {
                sends: vec![send(0, "verb"), send(1, "delay")],
                receives: Vec::new(),
            },
        );
        known
    }

    /// A level from the bus lands on the send it names.
    #[test]
    fn a_level_lands_on_its_own_send() {
        let mut known = held();
        let applied = apply_to(
            &mut known,
            &RoutingEvent::VolumeChanged {
                project_guid: String::new(),
                source_track_guid: "src".into(),
                route_type: RouteType::Send,
                route_index: 1,
                volume: 0.5,
            },
        );
        assert_eq!(applied, Applied::Field);
        let sends = &known["src"].sends;
        assert!((sends[1].volume - 0.5).abs() < 1e-9);
        assert!(
            (sends[0].volume - 1.0).abs() < 1e-9,
            "it moved the wrong send"
        );
    }

    /// A route going away is not patched: removing one renumbers every
    /// send after it, at both ends, and a panel editing send 2 by the
    /// number it remembers would edit somebody else's.
    #[test]
    fn a_removed_route_drops_the_list_rather_than_renumbering_it() {
        let mut known = held();
        let applied = apply_to(
            &mut known,
            &RoutingEvent::RouteDeleted {
                project_guid: String::new(),
                source_track_guid: "src".into(),
                route_type: RouteType::Send,
                route_index: 0,
            },
        );
        assert_eq!(applied, Applied::Structure);
        assert!(!known.contains_key("src"), "a stale numbering was kept");
    }

    /// An event for a track nobody is looking at changes nothing, and
    /// says so — a redraw per routing event in a big session would be a
    /// redraw per fader move on a track that is not on screen.
    #[test]
    fn a_field_for_an_unheld_track_is_nothing() {
        let mut known = held();
        let applied = apply_to(
            &mut known,
            &RoutingEvent::MuteChanged {
                project_guid: String::new(),
                source_track_guid: "somebody-else".into(),
                route_type: RouteType::Send,
                route_index: 0,
                muted: true,
            },
        );
        assert_eq!(applied, Applied::Nothing);
    }

    /// Not read yet and read-with-nothing are different answers.
    #[test]
    fn unread_is_not_the_same_as_unrouted() {
        let routes = Routes::default();
        assert!(routes.get("quiet").is_none());
        routes.put("quiet", Wiring::default());
        let read = routes.get("quiet").expect("read, and it has none");
        assert!(read.sends.is_empty() && read.receives.is_empty());
    }

    /// A prediction moves the send the gesture is on, and marks the
    /// cache fresh so the frame redraws without being told twice.
    #[test]
    fn a_prediction_moves_the_send_it_names() {
        let routes = Routes::default();
        routes.put(
            "src",
            Wiring {
                sends: vec![send(0, "verb"), send(1, "delay")],
                receives: Vec::new(),
            },
        );
        assert!(routes.take_fresh());
        routes.predict("src", 1, |route| route.volume = 0.25);
        assert!(routes.take_fresh(), "the frame was not told to redraw");
        let wiring = routes.get("src").expect("held");
        assert!((wiring.sends[1].volume - 0.25).abs() < 1e-9);
        assert!((wiring.sends[0].volume - 1.0).abs() < 1e-9);
    }

    /// Predicting into a track that is not held does nothing at all —
    /// including not marking a redraw, since nothing changed.
    #[test]
    fn a_prediction_into_nothing_is_nothing() {
        let routes = Routes::default();
        routes.predict("absent", 0, |route| route.volume = 0.25);
        assert!(!routes.take_fresh());
        assert!(routes.get("absent").is_none());
    }
}
