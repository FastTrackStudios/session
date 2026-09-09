//! Event-bus stream source for `Standalone`.
//!
//! The cross-domain bus is an architect `#[subscribe]` stream served
//! from one process-wide `PubSub<DawEvent>` hub. Per-domain publish
//! helpers (`publish_track_events`, `publish_marker_event`, …) wrap
//! their events in [`DawEvent`] and publish here alongside their own
//! domain hub; transport state + position ticks are bridged in by the
//! per-project pump spawned in `Standalone::transport_engine_for`.
//! Subscribers receive everything and filter client-side (the old
//! `BusFilter` parameter moved into `daw_control::Events`).

use crate::Standalone;
use daw_proto::event_bus::{DawEvent, EventBus, EventBusStreamSource};

// The base `EventBus` trait is empty after the `#[subscribe]` port —
// only the stream sibling carries surface. The impl keeps `Standalone`
// eligible for any code that bounds on the trait.
impl EventBus for Standalone {}

impl EventBusStreamSource for Standalone {
    fn events_hub(&self) -> &architect::PubSub<DawEvent> {
        &self.bus_events
    }
}
