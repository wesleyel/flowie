import Foundation
import FlowieFFI

/// The outline as the views see it: the page shown and its rows, read again after each edit and
/// each time sync brings changes. Every read and write goes through flowie-ffi; nothing is kept
/// or merged here (ios-cross-platform.md §1).
@MainActor
public final class OutlineStore: ObservableObject {
    public let outline: Outline
    /// The rows of the page shown.
    @Published public private(set) var rows: [RowView] = []
    /// The pages opened, Home first (empty: Home).
    @Published public private(set) var pages: [RowView] = []
    @Published public private(set) var syncStatus: SyncStatus = .off
    /// The last edit that failed, to show.
    @Published public var problem: String?
    /// Where the caret goes next, after a key moved it to another row: that row's editor takes
    /// it and clears it. Not published: the key's edit already updates the rows.
    public var focus: Focus?

    public init(outline: Outline) {
        self.outline = outline
        reload()
    }

    /// The outline in the database at `url` (made if it isn't there).
    public static func open(databaseAt url: URL) throws -> OutlineStore {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        return OutlineStore(outline: try Outline.open(db: url.path))
    }

    /// The bullet whose page is shown; nil for Home.
    public var root: String? { pages.last?.id }

    public func reload() {
        do {
            rows = try outline.page(root: root)
        } catch {
            // The page's bullet is gone (deleted on another device): back to Home.
            pages = []
            rows = (try? outline.page(root: nil)) ?? []
        }
        syncStatus = outline.syncStatus()
    }

    /// Open a bullet as a page.
    public func open(_ row: RowView) {
        pages.append(row)
        reload()
    }

    /// Back to the page above (Home when there is none).
    public func back() {
        _ = pages.popLast()
        reload()
    }

    /// Run an edit, then show its result (or why it failed).
    private func edit(_ change: () throws -> Void) {
        do {
            try change()
            problem = nil
        } catch {
            problem = String(describing: error)
        }
        reload()
        _ = outline.pumpSync()
    }

    /// Add a bullet at the end of the page shown.
    public func add(_ text: String) {
        let index = UInt32(rows.filter { $0.depth == 0 }.count)
        edit { _ = try outline.create(parent: root, index: index, text: text) }
    }

    public func setText(_ row: RowView, _ text: String) {
        guard text != row.text else { return }
        edit { try outline.setText(id: row.id, text: text) }
    }

    public func toggle(_ row: RowView) {
        edit { try outline.setCompleted(id: row.id, completed: !row.completed) }
    }

    /// Return in a bullet's text with `start..<end` selected (UTF-8 byte offsets; equal for a
    /// caret), as on the desktop: the text after the caret moves to a new bullet below, and the
    /// caret goes there (`focus`). One undo step.
    public func enter(_ row: RowView, from start: Int, to end: Int) {
        edit {
            let caret = try outline.enter(id: row.id, start: UInt32(max(0, start)), end: UInt32(max(start, end)), root: root)
            focus = Focus(caret)
        }
    }

    /// Backspace with the caret at the start of a bullet's text, as on the desktop: it joins the
    /// bullet above, or goes if it is empty, and the caret goes where they meet (`focus`). One
    /// undo step; nothing happens on the first bullet.
    public func backspaceAtStart(_ row: RowView) {
        edit {
            if let caret = try outline.backspaceAtStart(id: row.id, root: root) {
                focus = Focus(caret)
            }
        }
    }

    /// Make a bullet the last child of the one above it.
    public func indent(_ row: RowView) {
        edit { _ = try outline.indent(id: row.id) }
    }

    /// Move a bullet out to follow its parent, staying on the page shown.
    public func outdent(_ row: RowView) {
        edit { _ = try outline.outdent(id: row.id, root: root) }
    }

    /// Move a bullet and what is under it to the Trash.
    public func delete(_ row: RowView) {
        edit { try outline.delete(id: row.id) }
    }

    public func undo() {
        edit { _ = outline.undo() }
    }

    public func redo() {
        edit { _ = outline.redo() }
    }

    /// The bullet being typed in (nil: none): what another device changes in it waits until
    /// editing ends, so the text under the caret never changes.
    public func editing(_ row: RowView?) {
        try? outline.setEditing(id: row?.id)
        if row == nil { reload() }
    }

    /// Sync with a self-hosted server (`flowie-server`) from now on.
    public func startSync(server: String, token: String) {
        outline.startSync(server: server, token: token, observer: Pump(store: self))
        syncStatus = outline.syncStatus()
    }

    public func stopSync() {
        outline.stopSync()
        syncStatus = outline.syncStatus()
    }

    /// Take what sync finished; the outline changed if it brought records.
    func pump() {
        if outline.pumpSync() {
            reload()
        } else {
            syncStatus = outline.syncStatus()
        }
    }
}

/// Where the caret goes: a row, and a UTF-8 byte offset in its text (as flowie-ffi counts).
public struct Focus: Equatable {
    public let id: String
    public let offset: Int

    public init(id: String, offset: Int) {
        self.id = id
        self.offset = offset
    }

    init(_ caret: CaretView) {
        self.init(id: caret.id, offset: Int(caret.offset))
    }
}

extension String {
    /// The UTF-8 byte offset of the UTF-16 offset `n` (what UIKit counts).
    func utf8Offset(utf16 n: Int) -> Int {
        let index = String.Index(utf16Offset: Swift.min(Swift.max(0, n), utf16.count), in: self)
        return self[..<index].utf8.count
    }

    /// The UTF-16 offset of the UTF-8 byte offset `n`.
    func utf16Offset(utf8 n: Int) -> Int {
        let index = utf8.index(utf8.startIndex, offsetBy: Swift.min(Swift.max(0, n), utf8.count))
        let boundary = String.Index(index, within: self) ?? index
        return utf16.distance(from: startIndex, to: boundary)
    }
}

/// Sync's threads say when there is something to take; the store takes it on the main actor.
final class Pump: SyncObserver, @unchecked Sendable {
    weak var store: OutlineStore?

    init(store: OutlineStore) {
        self.store = store
    }

    func wake() {
        Task { @MainActor [weak store] in store?.pump() }
    }
}
