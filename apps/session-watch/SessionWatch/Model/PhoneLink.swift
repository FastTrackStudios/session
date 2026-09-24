// The link to the Session iPhone app: WatchConnectivity.
//
// The phone does all the timing (session-watch-guide, in Rust — the code
// the desktop's guide runs on) and sends feeds whose every beat is already
// on this watch's clock. This side does two things: answer the phone's
// clock pings with a reading of MonoClock, stamped the moment the ping
// lands and again just before the answer goes; and hand feeds to the store.
//
// Why not talk to Task directly, like every other peer: watchOS allows an
// app a WebSocket only inside an audio-streaming session (TN3135), so vox's
// /vox WebSocket is out of reach. WatchConnectivity rides the Bluetooth
// link to the phone, which is already in the live set.

import Foundation
import WatchConnectivity

final class PhoneLink: NSObject, WCSessionDelegate, @unchecked Sendable {
    private let onFeed: @Sendable (WatchGuideFeed) -> Void
    private let onReachable: @Sendable (Bool) -> Void

    init(
        onFeed: @escaping @Sendable (WatchGuideFeed) -> Void,
        onReachable: @escaping @Sendable (Bool) -> Void
    ) {
        self.onFeed = onFeed
        self.onReachable = onReachable
    }

    func start() {
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    // ── Messages ──

    /// A ping (the phone wants an answer), or a feed sent with a reply
    /// handler. The first stamp is taken before anything else.
    func session(
        _ session: WCSession, didReceiveMessageData messageData: Data,
        replyHandler: @escaping (Data) -> Void
    ) {
        let received = MonoClock.nowMicros()
        let message = try? JSONDecoder().decode(WatchMessage.self, from: messageData)
        if let ping = message?.ping {
            let pong = WatchPong(sentUs: ping.sentUs, receivedUs: received, repliedUs: MonoClock.nowMicros())
            replyHandler((try? JSONEncoder().encode(pong)) ?? Data())
        } else {
            replyHandler(Data())
        }
        if let feed = message?.feed { onFeed(feed) }
    }

    /// A feed (no answer wanted).
    func session(_ session: WCSession, didReceiveMessageData messageData: Data) {
        deliver(messageData)
    }

    /// The latest feed, left for when the app was not running.
    func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
        if let data = applicationContext["feed"] as? Data { deliver(data) }
    }

    private func deliver(_ data: Data) {
        if let feed = (try? JSONDecoder().decode(WatchMessage.self, from: data))?.feed {
            onFeed(feed)
        }
    }

    // ── Session state ──

    func session(
        _ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState,
        error: (any Error)?
    ) {
        onReachable(session.isReachable)
        if let data = session.receivedApplicationContext["feed"] as? Data { deliver(data) }
    }

    func sessionReachabilityDidChange(_ session: WCSession) {
        onReachable(session.isReachable)
    }
}
