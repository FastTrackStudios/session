//! Web client handler — receives pushed events from the desktop app.
//!
//! Implements `WebClientService` so the desktop gateway can call `push_event()`
//! via vox RPC. Events are applied to UI signals via the shared event bridge.

use session::{SetlistEvent, WebClientService};

#[derive(Clone)]
pub struct WebClientHandler;

impl WebClientService for WebClientHandler {
    async fn push_event(&self, event: SetlistEvent) {
        session_ui::apply_setlist_event(&event);
    }
}
