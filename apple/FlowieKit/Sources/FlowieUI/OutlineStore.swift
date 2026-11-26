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
