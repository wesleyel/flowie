import SwiftUI
import FlowieFFI

/// Sync with a self-hosted server: its address and this device's token (docs/sync.md).
public struct SyncSettingsView: View {
    @ObservedObject var store: OutlineStore
    @AppStorage("sync.server") private var server = ""
    @AppStorage("sync.token") private var token = ""

    public init(store: OutlineStore) {
        self.store = store
    }

    public var body: some View {
        Form {
            Section {
                TextField("Server", text: $server, prompt: Text("https://sync.example.com"))
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                SecureField("Device token", text: $token)
            } footer: {
                Text("Issue a token for this device on the server with flowie-server token add NAME.")
            }
            Section {
                if store.syncStatus == .off {
                    Button("Sync") { store.startSync(server: server, token: token) }
                        .disabled(server.isEmpty || token.isEmpty)
                } else {
                    Button("Stop Syncing", role: .destructive) {
                        store.stopSync()
                        store.resumeICloud()
                    }
                }
                Text(statusText)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var statusText: String {
        switch store.syncStatus {
        case .off: return "Sync is off."
        case .syncing: return "Syncing…"
        case .synced: return "Up to date."
        case .offline(let error): return "Can’t sync: \(error). Edits are kept here and sent later."
        }
    }
}

#if !canImport(UIKit)
/// Where the Mac preview's navigation stack is: bullets opened as pages.
@MainActor
final class PreviewNavigation: ObservableObject {
    static let shared = PreviewNavigation()
    @Published var path: [String] = []

    static func open(_ id: String) {
        shared.path.append(id)
    }
}

/// The Mac preview's root: the outline, a bullet's page pushed when opened, with sync settings
/// in a sheet. (The iOS app's shell is `FlowieShell`.)
public struct FlowieRootView: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject private var nav = PreviewNavigation.shared
    @StateObject private var settings = Local(false)

    public init(store: OutlineStore) {
        self.store = store
    }

    public var body: some View {
        NavigationStack(path: $nav.path) {
            OutlineView(store: store, kind: .home)
                .navigationDestination(for: String.self) { id in
                    OutlineView(store: store, kind: .bullet(id))
                }
                .toolbar {
                    ToolbarItem {
                        Button("Sync Settings", systemImage: "gearshape") { settings.value = true }
                    }
                }
        }
        .sheet(isPresented: $settings.value) {
            NavigationStack {
                SyncSettingsView(store: store)
                    .navigationTitle("Sync")
                    .toolbar {
                        Button("Done") { settings.value = false }
                    }
            }
        }
    }
}
#endif
