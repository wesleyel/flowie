// Quick capture (wiki: iOS-Cross-Platform §2.1, §5): "Add to Today's Journal" for Shortcuts and
// Siri, and the home-screen quick action. Both run in the app, which alone writes the outline.
import AppIntents
import FlowieUI
import UIKit

/// Add text to the end of today's journal, one bullet a line, without opening the app (the app
/// is started in the background when it isn't running). Saved before it returns; a window
/// showing the day reads it.
struct AddToJournal: AppIntent {
    static let title: LocalizedStringResource = "Add to Today’s Journal"
    static let description = IntentDescription("Adds text to the end of today’s journal in Flowie, one bullet a line.")
    static let supportedModes: IntentModes = .background
    @available(iOS 27, *)
    static var allowedExecutionTargets: IntentExecutionTargets { .main }

    @Parameter(title: "Text", inputOptions: String.IntentInputOptions(multiline: true), requestValueDialog: "What do you want to add?")
    var text: String

    static var parameterSummary: some ParameterSummary {
        Summary("Add \(\.$text) to today’s journal")
    }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        guard try Launch.shared.openStore().appendToToday(text) else { throw CaptureFailure.empty }
        return .result(dialog: "Added to today’s journal.")
    }
}

struct CaptureShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AddToJournal(),
            phrases: ["Add to today’s journal in \(.applicationName)", "Add to my \(.applicationName) journal"],
            shortTitle: "Add to Today’s Journal", systemImageName: "text.badge.plus"
        )
        AppShortcut(
            intent: WriteInJournal(),
            phrases: ["Write in today’s journal in \(.applicationName)", "Open today’s journal in \(.applicationName)"],
            shortTitle: "Write in Today’s Journal", systemImageName: "square.and.pencil"
        )
    }
}

/// The home-screen quick action (`UIApplicationShortcutItems` in Project.swift): today's
/// journal, ready to type.
enum QuickAction {
    static let capture = "flowie.spokey.cn.capture"

    @MainActor static func perform(_ item: UIApplicationShortcutItem) -> Bool {
        guard item.type == capture, let store = try? Launch.shared.openStore() else { return false }
        do {
            try store.captureToday()
            return true
        } catch {
            store.problem = error.localizedDescription
            return false
        }
    }
}

/// SwiftUI apps hear about quick actions through a scene delegate.
final class CaptureAppDelegate: NSObject, UIApplicationDelegate {
    /// CKSyncEngine hears of changes in iCloud through silent pushes (its own subscription).
    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        application.registerForRemoteNotifications()
        return true
    }

    /// A push from iCloud, also while in the background: fetch what changed, and save it
    /// before iOS suspends the app again.
    func application(_ application: UIApplication, didReceiveRemoteNotification userInfo: [AnyHashable: Any]) async -> UIBackgroundFetchResult {
        guard let store = Launch.shared.store else { return .noData }
        await store.fetchFromICloud()
        try? store.flush()
        return .newData
    }

    func application(_ application: UIApplication, configurationForConnecting session: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: nil, sessionRole: session.role)
        config.delegateClass = CaptureSceneDelegate.self
        return config
    }
}

final class CaptureSceneDelegate: NSObject, UIWindowSceneDelegate {
    /// Launched from the quick action.
    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        if let item = options.shortcutItem {
            _ = MainActor.assumeIsolated { QuickAction.perform(item) }
        }
    }

    /// The quick action, with the app already running.
    func windowScene(_ scene: UIWindowScene, performActionFor item: UIApplicationShortcutItem,
                     completionHandler: @escaping (Bool) -> Void) {
        completionHandler(MainActor.assumeIsolated { QuickAction.perform(item) })
    }
}
