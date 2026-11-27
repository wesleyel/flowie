// Shared by the app and its Control Center control: the control only asks the system to run
// this intent, which runs in the app (wiki: iOS-Cross-Platform §5). The control's extension
// never opens the outline: one process writes the database.
import AppIntents

/// Open today's journal with the caret on an empty row and the keyboard up: the Control Center
/// control, the Action Button (Settings → Action Button → Controls), Shortcuts and Siri.
struct WriteInJournal: AppIntent {
    static let title: LocalizedStringResource = "Write in Today’s Journal"
    static let description = IntentDescription("Opens today’s journal in Flowie, ready to type.")
    static let supportedModes: IntentModes = .foreground(.immediate)
    @available(iOS 27, *)
    static var allowedExecutionTargets: IntentExecutionTargets { .main }

    @MainActor func perform() async throws -> some IntentResult {
        #if !FLOWIE_CONTROL
        try Launch.shared.openStore().captureToday()
        #endif
        return .result()
    }
}

/// Why a capture failed.
enum CaptureFailure: LocalizedError {
    /// The outline couldn't be opened (the app shows why).
    case unavailable
    /// There was no text to add.
    case empty

    var errorDescription: String? {
        switch self {
        case .unavailable: return String(localized: "Flowie can’t open your outline")
        case .empty: return String(localized: "There’s nothing to add.")
        }
    }
}
