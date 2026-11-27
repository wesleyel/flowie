// Flowie for iOS: FlowieUI's views over the outline kept in Application Support.
import SwiftUI
import FlowieUI

@main
struct FlowieApp: App {
    @StateObject private var store = try! OutlineStore.open(
        databaseAt: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Flowie/flowie.sqlite")
    )
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            FlowieRootView(store: store)
        }
        .onChange(of: scenePhase) { _, phase in
            // iOS may end the app any time once it's in the background: save what is queued.
            if phase != .active {
                try? store.outline.flush()
            }
        }
    }
}
