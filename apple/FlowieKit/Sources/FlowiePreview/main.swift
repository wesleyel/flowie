// FlowieUI in a Mac window, for trying the iOS app's views without an iOS build.
import SwiftUI
import FlowieUI

struct FlowiePreviewApp: App {
    @StateObject private var store = try! OutlineStore.open(
        databaseAt: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("FlowiePreview/outline.sqlite")
    )

    var body: some Scene {
        WindowGroup("Flowie") {
            FlowieRootView(store: store)
        }
    }
}

FlowiePreviewApp.main()
