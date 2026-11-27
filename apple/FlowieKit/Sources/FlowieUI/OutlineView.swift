import SwiftUI
import FlowieFFI

/// A view's own bit of state. (Kept in an object rather than `@State`, which the SDK makes a
/// macro whose plugin ships only with Xcode: this way the package builds with the plain Swift
/// toolchain too.)
final class Local<Value>: ObservableObject {
    @Published var value: Value

    init(_ value: Value) {
        self.value = value
    }
}

#if !canImport(UIKit)
/// A page of the outline in the Mac preview: a simple list of the rows the iOS page shows
/// (`OutlinePage`, wiki: iOS-Cross-Platform §4.1), each to tick off, edit, open as a page or
/// delete, and a field to add one at the end.
public struct OutlineView: View {
    @StateObject private var page: PageModel
    @ObservedObject private var store: OutlineStore
    @StateObject private var newText = Local("")

    public init(store: OutlineStore, kind: PageKind) {
        self.store = store
        _page = StateObject(wrappedValue: store.page(kind))
    }

    private var title: String {
        page.info?.title ?? NSLocalizedString("Home", comment: "Title of the outline's top page")
    }

    public var body: some View {
        List {
            ForEach(page.rows, id: \.id) { row in
                RowLine(page: page, row: row)
            }
            TextField("New bullet", text: $newText.value)
                .onSubmit {
                    let text = newText.value.trimmingCharacters(in: .whitespaces)
                    guard !text.isEmpty else { return }
                    page.add(text)
                    newText.value = ""
                }
        }
        .navigationTitle(title)
        .toolbar {
            ToolbarItemGroup {
                Button("Undo", systemImage: "arrow.uturn.backward") { page.undo() }
                    .keyboardShortcut("z", modifiers: .command)
                Button("Redo", systemImage: "arrow.uturn.forward") { page.redo() }
                    .keyboardShortcut("z", modifiers: [.command, .shift])
                SyncBadge(status: store.syncStatus)
            }
        }
        .safeAreaInset(edge: .bottom) {
            if let problem = store.problem {
                Text(problem).font(.footnote).foregroundStyle(.red).padding(8)
            }
        }
    }
}
#endif

#if !canImport(UIKit)
/// One bullet in the Mac preview's list.
struct RowLine: View {
    @ObservedObject var page: PageModel
    let row: RowView
    @StateObject private var text = Local("")
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Button {
                page.toggle(row)
            } label: {
                Image(systemName: row.completed ? "checkmark.circle.fill" : "circle")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(row.completed ? "Completed" : "Not completed")
            TextField("", text: $text.value, axis: .vertical)
                .strikethrough(row.completed)
                .foregroundStyle(row.completed ? .secondary : .primary)
                .focused($focused)
                .onSubmit { focused = false }
            if row.hasChildren {
                Button {
                    PreviewNavigation.open(row.id)
                } label: {
                    Image(systemName: "chevron.right")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Open as a page")
            }
        }
        .padding(.leading, CGFloat(row.depth) * 20)
        // Return ends editing.
        .onAppear { text.value = row.text }
        .onChange(of: row.text) { _, new in
            if !focused { text.value = new }
        }
        .onChange(of: text.value) { _, new in
            // Typing is saved as it goes, one undo step per bullet.
            if new.contains("\n") {
                text.value = new.replacingOccurrences(of: "\n", with: "")
                focused = false
            } else if focused {
                page.setText(row, new)
            }
        }
        .onChange(of: focused) { _, now in
            if now {
                page.editing(row)
            } else {
                page.setText(row, text.value)
                page.editing(nil)
            }
        }
        .swipeActions(edge: .leading) {
            Button("Indent", systemImage: "increase.indent") { page.indent(row) }
            Button("Outdent", systemImage: "decrease.indent") { page.outdent(row) }
        }
        .swipeActions {
            Button("Delete", systemImage: "trash", role: .destructive) { page.delete(row) }
        }
        .contextMenu {
            Button("Open as a Page", systemImage: "arrow.right.circle") { PreviewNavigation.open(row.id) }
            Button("Move Up", systemImage: "arrow.up") { page.moveUp(row) }
            Button("Move Down", systemImage: "arrow.down") { page.moveDown(row) }
            Button("Indent", systemImage: "increase.indent") { page.indent(row) }
            Button("Outdent", systemImage: "decrease.indent") { page.outdent(row) }
            Button("Delete", systemImage: "trash", role: .destructive) { page.delete(row) }
        }
    }
}
#endif

/// How sync is doing, as a small icon.
struct SyncBadge: View {
    let status: SyncStatus

    var body: some View {
        switch status {
        case .off:
            EmptyView()
        case .syncing:
            Image(systemName: "arrow.triangle.2.circlepath").accessibilityLabel("Syncing")
        case .synced:
            Image(systemName: "checkmark.icloud").accessibilityLabel("Up to date")
        case .offline(let error):
            Image(systemName: "icloud.slash").foregroundStyle(.red).help(error).accessibilityLabel("Offline: \(error)")
        }
    }
}
