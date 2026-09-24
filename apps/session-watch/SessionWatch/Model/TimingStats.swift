// How late the click's timers actually fire — the part of the timing the
// watch can measure about itself.

import Foundation

/// A rolling window of timer lateness (µs: when a tap fired minus when it
/// was due). What a tap sounds like on the skin adds the Taptic Engine's own
/// latency on top; Calibrate measures that, and the lead covers it.
@MainActor
@Observable
final class TimingStats {
    private var window: [Double] = []
    private let cap = 256
    private(set) var count = 0

    func record(_ latenessMicros: Double) {
        if window.count == cap { window.removeFirst() }
        window.append(latenessMicros)
        count += 1
    }

    func reset() {
        window.removeAll()
        count = 0
    }

    /// Percentile of the absolute lateness, ms (nil before any taps).
    func percentileMs(_ p: Double) -> Double? {
        guard !window.isEmpty else { return nil }
        let sorted = window.map(abs).sorted()
        let i = min(sorted.count - 1, max(0, Int((Double(sorted.count - 1) * p).rounded())))
        return sorted[i] / 1_000
    }

    /// Mean signed lateness, ms: a steady bias the lead could absorb.
    var meanMs: Double? {
        guard !window.isEmpty else { return nil }
        return window.reduce(0, +) / Double(window.count) / 1_000
    }
}
