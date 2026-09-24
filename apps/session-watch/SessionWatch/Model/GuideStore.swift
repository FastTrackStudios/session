// The app's state: the latest feed, the beat last sounded, and the click's
// settings. Feeds come from the phone (PhoneLink) or the Demo.

import Foundation
import SwiftUI
import WatchKit

@MainActor
@Observable
final class GuideStore {
    /// The latest feed, and when (MonoClock µs) it came.
    private(set) var feed: WatchGuideFeed?
    private(set) var feedAt: Double = 0
    /// The beat the face shows: the one last sounded, or where the song
    /// rests while stopped.
    private(set) var beat: WatchBeat?
    private(set) var phoneReachable = false

    let click = HapticClick()
    let wrist = WristDown()
    let calibration = Calibration()

    var clickOn: Bool {
        didSet {
            defaults.set(clickOn, forKey: "clickOn")
            reload()
        }
    }
    /// Keep the click going wrist-down (an extended runtime session).
    var keepAwake: Bool {
        didSet {
            defaults.set(keepAwake, forKey: "keepAwake")
            reload()
        }
    }
    /// ms early each pattern fires: the Taptic Engine's latency.
    var leadClickMs: Double {
        didSet {
            defaults.set(leadClickMs, forKey: "leadClickMs")
            applyLeads()
        }
    }
    var leadStrongMs: Double {
        didSet {
            defaults.set(leadStrongMs, forKey: "leadStrongMs")
            applyLeads()
        }
    }
    var demo: Bool {
        didSet { switchSource() }
    }

    private let defaults = UserDefaults.standard
    private var phone: PhoneLink?
    private var demoPlayer: DemoPlayer?

    init() {
        let env = ProcessInfo.processInfo.environment
        clickOn = defaults.object(forKey: "clickOn") as? Bool ?? true
        keepAwake = defaults.object(forKey: "keepAwake") as? Bool ?? true
        // 30 ms until calibrated: the order of what the Taptic Engine is
        // reported to take; Calibrate replaces it with this watch's own.
        leadClickMs = defaults.object(forKey: "leadClickMs") as? Double ?? 30
        leadStrongMs = defaults.object(forKey: "leadStrongMs") as? Double ?? 30
        demo = env["FTS_DEMO"] == "1"
        applyLeads()
        click.onBeat = { [weak self] beat in self?.sounded(beat) }
        let link = PhoneLink(
            onFeed: { [weak self] feed in
                Task { @MainActor in self?.fromPhone(feed) }
            },
            onReachable: { [weak self] up in
                Task { @MainActor in self?.phoneReachable = up }
            })
        link.start()
        phone = link
        switchSource()
    }

    /// Whether the feed has gone quiet (the phone out of reach, or not in a
    /// set): the face says so, and the click has already run out.
    var stale: Bool {
        feed == nil || MonoClock.nowMicros() - feedAt > 8_000_000
    }

    func appBecameActive() {
        wrist.appBecameActive()
    }

    func calibrate() {
        calibration.run { [weak self] click, strong in
            self?.leadClickMs = click.rounded()
            self?.leadStrongMs = strong.rounded()
        }
    }

    private func fromPhone(_ feed: WatchGuideFeed) {
        guard !demo else { return }
        apply(feed)
    }

    private func apply(_ feed: WatchGuideFeed) {
        let newRun = self.feed?.run != feed.run
        self.feed = feed
        feedAt = MonoClock.nowMicros()
        // Stopped, or a new run not yet sounding: show where the song is.
        if !feed.playing || newRun || beat == nil { beat = feed.here }
        click.load(feed, tap: clickOn)
        if clickOn, keepAwake, feed.playing { wrist.hold() }
    }

    private func sounded(_ beat: WatchBeat) {
        self.beat = beat
    }

    private func reload() {
        if let feed { click.load(feed, tap: clickOn) }
        if !(clickOn && keepAwake) { wrist.release() }
    }

    private func applyLeads() {
        click.leads = [.click: leadClickMs * 1_000, .start: leadStrongMs * 1_000]
        reload()
    }

    private func switchSource() {
        demoPlayer?.stop()
        demoPlayer = nil
        click.stop()
        feed = nil
        beat = nil
        if demo, let player = DemoPlayer() {
            demoPlayer = player
            player.start { [weak self] feed in self?.apply(feed) }
        }
    }
}
