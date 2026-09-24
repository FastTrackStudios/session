// The Demo: a whole song's feed (demo-feed.json, built by the Rust
// timeline — `cargo run -p session-watch-guide --example demo_feed`),
// replayed on a loop with its beats shifted onto this watch's clock. No
// phone needed, so the face and the click can be tried — and the click's
// timers timed — anywhere, the simulator included.

import Foundation

@MainActor
final class DemoPlayer {
    private let song: WatchGuideFeed
    private let length: Double
    private let origin: Double
    private var timer: Timer?
    private static let lead = 500_000.0

    init?() {
        guard let url = Bundle.main.url(forResource: "demo-feed", withExtension: "json"),
            let data = try? Data(contentsOf: url),
            let song = try? JSONDecoder().decode(WatchGuideFeed.self, from: data),
            let last = song.beats.last
        else { return nil }
        self.song = song
        // A lap starts half a second before its first beat (so the new run
        // is in force before anything of it is due), and ends a beat after
        // its last.
        length = Self.lead + last.atUs + 60_000_000 / Double(max(last.bpm, 1))
        origin = MonoClock.nowMicros() + 1_500_000
    }

    func start(deliver: @escaping @MainActor (WatchGuideFeed) -> Void) {
        deliver(feed(at: MonoClock.nowMicros()))
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                deliver(self.feed(at: MonoClock.nowMicros()))
            }
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    /// The next six seconds of the loop, as a phone would send them.
    private func feed(at now: Double) -> WatchGuideFeed {
        let elapsed = max(0, now - origin)
        let lap = (elapsed / length).rounded(.down)
        var feed = song
        feed.run = UInt64(lap) + 1
        feed.revision = UInt64(elapsed / 250_000)
        let base = origin + lap * length + Self.lead
        feed.beats = song.beats
            .map { beat -> WatchBeat in
                var b = beat
                b.atUs += base
                return b
            }
            .filter { $0.atUs > now && $0.atUs <= now + 6_000_000 }
        let played = song.beats.last { base + $0.atUs <= now } ?? song.here
        feed.here = played
        feed.here.atUs = now
        return feed
    }
}
