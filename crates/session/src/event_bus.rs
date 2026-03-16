//! Reusable event broadcasting abstractions.
//!
//! [`EventBus`] wraps `moire::sync::broadcast` for multi-consumer event streaming.
//! [`WatchBus`] wraps `moire::sync::watch` for single-latest-value streaming.

use moire::sync::{broadcast, watch};
use tokio::sync::watch as tokio_watch;

/// Multi-consumer event bus backed by `moire::sync::broadcast`.
///
/// Each call to [`subscribe`](EventBus::subscribe) creates an independent receiver
/// that will see all subsequent events emitted via [`emit`](EventBus::emit).
pub struct EventBus<T: Clone> {
    tx: broadcast::Sender<T>,
}

impl<T: Clone> EventBus<T> {
    /// Create a new event bus with the given name and channel capacity.
    pub fn new(name: &str, capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(name, capacity);
        Self { tx }
    }

    /// Broadcast a value to all current subscribers.
    ///
    /// If there are no active subscribers the value is silently dropped.
    pub fn emit(&self, value: T) {
        let _ = self.tx.send(value);
    }

    /// Create a new receiver that will observe future events.
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.tx.subscribe()
    }
}

/// Single-value streaming bus backed by `moire::sync::watch`.
///
/// Holds the latest value and lets any number of subscribers observe changes.
pub struct WatchBus<T> {
    tx: watch::Sender<T>,
    rx: watch::Receiver<T>,
}

impl<T: Clone> WatchBus<T> {
    /// Create a new watch bus with the given name and initial value.
    pub fn new(name: &str, initial: T) -> Self {
        let (tx, rx) = watch::channel(name, initial);
        Self { tx, rx }
    }

    /// Update the stored value, notifying all subscribers.
    pub fn send(&self, value: T) {
        let _ = self.tx.send(value);
    }

    /// Create a new receiver that will see the current and future values.
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.rx.clone()
    }

    /// Borrow the current value.
    pub fn borrow(&self) -> tokio_watch::Ref<'_, T> {
        self.rx.borrow()
    }
}
