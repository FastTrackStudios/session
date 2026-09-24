//! `WatchConnectivity`: the iPhone's link to the Session watch app.
//!
//! Why this link, and not the watch talking to Task itself: watchOS gives
//! an app a WebSocket (or any socket) only inside an audio-streaming
//! session (TN3135), so the watch cannot speak vox over Task's `/vox`
//! WebSocket the way every other peer does. `WatchConnectivity` rides the
//! Bluetooth link the watch always has to its phone — no Wi-Fi, no signal
//! at the venue needed — and the phone is already in the live set, already
//! following Task's clock, already holding the song's tempo map. So the
//! phone computes, and the watch taps.
//!
//! Messages are `sendMessageData` (JSON, see [`crate::wire`]): a ping's
//! answer comes back through its reply handler, stamped the moment it
//! lands. When the watch app is not running, the latest feed is left as
//! the application context instead, for it to show when it opens.

use std::ptr::NonNull;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_watch_connectivity::{WCSession, WCSessionActivationState, WCSessionDelegate};

use crate::driver::WatchLink;

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this adds no
    // Drop impl.
    #[unsafe(super(NSObject))]
    #[name = "FTSSessionWatchLinkDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WCSessionDelegate for Delegate {
        #[unsafe(method(session:activationDidCompleteWithState:error:))]
        fn activated(
            &self,
            _session: &WCSession,
            state: WCSessionActivationState,
            error: Option<&NSError>,
        ) {
            if let Some(error) = error {
                tracing::warn!(
                    watch.activation_state = state.0,
                    watch.error = %error.localizedDescription(),
                    "watch link: WatchConnectivity did not activate"
                );
            }
        }

        #[unsafe(method(sessionDidBecomeInactive:))]
        fn inactive(&self, _session: &WCSession) {}

        /// The paired watch changed: start again with the new one.
        #[unsafe(method(sessionDidDeactivate:))]
        fn deactivated(&self, session: &WCSession) {
            // SAFETY: activating the default session from its own delegate
            // callback is the documented way to switch watches.
            unsafe { session.activateSession() };
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// The phone's `WatchConnectivity` session, as a [`WatchLink`].
pub struct WcLink {
    session: Retained<WCSession>,
    /// `WCSession` keeps its delegate weakly.
    _delegate: Retained<Delegate>,
}

// SAFETY: WCSession is documented thread-safe (its delegate is called on a
// background queue, and sends may come from any thread); the delegate holds
// no state.
#[allow(clippy::non_send_fields_in_send_ty)] // the Retained<WCSession> is the point
unsafe impl Send for WcLink {}
// SAFETY: as above.
unsafe impl Sync for WcLink {}

impl WcLink {
    /// Activate `WatchConnectivity`; `None` on a device that has none (an
    /// iPad).
    #[must_use]
    pub fn activate() -> Option<Self> {
        // SAFETY: plain class/instance queries on WCSession.
        unsafe {
            if !WCSession::isSupported() {
                return None;
            }
            let session = WCSession::defaultSession();
            let delegate = Delegate::new();
            session.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            session.activateSession();
            Some(Self {
                session,
                _delegate: delegate,
            })
        }
    }
}

impl WatchLink for WcLink {
    fn reachable(&self) -> bool {
        // SAFETY: a property read.
        unsafe {
            self.session.activationState() == WCSessionActivationState::Activated
                && self.session.isReachable()
        }
    }

    fn send(&self, bytes: Vec<u8>, reply: Option<Box<dyn FnOnce(Vec<u8>) + Send>>) {
        let data = NSData::with_bytes(&bytes);
        let reply = reply.map(|once| {
            let once = Mutex::new(Some(once));
            RcBlock::new(move |answer: NonNull<NSData>| {
                // SAFETY: WatchConnectivity hands the reply block a valid
                // NSData for the duration of the call.
                let answer = unsafe { answer.as_ref() }.to_vec();
                if let Some(once) = once.lock().ok().and_then(|mut o| o.take()) {
                    once(answer);
                }
            })
        });
        // A failed send is a dropped ping or a feed the next one replaces:
        // the error handler has nothing to do.
        let failed = RcBlock::new(|_: NonNull<NSError>| {});
        // SAFETY: both blocks are 'static and only read what they captured.
        unsafe {
            self.session.sendMessageData_replyHandler_errorHandler(
                &data,
                reply.as_deref(),
                Some(&failed),
            );
        }
    }

    fn remember(&self, bytes: Vec<u8>) {
        let data = NSData::with_bytes(&bytes);
        let key = NSString::from_str("feed");
        let value: &AnyObject = &data;
        let context = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[value]);
        // SAFETY: the dictionary holds property-list types only (NSData).
        if let Err(e) = unsafe { self.session.updateApplicationContext_error(&context) } {
            tracing::debug!(watch.error = %e.localizedDescription(), "watch link: application context not updated");
        }
    }
}
