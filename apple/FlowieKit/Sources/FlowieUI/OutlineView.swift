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

/// A page of the outline. On iOS, the UIKit page (`OutlinePage`, wiki: iOS-Cross-Platform §4.1);
/// on the Mac preview, a simple list of the same rows: each to tick off, edit, open as a page or
/// delete, and a field to add one at the end.
public struct OutlineView: View {
    @ObservedObject var store: OutlineStore
    @StateObject private var newText = Local("")

    public init(store: OutlineStore) {
        self.store = store
    }

    private var title: String {
        store.pages.last.map { $0.text.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : $0.text }
            ?? NSLocalizedString("Home", comment: "Title of the outline's top page")
    }

    public var body: some View {
        #if canImport(UIKit)
        OutlinePage(store: store)
            // The page keeps its rows above the keyboard itself (keyboardLayoutGuide), and
            // its list scrolls under the home indicator.
            .ignoresSafeArea(.keyboard)
            .ignoresSafeArea(.container, edges: .bottom)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if !store.pages.isEmpty {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Back", systemImage: "chevron.left") { store.back() }
                    }
                }
                ToolbarItem {
                    SyncBadge(status: store.syncStatus)
                }
            }
            .safeAreaInset(edge: .top) {
                if let problem = store.problem {
                    Text(problem).font(.footnote).foregroundStyle(.red).padding(8)
                }
            }
        #else
        List {
            ForEach(store.rows, id: \.id) { row in
                RowLine(store: store, row: row)
            }
            TextField("New bullet", text: $newText.value)
                .onSubmit {
                    let text = newText.value.trimmingCharacters(in: .whitespaces)
                    guard !text.isEmpty else { return }
                    store.add(text)
                    newText.value = ""
                }
        }
        .navigationTitle(title)
        .toolbar {
            ToolbarItemGroup {
                if !store.pages.isEmpty {
                    Button("Back", systemImage: "chevron.left") { store.back() }
                }
                Button("Undo", systemImage: "arrow.uturn.backward") { store.undo() }
                    .keyboardShortcut("z", modifiers: .command)
                Button("Redo", systemImage: "arrow.uturn.forward") { store.redo() }
                    .keyboardShortcut("z", modifiers: [.command, .shift])
                SyncBadge(status: store.syncStatus)
            }
        }
        .safeAreaInset(edge: .bottom) {
            if let problem = store.problem {
                Text(problem).font(.footnote).foregroundStyle(.red).padding(8)
            }
        }
        #endif
    }
}

#if !canImport(UIKit)
/// One bullet in the Mac preview's list.
struct RowLine: View {
    @ObservedObject var store: OutlineStore
    let row: RowView
    @StateObject private var text = Local("")
    @FocusState private var focused: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Button {
                store.toggle(row)
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
                    store.open(row)
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
                store.setText(row, new)
            }
        }
        .onChange(of: focused) { _, now in
            if now {
                store.editing(row)
            } else {
                store.setText(row, text.value)
                store.editing(nil)
            }
        }
        .swipeActions(edge: .leading) {
            Button("Indent", systemImage: "increase.indent") { store.indent(row) }
            Button("Outdent", systemImage: "decrease.indent") { store.outdent(row) }
        }
        .swipeActions {
            Button("Delete", systemImage: "trash", role: .destructive) { store.delete(row) }
        }
        .contextMenu {
            Button("Open as a Page", systemImage: "arrow.right.circle") { store.open(row) }
            Button("Move Up", systemImage: "arrow.up") { store.moveUp(row) }
            Button("Move Down", systemImage: "arrow.down") { store.moveDown(row) }
            Button("Indent", systemImage: "increase.indent") { store.indent(row) }
            Button("Outdent", systemImage: "decrease.indent") { store.outdent(row) }
            Button("Delete", systemImage: "trash", role: .destructive) { store.delete(row) }
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
