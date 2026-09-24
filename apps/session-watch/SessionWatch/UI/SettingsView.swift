// The click's settings, and how well it is keeping time.

import SwiftUI

struct SettingsView: View {
    @Environment(GuideStore.self) private var store

    var body: some View {
        @Bindable var store = store
        List {
            Section {
                Toggle("Haptic click", isOn: $store.clickOn)
                Toggle("Wrist down", isOn: $store.keepAwake)
                if store.keepAwake {
                    Text(wristStatus)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
            Section("Tap lead (ms early)") {
                Stepper("Beat \(Int(store.leadClickMs))", value: $store.leadClickMs, in: 0...150, step: 5)
                Stepper("Beat 1 \(Int(store.leadStrongMs))", value: $store.leadStrongMs, in: 0...150, step: 5)
                Button("Calibrate") { store.calibrate() }
                Text(calibrationStatus)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Section("Timing") {
                row("Timer p50", store.click.stats.percentileMs(0.5).map { String(format: "%.2f ms", $0) })
                row("Timer p95", store.click.stats.percentileMs(0.95).map { String(format: "%.2f ms", $0) })
                row("Clock ±", store.feed.map { String(format: "%.1f ms", $0.clockErrorUs / 1_000) })
                row("Phone", store.demo ? "demo" : store.phoneReachable ? "reachable" : "not reachable")
            }
            Section {
                Toggle("Demo song", isOn: $store.demo)
            }
        }
    }

    private func row(_ label: String, _ value: String?) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(value ?? "—").foregroundStyle(.secondary).monospacedDigit()
        }
        .font(.footnote)
    }

    private var wristStatus: String {
        if store.wrist.running { return "Running with the wrist down (up to 1 h)" }
        return store.wrist.lastProblem ?? "Starts when the click does"
    }

    private var calibrationStatus: String {
        switch store.calibration.state {
        case .idle: "Measures the tap's latency with the accelerometer"
        case .running(let done, let of): "Tapping… \(done)/\(of) — hold still"
        case .measured(let click, let start): String(format: "Measured %.0f / %.0f ms", click, start)
        case .failed(let why): why
        }
    }
}
