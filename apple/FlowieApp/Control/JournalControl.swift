// Flowie's Control Center control (also offered for the Action Button and the Lock Screen).
// It only asks the system to run `WriteInJournal`, which runs in the app: this extension never
// opens the outline (wiki: iOS-Cross-Platform §5).
import AppIntents
import SwiftUI
import WidgetKit

@main
struct JournalControls: WidgetBundle {
    var body: some Widget { JournalControl() }
}

struct JournalControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "flowie.spokey.cn.capture") {
            ControlWidgetButton(action: WriteInJournal()) {
                Label("Write in Today’s Journal", systemImage: "square.and.pencil")
            }
        }
        .displayName("Write in Today’s Journal")
        .description("Opens today’s journal in Flowie, ready to type.")
    }
}
