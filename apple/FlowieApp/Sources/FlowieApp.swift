// Flowie for iOS: FlowieUI's views over the outline kept in the App Group container.
import SwiftUI
import StoreKit
import CoreSpotlight
import FlowieFFI
import FlowieInbox
import FlowieUI

@main
struct FlowieApp: App {
    @UIApplicationDelegateAdaptor(CaptureAppDelegate.self) private var appDelegate
    @StateObject private var launch = Launch.shared
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            if let store = launch.store {
                FlowieShell(store: store, showsSelfHosted: launch.testing)
                    // A page tapped in Spotlight's results.
                    .onContinueUserActivity(CSSearchableItemActionType) { activity in
                        if let id = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String, store.bullet(id) != nil {
                            store.show(store.pageRef(bullet: id))
                        }
                    }
            } else if let error = launch.error {
                OpenFailureView(error: error, database: launch.database)
            }
        }
        // Back, Forward and Zoom Out, with the desktop's keys.
        .commands { FlowieCommands() }
        .onChange(of: scenePhase, initial: true) { _, phase in
            // What the share extension left in the inbox goes into the journal at launch and
            // each time the app comes back.
            if phase == .active {
                launch.importShares()
            }
        }
        .onChange(of: scenePhase) { _, phase in
            // iOS may end the app any time once it's in the background (§5): typing ends first,
            // so text still being composed is kept, then everything queued is saved and no
            // write is left open.
            guard let store = launch.store, phase != .active else { return }
            if phase == .background {
                store.finishEditing()
            }
            do {
                try store.flush()
            } catch {
                store.problem = error.localizedDescription
            }
        }
    }
}

/// The outline opened at launch, or why it couldn't be. Opening never deletes, moves or
/// replaces a database it can't use (`store::open`): the app shows why and stops there.
@MainActor
final class Launch: ObservableObject {
    /// One for the app: its window and the App Intents, which run in the app (§5), share it.
    static let shared: Launch = {
        #if DEBUG && targetEnvironment(simulator)
        // UI tests get a local journal entry they can search for, with their own history.
        if let text = ProcessInfo.processInfo.environment["FLOWIE_UI_TEST_ENTRY"] {
            let suite = "FlowieNavigationTests"
            let defaults = UserDefaults(suiteName: suite)!
            defaults.removePersistentDomain(forName: suite)
            LaunchMemory.defaults = defaults
            OutlineStore.defaults = defaults
            let database = URL.temporaryDirectory.appending(path: "FlowieTests/\(UUID())/flowie.sqlite")
            let launch = Launch(database: database, startServices: false)
            _ = try? launch.store?.appendToToday(text)
            return launch
        }
        #endif
        return Launch(database: OutlineLocation.database())
    }()
    let database: URL
    let store: OutlineStore?
    let error: Error?
    /// The pages in Spotlight, kept current while the app runs.
    private var spotlight: SpotlightIndex?
    private var inbox: ShareInbox?
    /// A debug or TestFlight build: these keep the self-hosted sync settings, which App Store
    /// builds leave out (they sync only through iCloud; wiki: iOS-Cross-Platform §5).
    @Published private(set) var testing: Bool

    /// Add the shares the share extension left in its inbox to the journal (§5), saved before
    /// their files are deleted.
    func importShares() {
        guard let store, let inbox else { return }
        do {
            try store.importShares(from: inbox)
        } catch {
            store.problem = error.localizedDescription
        }
    }

    /// The outline, for the App Intents; they fail when it couldn't be opened.
    func openStore() throws -> OutlineStore {
        guard let store else { throw CaptureFailure.unavailable }
        return store
    }

    private init(database: URL, startServices: Bool = true) {
        self.database = database
        do {
            store = try OutlineStore.open(databaseAt: database)
            error = nil
        } catch {
            store = nil
            self.error = error
        }
        #if DEBUG
        testing = true
        #else
        testing = false
        #endif
        guard startServices else { return }
        inbox = ShareInbox.appGroup()
        // iCloud sync starts at launch, and runs while an account is signed in (wiki:
        // Per-Node-Sync §5.2). CKSyncEngine keeps its place beside the database.
        store?.startICloud(stateURL: database.deletingLastPathComponent().appendingPathComponent("icloud-sync-state.json"))
        if let store {
            spotlight = SpotlightIndex(store: store, stateURL: database.deletingLastPathComponent().appendingPathComponent("spotlight-index.json"))
        }
        // A share made while the app is on screen (beside the sharing app, on iPad) comes in at
        // once; otherwise it waits for the app to come back.
        CFNotificationCenterAddObserver(CFNotificationCenterGetDarwinNotifyCenter(), nil, { _, _, _, _, _ in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    if UIApplication.shared.applicationState == .active {
                        Launch.shared.importShares()
                    }
                }
            }
        }, ShareInbox.notification as CFString, nil, .deliverImmediately)
        // Siri's "Open <page> in Flowie" learns the starred pages' names.
        CaptureShortcuts.updateAppShortcutParameters()
        #if !DEBUG
        Task { @MainActor [weak self] in
            // TestFlight installs are signed for the sandbox environment.
            if case .verified(let transaction) = try? await AppTransaction.shared {
                self?.testing = transaction.environment == .sandbox
            }
        }
        #endif
    }
}

/// What the app shows instead of the outline when its database can't be opened.
struct OpenFailureView: View {
    let error: Error
    let database: URL

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Image(systemName: "exclamationmark.triangle")
                    .font(.largeTitle)
                    .foregroundStyle(.orange)
                Text("Flowie can’t open your outline")
                    .font(.title2.bold())
                Text(reason)
                Text("Flowie hasn’t deleted, moved or replaced your outline.")
                    .foregroundStyle(.secondary)
                if case .UpgradeFailed(let backup, _) = error as? FlowieError {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("The copy taken before the update")
                            .font(.footnote.bold())
                        Text(verbatim: backup)
                            .font(.footnote.monospaced())
                            .textSelection(.enabled)
                    }
                }
                VStack(alignment: .leading, spacing: 4) {
                    Text("Details")
                        .font(.footnote.bold())
                    Text(verbatim: "\(details)\n\(database.path)")
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                }
                .foregroundStyle(.secondary)
            }
            .padding(24)
            .frame(maxWidth: 560, alignment: .leading)
        }
    }

    /// The error as the Rust core words it.
    private var details: String {
        switch error as? FlowieError {
        case .Storage(let message), .BackupFailed(let message), .UpgradeFailed(_, let message):
            return message
        case .TooNew(let found, let current):
            return "the database has format \(found), from a newer Flowie (this one reads up to \(current))"
        default:
            return String(describing: error)
        }
    }

    private var reason: LocalizedStringKey {
        switch error as? FlowieError {
        case .TooNew:
            return "Your outline was saved by a newer version of Flowie. Update Flowie to open it."
        case .UpgradeFailed:
            return "Flowie couldn’t update your outline to this version’s format. Your outline is as it was, and a copy taken before the update is kept beside it."
        case .BackupFailed:
            return "Before updating your outline to this version’s format, Flowie saves a copy of it, and that failed. Nothing was changed. Make sure this device has free space, then open Flowie again."
        default:
            return "Flowie couldn’t read your outline’s database."
        }
    }
}
