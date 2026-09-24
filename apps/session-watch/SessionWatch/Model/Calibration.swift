// How long the Taptic Engine takes to move: measured, not guessed.
//
// The watch feels its own taps. The accelerometer (100 Hz) sits in the
// same case as the Taptic Engine, so a tap is a spike in its readings; the
// time from `play` to the first sample that leaves the resting noise is
// the latency to fire early by. Ten taps of each pattern, the median of
// each — to about ±5 ms (the sample period's half), which is also the
// honest resolution of the result. The watch must be on a wrist or a table
// and still while it runs.

import CoreMotion
import Foundation
import WatchKit

@MainActor
@Observable
final class Calibration {
    enum State: Equatable {
        case idle
        case running(done: Int, of: Int)
        case measured(clickMs: Double, startMs: Double)
        case failed(String)
    }

    private(set) var state: State = .idle
    private let motion = CMMotionManager()

    /// Measure, then hand the two latencies (ms) to `apply`.
    func run(apply: @escaping @MainActor (Double, Double) -> Void) {
        guard motion.isAccelerometerAvailable else {
            state = .failed("No accelerometer here (the simulator has none)")
            return
        }
        let samples = SampleLog()
        motion.accelerometerUpdateInterval = 1.0 / 100.0
        motion.startAccelerometerUpdates(to: OperationQueue()) { data, _ in
            guard let a = data?.acceleration, let t = data?.timestamp else { return }
            samples.append(t, (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot())
        }
        let taps: [WKHapticType] = Array(repeating: .click, count: 10) + Array(repeating: .start, count: 10)
        Task {
            // Let the sensor settle.
            try? await Task.sleep(for: .milliseconds(600))
            var calls: [(WKHapticType, TimeInterval)] = []
            for (i, type) in taps.enumerated() {
                state = .running(done: i, of: taps.count)
                // Same time base as CMLogItem.timestamp: seconds since boot.
                calls.append((type, ProcessInfo.processInfo.systemUptime))
                WKInterfaceDevice.current().play(type)
                try? await Task.sleep(for: .milliseconds(700))
            }
            motion.stopAccelerometerUpdates()
            let log = samples.snapshot()
            let latency = { (type: WKHapticType) -> Double? in
                let found = calls.filter { $0.0 == type }.compactMap { Self.onset(after: $0.1, in: log) }
                guard found.count >= 5 else { return nil }
                return found.sorted()[found.count / 2] * 1_000
            }
            if let click = latency(.click), let start = latency(.start) {
                state = .measured(clickMs: click, startMs: start)
                apply(click, start)
            } else {
                state = .failed("Taps not found in the motion data — hold still and retry")
            }
        }
    }

    /// Seconds from `call` to the first sample clearly off the resting
    /// level (the 200 ms before it), or nil.
    nonisolated static func onset(after call: TimeInterval, in log: [(TimeInterval, Double)]) -> Double? {
        let rest = log.filter { $0.0 > call - 0.25 && $0.0 < call - 0.02 }.map(\.1)
        guard rest.count >= 10 else { return nil }
        let mean = rest.reduce(0, +) / Double(rest.count)
        let spread = (rest.map { ($0 - mean) * ($0 - mean) }.reduce(0, +) / Double(rest.count)).squareRoot()
        let threshold = max(5 * spread, 0.01)
        // The sample is stamped at the END of its 10 ms: the movement began,
        // on average, half a period earlier.
        guard let hit = log.first(where: { $0.0 > call && $0.0 < call + 0.4 && abs($0.1 - mean) > threshold })
        else { return nil }
        return max(0, hit.0 - call - 0.005)
    }
}

/// Accelerometer samples, appended from CoreMotion's queue.
private final class SampleLog: @unchecked Sendable {
    private let lock = NSLock()
    private var samples: [(TimeInterval, Double)] = []

    func append(_ t: TimeInterval, _ magnitude: Double) {
        lock.lock()
        samples.append((t, magnitude))
        lock.unlock()
    }

    func snapshot() -> [(TimeInterval, Double)] {
        lock.lock()
        defer { lock.unlock() }
        return samples
    }
}
