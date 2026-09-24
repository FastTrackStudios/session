// The click on the wrist: every beat of the feed, fired at its instant on
// the watch's clock, early by the Taptic Engine's latency.

import Foundation
import WatchKit
import os

extension WatchAccent {
    /// What the beat feels like. watchOS has no Core Haptics — WKHapticType
    /// is the whole palette — and only two of its patterns are a single,
    /// short, clearly different tap: `.click` (light) and `.start` (firm).
    var haptic: WKHapticType {
        switch self {
        case .beat: .click
        case .downbeat, .countIn: .start
        }
    }
}

/// Schedules the feed's beats and fires them.
///
/// Every beat is armed on a strict (zero-leeway) main-queue timer — the
/// face only redraws once a beat, so the main queue is idle when one is
/// due. A beat far off gets a coarse wake 30 ms early and is re-armed from
/// a fresh clock reading then, so a doze in between cannot push it late.
/// A beat is tapped once per run (by its index), however many feeds
/// resend it; one that comes due more than 40 ms late is skipped rather
/// than tapped out of time.
@MainActor
final class HapticClick {
    /// µs early each pattern is fired (the Taptic Engine's latency).
    var leads: [WKHapticType: Double] = [.click: 30_000, .start: 30_000]
    let stats = TimingStats()
    /// Each beat as it sounds (whether or not it was tapped).
    var onBeat: ((WatchBeat) -> Void)?

    private var run: UInt64 = .max
    private var fired = Set<UInt32>()
    private var armed: [UInt32: (at: Double, timer: DispatchSourceTimer)] = [:]
    private var tapping = false
    private let log = Logger(subsystem: "app.fasttrackstudio.session.watch", category: "click")

    /// Take a new feed. `tap` = the click is on; the feed must also be
    /// clock-locked and playing for anything to be felt.
    func load(_ feed: WatchGuideFeed, tap: Bool) {
        if feed.run != run {
            cancelAll()
            fired.removeAll()
            run = feed.run
        }
        tapping = tap && feed.clockLocked && feed.playing
        let wanted = Set(feed.beats.map(\.index))
        for (index, entry) in armed where !wanted.contains(index) {
            entry.timer.cancel()
            armed[index] = nil
        }
        let now = MonoClock.nowMicros()
        for beat in feed.beats where !fired.contains(beat.index) {
            let at = beat.atUs - (tapping ? lead(beat.accent) : 0)
            if let existing = armed[beat.index], abs(existing.at - at) < 500 { continue }
            armed[beat.index]?.timer.cancel()
            armed[beat.index] = nil
            if at < now - 40_000 {
                fired.insert(beat.index)
                continue
            }
            arm(beat, at: at)
        }
    }

    func stop() {
        cancelAll()
        tapping = false
    }

    private func lead(_ accent: WatchAccent) -> Double {
        leads[accent.haptic] ?? 0
    }

    private func arm(_ beat: WatchBeat, at: Double) {
        let timer = DispatchSource.makeTimerSource(flags: .strict, queue: .main)
        let delay = at - MonoClock.nowMicros()
        if delay > 60_000 {
            timer.schedule(deadline: .now() + .microseconds(Int(delay - 30_000)), leeway: .milliseconds(5))
            timer.setEventHandler { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.armed[beat.index]?.timer === timer else { return }
                    self.arm(beat, at: at)
                }
            }
        } else {
            timer.schedule(deadline: .now() + .microseconds(max(0, Int(delay))), leeway: .nanoseconds(0))
            timer.setEventHandler { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.armed[beat.index]?.timer === timer else { return }
                    self.fire(beat, at: at)
                }
            }
        }
        armed[beat.index]?.timer.cancel()
        armed[beat.index] = (at, timer)
        timer.resume()
    }

    private func fire(_ beat: WatchBeat, at: Double) {
        let late = MonoClock.nowMicros() - at
        armed[beat.index]?.timer.cancel()
        armed[beat.index] = nil
        fired.insert(beat.index)
        if late < 40_000 {
            if tapping { WKInterfaceDevice.current().play(beat.accent.haptic) }
            stats.record(late)
            if stats.count % 32 == 0, let p50 = stats.percentileMs(0.5), let p95 = stats.percentileMs(0.95) {
                log.info("click timer lateness p50=\(p50, format: .fixed(precision: 2))ms p95=\(p95, format: .fixed(precision: 2))ms n=\(self.stats.count)")
            }
        }
        onBeat?(beat)
    }

    private func cancelAll() {
        for entry in armed.values { entry.timer.cancel() }
        armed.removeAll()
    }
}
