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
                    Button("Stop Syncing", role: .destructive) { store.stopSync() }
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

/// The app's root: the outline, with sync settings in a sheet.
public struct FlowieRootView: View {
    @ObservedObject var store: OutlineStore
    @StateObject private var settings = Local(false)

    public init(store: OutlineStore) {
        self.store = store
    }

    public var body: some View {
        NavigationStack {
            OutlineView(store: store)
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
