// Keeping the click going with the wrist down.
//
// A mindfulness extended runtime session (WKBackgroundModes: mindfulness):
// the app stays FRONTMOST with the screen dimmed for up to an hour, so its
// main-queue timers, its haptics and its WatchConnectivity link all carry
// on — the same arrangement Apple's Breathe app uses to pace breathing with
// taps. Chosen over a workout session because a workout needs HealthKit
// authorisation, starts the heart-rate sensor, logs (or must discard) a
// workout, and is App Review's to question for a metronome; its one edge is
// no hour limit. When the hour is nearly up the system warns, and so does
// this (a notification tap); raising the wrist starts a fresh session.

import Foundation
import WatchKit

@MainActor
@Observable
final class WristDown: NSObject, WKExtendedRuntimeSessionDelegate {
    private(set) var running = false
    private(set) var lastProblem: String?
    private var session: WKExtendedRuntimeSession?
    private var wanted = false

    /// Keep running wrist-down from now (the app must be active to start).
    func hold() {
        wanted = true
        start()
    }

    func release() {
        wanted = false
        session?.invalidate()
        session = nil
        running = false
    }

    /// The app is frontmost again: renew a session that ran out.
    func appBecameActive() {
        if wanted { start() }
    }

    private func start() {
        // Only an active app may start one; the next activation will.
        guard session == nil, WKApplication.shared().applicationState == .active else { return }
        let s = WKExtendedRuntimeSession()
        s.delegate = self
        s.start()
        session = s
    }

    nonisolated func extendedRuntimeSessionDidStart(_ extendedRuntimeSession: WKExtendedRuntimeSession) {
        Task { @MainActor in
            self.running = true
            self.lastProblem = nil
        }
    }

    nonisolated func extendedRuntimeSessionWillExpire(_ extendedRuntimeSession: WKExtendedRuntimeSession) {
        Task { @MainActor in WKInterfaceDevice.current().play(.notification) }
    }

    nonisolated func extendedRuntimeSession(
        _ extendedRuntimeSession: WKExtendedRuntimeSession,
        didInvalidateWith reason: WKExtendedRuntimeSessionInvalidationReason, error: (any Error)?
    ) {
        let problem: String? =
            switch reason {
            case .none: nil
            case .expired: "The hour ran out — raise your wrist to renew"
            case .resignedFrontmost: "Another app came to the front"
            case .suppressedBySystem: "watchOS would not allow it now"
            case .sessionInProgress: nil
            case .error: error.map { "Failed (\(($0 as NSError).code)): \($0.localizedDescription)" } ?? "Failed"
            @unknown default: "Ended"
            }
        let ended = ObjectIdentifier(extendedRuntimeSession)
        Task { @MainActor in
            if let session = self.session, ObjectIdentifier(session) == ended { self.session = nil }
            self.running = false
            if let problem { self.lastProblem = problem }
        }
    }
}
