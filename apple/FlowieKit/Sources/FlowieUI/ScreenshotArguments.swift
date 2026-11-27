#if DEBUG && canImport(UIKit)
import Foundation

/// Debug builds only: the states of the App Store screenshots (scripts/ios-screenshots.py), set
/// by launch arguments. The page to open is the launch rule's (`LaunchMemory`), whose keys are
/// given as arguments too (`-launch.tab home -launch.path '(id1,id2)' …`); on top of that:
///
/// - `-FlowieSearch <query>`: the Search tab, searching for `query`.
/// - `-FlowieCapture <text>`: a quick capture in today's journal with `text` already typed, the
///   caret after it (so the keyboard is up).
enum ScreenshotArguments {
    @MainActor static func apply(to nav: Navigation, store: OutlineStore) {
        let arguments = UserDefaults.standard
        if let query = arguments.string(forKey: "FlowieSearch") {
            nav.showSearch(query)
        }
        if let text = arguments.string(forKey: "FlowieCapture") {
            try? store.captureToday(typed: text)
        }
    }
}
#endif
