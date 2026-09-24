// Session on the wrist: where the band is in the song, and the beat as a
// tap. Two pages — the guide, and the click's settings.

import SwiftUI

@main
struct SessionWatchApp: App {
    @State private var store = GuideStore()
    @Environment(\.scenePhase) private var phase
    // Initial page for screenshot automation: SIMCTL_CHILD_FTS_TAB=settings.
    @State private var tab: String = ProcessInfo.processInfo.environment["FTS_TAB"] ?? "guide"

    var body: some Scene {
        WindowGroup {
            TabView(selection: $tab) {
                GuideView().tag("guide")
                SettingsView().tag("settings")
            }
            .tabViewStyle(.verticalPage)
            .environment(store)
            .onChange(of: phase) { _, now in
                if now == .active { store.appBecameActive() }
            }
        }
    }
}
