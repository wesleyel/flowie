import Foundation
import FlowieFFI

/// The outline as the views see it: the page shown and its rows. An edit says what it changed
/// (`EditOutcome`): typing reads back only its row, and the page is read again when rows come,
/// go or move, and each time sync brings changes. Every read and write goes through flowie-ffi;
/// nothing is kept or merged here (ios-cross-platform.md §1, §4.2).
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
    /// Told right away, before the edit that changed the rows returns, so a page view can move
    /// the caret before the next key arrives (two quick Returns land on the new rows).
    public var onChange: ((PageChange) -> Void)?

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

    /// Read the page again.
    public func reload() {
        readPage()
        onChange?(.rows)
    }

    private func readPage() {
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
        readPage()
        onChange?(.page)
    }

    /// Back to the page above (Home when there is none).
    public func back() {
        _ = pages.popLast()
        readPage()
        onChange?(.page)
    }

    /// Run an edit, then show what it changed (or why it failed).
    private func edit(_ change: () throws -> EditOutcome) {
        do {
            apply(try change())
            problem = nil
        } catch {
            problem = String(describing: error)
            reload()
        }
        if outline.pumpSync() {
            reload()
        }
    }

    /// Show what an edit changed: the page read again when rows came, went or moved, otherwise
    /// just the rows that changed. The caret moves when the edit says where.
    private func apply(_ outcome: EditOutcome) {
        if let focus = outcome.focus {
            self.focus = Focus(focus)
        }
        if outcome.structureChanged {
            readPage()
            onChange?(.rows)
        } else {
            let shown = outcome.changedIds.filter { id in rows.contains { $0.id == id } }
            if !shown.isEmpty, let fresh = try? outline.rows(ids: shown, root: root) {
                for row in fresh {
                    if let index = rows.firstIndex(where: { $0.id == row.id }) {
                        rows[index] = row
                    }
                }
                onChange?(.content(fresh.map(\.id)))
            } else if outcome.focus != nil {
                onChange?(.content([]))
            }
        }
    }

    /// Add a bullet at the end of the page shown. With `focus`, the caret goes to it;
    /// otherwise it stays where it is.
    public func add(_ text: String, focus: Bool = false) {
        let index = UInt32(rows.filter { $0.depth == 0 }.count)
        edit {
            var outcome = try outline.create(parent: root, index: index, text: text)
            if !focus { outcome.focus = nil }
            return outcome
        }
    }

    public func setText(_ row: RowView, _ text: String) {
        guard text != row.text else { return }
        edit { try outline.setText(id: row.id, text: text) }
    }

    public func toggle(_ row: RowView) {
        edit { try outline.setCompleted(id: row.id, completed: !row.completed) }
    }

    /// Return in a bullet's text with `start..<end` selected (UTF-16 offsets, as UIKit counts;
    /// equal for a caret), as on the desktop: the text after the caret moves to a new bullet
    /// below, and the caret goes there (`focus`). One undo step.
    public func enter(_ row: RowView, from start: Int, to end: Int) {
        edit { try outline.enter(id: row.id, start: UInt32(max(0, start)), end: UInt32(max(start, end)), root: root) }
    }

    /// Backspace with the caret at the start of a bullet's text, as on the desktop: it joins the
    /// bullet above, or goes if it is empty, and the caret goes where they meet (`focus`). One
    /// undo step; nothing happens on the first bullet.
    public func backspaceAtStart(_ row: RowView) {
        edit { try outline.backspaceAtStart(id: row.id, root: root) }
    }

    /// Make a bullet the last child of the one above it.
    public func indent(_ row: RowView) {
        edit { try outline.indent(id: row.id) }
    }

    /// Move a bullet out to follow its parent, staying on the page shown.
    public func outdent(_ row: RowView) {
        edit { try outline.outdent(id: row.id, root: root) }
    }

    /// Move a bullet above the one before it, as the desktop's Move Up, staying on the page
    /// shown.
    public func moveUp(_ row: RowView) {
        edit { try outline.moveUp(id: row.id, root: root) }
    }

    /// Move a bullet below the one after it, as the desktop's Move Down.
    public func moveDown(_ row: RowView) {
        edit { try outline.moveDown(id: row.id, root: root) }
    }

    /// Move a bullet and what is under it to the Trash.
    public func delete(_ row: RowView) {
        edit { try outline.delete(id: row.id) }
    }

    /// Undo the last step; the caret goes back where it was, when that is known.
    public func undo() {
        edit { outline.undo() }
    }

    public func redo() {
        edit { outline.redo() }
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

/// How the rows of the page shown changed.
public enum PageChange: Equatable {
    /// Another page is shown.
    case page
    /// Rows came, went or moved (or anything may have changed): the rows were read again.
    case rows
    /// Only these rows' content changed (none: only the caret moves).
    case content([String])
}

/// Where the caret goes: a row, and the selection in its text in UTF-16 offsets (as UIKit and
/// flowie-ffi count); a caret when `start == end`.
public struct Focus: Equatable {
    public let id: String
    public let start: Int
    public let end: Int

    public init(id: String, start: Int, end: Int) {
        self.id = id
        self.start = start
        self.end = end
    }

    init(_ focus: FocusView) {
        self.init(id: focus.id, start: Int(focus.start), end: Int(focus.end))
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
