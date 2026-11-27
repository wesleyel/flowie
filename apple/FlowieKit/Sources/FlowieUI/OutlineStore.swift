import Foundation
import FlowieFFI

/// The open outline, as the app's pages and lists share it: sync, stars, search, Recently
/// Deleted and the Hide Completed setting. Each page shown has its own `PageModel`; an edit on
/// one page reaches the others through here. Every read and write goes through flowie-ffi;
/// nothing is kept or merged here (ios-cross-platform.md §1, §4.2).
@MainActor
public final class OutlineStore: ObservableObject {
    public let outline: Outline
    @Published public private(set) var syncStatus: SyncStatus = .off
    /// Sync through iCloud (CloudSyncing.swift).
    @Published public internal(set) var iCloud: ICloudStatus = .off
    /// A new install fetching the outline from iCloud for the first time (§4.3).
    @Published public internal(set) var fetchingFirst = false
    /// The running `CloudController` (iOS 17 and macOS 14 only, so kept untyped), and where
    /// its state is kept.
    var cloud: AnyObject?
    var cloudStateURL: URL?
    /// The last edit that failed, to show.
    @Published public var problem: String?
    /// Whether pages leave out completed rows (this device's setting, as on the desktop).
    @Published public private(set) var hideCompleted: Bool
    /// Goes up with every change, so lists (stars, search results, Recently Deleted) read again.
    @Published public private(set) var revision = 0
    /// The pages alive now (weakly held).
    private let pages = NSHashTable<PageModel>.weakObjects()
    /// The last capture asked for (quick action, Control Center, Action Button): today's
    /// journal with the caret on a row (wiki: iOS-Cross-Platform §5). The shell shows the day;
    /// the page showing it puts the caret there once (`takeCapture`).
    @Published public private(set) var capture: JournalCapture?
    private var captureTaken = true

    public init(outline: Outline) {
        self.outline = outline
        hideCompleted = outline.hideCompleted()
    }

    /// The outline in the database at `url` (made if it isn't there).
    public static func open(databaseAt url: URL) throws -> OutlineStore {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        return OutlineStore(outline: try Outline.open(db: url.path))
    }

    /// A page to show.
    public func page(_ kind: PageKind) -> PageModel {
        let page = PageModel(store: self, kind: kind)
        pages.add(page)
        return page
    }

    /// An edit on `source` changed the outline: the other pages take what it changed (pages not
    /// on screen read it when they come back), and lists read again.
    func edited(_ outcome: EditOutcome, from source: PageModel?) {
        for page in pages.allObjects where page !== source {
            page.take(outcome)
        }
        revision += 1
        if outline.pumpSync() {
            reloadAll()
        }
        queueForICloud()
    }

    /// Every page reads its rows again (sync brought changes, or a setting changed).
    public func reloadAll() {
        for page in pages.allObjects {
            page.take(EditOutcome(changedIds: [], structureChanged: true, focus: nil))
        }
        syncStatus = outline.syncStatus()
        revision += 1
    }

    // MARK: Capture and leaving

    /// End typing on every page: text still being composed, and an open note, are saved; the
    /// keyboard goes; and other devices' changes held for the row being typed in are applied.
    public func finishEditing() {
        for page in pages.allObjects {
            page.finishEditor?()
        }
        try? outline.setEditing(id: nil)
    }

    /// Save everything so far (before the app may be ended, and after a write without the UI).
    public func flush() throws {
        try outline.flush()
    }

    /// "Add to Today's Journal" (Shortcuts, Siri): `text` at the end of today's journal, one
    /// bullet a line, and saved before returning. A row being typed in stays. Pages showing the
    /// day read it. False when `text` is blank (nothing changed).
    @discardableResult
    public func appendToToday(_ text: String) throws -> Bool {
        let outcome = try outline.journalAppend(date: LaunchMemory.today, text: text)
        guard outcome.focus != nil else { return false }
        edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
        try outline.flush()
        return true
    }

    /// The capture entries: today's journal with the caret on its empty last row, made only
    /// when there isn't one (so two devices don't each add one, §4.3).
    public func captureToday() throws {
        finishEditing()
        let day = LaunchMemory.today
        let outcome = try outline.journalCapture(date: day)
        edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
        guard let focus = outcome.focus else { return }
        captureTaken = false
        capture = JournalCapture(day: day, row: focus.id, serial: (capture?.serial ?? 0) + 1)
        for page in pages.allObjects {
            page.onCapture?()
        }
    }

    /// The caret for a capture not yet shown, when `page` shows its row; taking it clears it.
    func takeCapture(on page: PageModel) -> Focus? {
        guard !captureTaken, let capture, page.kind == .day(capture.day), page.rows.contains(where: { $0.id == capture.row }) else {
            return nil
        }
        captureTaken = true
        return Focus(id: capture.row, start: 0, end: 0)
    }

    // MARK: Settings and lists

    public func setHideCompleted(_ hide: Bool) {
        outline.setHideCompleted(hide: hide)
        hideCompleted = hide
        reloadAll()
    }

    /// A bullet with the path to it; nil when it is gone.
    public func bullet(_ id: String) -> BulletView? {
        try? outline.bullet(id: id)
    }

    public func starred() -> [BulletView] {
        outline.starred()
    }

    /// Star a bullet or unstar it (not an undo step, as on the desktop).
    public func setStarred(_ id: String, _ starred: Bool) {
        try? outline.setStarred(id: id, starred: starred)
        for page in pages.allObjects where page.root == id {
            page.readInfo()
        }
        revision += 1
        _ = outline.pumpSync()
    }

    /// Bullets matching `query`, with the desktop's filter syntax; the most recently changed
    /// first.
    public func search(_ query: String, limit: Int = 200) -> [BulletView] {
        outline.search(query: query, limit: UInt32(limit))
    }

    /// Bullets to pick by name (Move To), leaving out `exclude` and what is under them.
    public func quickFind(_ query: String, excluding exclude: [String], limit: Int = 40) -> [BulletView] {
        outline.quickFind(query: query, exclude: exclude, limit: UInt32(limit))
    }

    /// A Markdown link to a bullet, as the desktop's Copy Link makes it.
    public func link(_ id: String) -> String? {
        try? outline.link(id: id, untitled: NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text"))
    }

    /// What Recently Deleted lists, newest first.
    public func trashed() -> [TrashView] {
        outline.trashed()
    }

    /// Bring back a deleted bullet where it was (an undo step). Returns it, with the path to
    /// it, to go to.
    @discardableResult
    public func restore(_ id: String) -> BulletView? {
        do {
            let outcome = try outline.restore(id: id)
            edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
            return bullet(id)
        } catch {
            problem = String(describing: error)
            return nil
        }
    }

    // MARK: Sync

    /// Sync with a self-hosted server (`flowie-server`) from now on, instead of iCloud.
    public func startSync(server: String, token: String) {
        stopICloud()
        outline.startSync(server: server, token: token, observer: Pump(store: self))
        syncStatus = outline.syncStatus()
    }

    public func stopSync() {
        outline.stopSync()
        syncStatus = outline.syncStatus()
    }

    /// Sync through iCloud again, after a self-hosted server's sync stopped.
    public func resumeICloud() {
        if let url = cloudStateURL, iCloud == .off {
            startICloud(stateURL: url)
        }
    }

    /// Take what sync finished; the outline changed if it brought records.
    func pump() {
        if outline.pumpSync() {
            reloadAll()
        } else {
            syncStatus = outline.syncStatus()
        }
    }
}

/// Which page: Home, a bullet's, or a day of the journal (YYYY-MM-DD), which may not exist yet.
public enum PageKind: Hashable, Sendable {
    case home
    case bullet(String)
    case day(String)

    /// A short key for remembering the page (launch rule).
    public var key: String {
        switch self {
        case .home: return "home"
        case .bullet(let id): return "b:\(id)"
        case .day(let date): return "d:\(date)"
        }
    }
}

/// A capture asked for: the day of the journal and its row to put the caret on. `serial` tells
/// one capture from the next on the same row.
public struct JournalCapture: Equatable, Sendable {
    public let day: String
    public let row: String
    public let serial: Int
}

/// One page of the outline and its rows. An edit says what it changed (`EditOutcome`): typing
/// reads back only its row, and the page is read again when rows come, go or move. A day of the
/// journal that doesn't exist yet shows no rows, and is made (as on the desktop, with the same
/// ids on every device) only when something is written in it.
@MainActor
public final class PageModel: ObservableObject {
    public unowned let store: OutlineStore
    public let kind: PageKind
    /// The rows of the page.
    @Published public private(set) var rows: [RowView] = []
    /// The page's own bullet (none: Home, or a day not made yet): its title and the path to it.
    @Published public private(set) var info: BulletView?
    /// Where the caret goes next, after a key moved it to another row: the page view takes it
    /// and clears it. Not published: the key's edit already updates the rows.
    public var focus: Focus?
    /// Whether a row of the page is being typed in.
    @Published public private(set) var isEditing = false
    /// A row to scroll to and point out once shown (a search result); the page view clears it.
    public var reveal: String?
    /// Told right away, before the edit that changed the rows returns, so a page view can move
    /// the caret before the next key arrives (two quick Returns land on the new rows).
    public var onChange: ((PageChange) -> Void)?
    /// Set by the page view: end typing now (`OutlineStore.finishEditing`).
    var finishEditor: (() -> Void)?
    /// Set by the page view: a capture was asked for (`OutlineStore.captureToday`).
    var onCapture: (() -> Void)?
    /// Whether the page is on screen. Pages off screen don't read changes as they happen: they
    /// read the page again when they come back.
    public var isVisible = true {
        didSet {
            if isVisible && stale {
                reload()
            }
        }
    }
    private var stale = false
    /// The journal page shown, once it exists.
    private var dayID: String?

    init(store: OutlineStore, kind: PageKind) {
        self.store = store
        self.kind = kind
        readPage()
    }

    /// The bullet whose page is shown; nil for Home (and a day not made yet).
    public var root: String? {
        switch kind {
        case .home: return nil
        case .bullet(let id): return id
        case .day: return dayID
        }
    }

    /// Whether this is a day of the journal that hasn't been written in yet.
    public var isUnmadeDay: Bool {
        if case .day = kind { return dayID == nil }
        return false
    }

    /// Read the page again.
    public func reload() {
        readPage()
        onChange?(.rows)
    }

    private func readPage() {
        stale = false
        if case .day(let date) = kind {
            dayID = (try? store.outline.journalDay(date: date)) ?? nil
            if dayID == nil {
                rows = []
                info = nil
                return
            }
        }
        do {
            rows = try store.outline.page(root: root)
        } catch {
            // The page's bullet is gone (deleted, maybe on another device).
            rows = []
        }
        readInfo()
    }

    func readInfo() {
        info = root.flatMap { store.bullet($0) }
    }

    /// Another page's edit, or sync, changed the outline.
    func take(_ outcome: EditOutcome) {
        guard isVisible else {
            stale = true
            return
        }
        apply(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil))
    }

    /// Run an edit, then show what it changed (or why it failed).
    private func edit(_ change: () throws -> EditOutcome) {
        do {
            let outcome = try change()
            store.problem = nil
            apply(outcome)
            store.edited(outcome, from: self)
        } catch {
            store.problem = String(describing: error)
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
            if !shown.isEmpty, let fresh = try? store.outline.rows(ids: shown, root: root) {
                for row in fresh {
                    if let index = rows.firstIndex(where: { $0.id == row.id }) {
                        rows[index] = row
                    }
                }
                onChange?(.content(fresh.map(\.id)))
            } else if outcome.focus != nil {
                onChange?(.content([]))
            }
            if let root, outcome.changedIds.contains(root) {
                readInfo()
            }
        }
    }

    /// Add a bullet at the end of the page. With `focus`, the caret goes to it; otherwise it
    /// stays where it is. On a day not made yet, the day is made first, as on the desktop: with
    /// a blank first line, which is the bullet added.
    public func add(_ text: String, focus: Bool = false) {
        if case .day(let date) = kind, dayID == nil {
            edit {
                var outcome = try store.outline.openDay(date: date)
                if let first = outcome.focus?.id, !text.isEmpty {
                    let typed = try store.outline.setText(id: first, text: text)
                    outcome.changedIds += typed.changedIds
                    outcome.focus = FocusView(id: first, start: UInt32(text.utf16.count), end: UInt32(text.utf16.count))
                }
                if !focus { outcome.focus = nil }
                return outcome
            }
            return
        }
        let index = UInt32(rows.filter { $0.depth == 0 }.count)
        edit {
            var outcome = try store.outline.create(parent: root, index: index, text: text)
            if !focus { outcome.focus = nil }
            return outcome
        }
    }

    public func setText(_ row: RowView, _ text: String) {
        guard !text.utf8.elementsEqual(row.text.utf8) else { return }
        edit { try store.outline.setText(id: row.id, text: text) }
    }

    /// Replace a bullet's note (the notes sheet saves once, as one undo step).
    public func setNote(_ row: RowView, _ note: String) {
        guard !note.utf8.elementsEqual(row.note.utf8) else { return }
        edit { try store.outline.setNote(id: row.id, note: note) }
    }

    public func toggle(_ row: RowView) {
        edit { try store.outline.setCompleted(id: row.id, completed: !row.completed) }
    }

    /// Fold a bullet with children, or unfold it (one undo step).
    public func toggleFold(_ row: RowView) {
        guard row.hasChildren else { return }
        edit { try store.outline.setCollapsed(id: row.id, collapsed: !row.collapsed) }
    }

    /// Return in a bullet's text with `start..<end` selected (UTF-16 offsets, as UIKit counts;
    /// equal for a caret), as on the desktop: the text after the caret moves to a new bullet
    /// below, and the caret goes there (`focus`). One undo step.
    public func enter(_ row: RowView, from start: Int, to end: Int) {
        edit { try store.outline.enter(id: row.id, start: UInt32(max(0, start)), end: UInt32(max(start, end)), root: root) }
    }

    /// Backspace with the caret at the start of a bullet's text, as on the desktop: it joins the
    /// bullet above, or goes if it is empty, and the caret goes where they meet (`focus`). One
    /// undo step; nothing happens on the first bullet.
    public func backspaceAtStart(_ row: RowView) {
        edit { try store.outline.backspaceAtStart(id: row.id, root: root) }
    }

    /// Make a bullet the last child of the one above it.
    public func indent(_ row: RowView) {
        edit { try store.outline.indent(id: row.id) }
    }

    /// Move a bullet out to follow its parent, staying on the page.
    public func outdent(_ row: RowView) {
        edit { try store.outline.outdent(id: row.id, root: root) }
    }

    /// Move a bullet above the one before it, as the desktop's Move Up, staying on the page.
    public func moveUp(_ row: RowView) {
        edit { try store.outline.moveUp(id: row.id, root: root) }
    }

    /// Move a bullet below the one after it, as the desktop's Move Down.
    public func moveDown(_ row: RowView) {
        edit { try store.outline.moveDown(id: row.id, root: root) }
    }

    /// Move a bullet, with what is under it, to the end of `parent` (nil: Home), as the
    /// desktop's Move To. One undo step.
    public func move(_ id: String, to parent: String?) {
        edit { try store.outline.moveTo(id: id, parent: parent) }
    }

    /// Move a bullet and what is under it to the Trash.
    public func delete(_ row: RowView) {
        edit { try store.outline.delete(id: row.id) }
    }

    /// Undo the last step; the caret goes back where it was, when that is known.
    public func undo() {
        edit { store.outline.undo() }
    }

    public func redo() {
        edit { store.outline.redo() }
    }

    /// The bullet being typed in (nil: none): what another device changes in it waits until
    /// editing ends, so the text under the caret never changes.
    public func editing(_ row: RowView?) {
        try? store.outline.setEditing(id: row?.id)
        if isEditing != (row != nil) {
            isEditing = row != nil
        }
        if row == nil { reload() }
    }
}

/// How the rows of a page changed.
public enum PageChange: Equatable {
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

extension BulletView {
    /// The text to show for it: "Untitled" when it has none.
    public var title: String {
        text.trimmingCharacters(in: .whitespaces).isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : text
    }

    /// The path to it as one line ("Home" when it is at the top), cut in the middle to about
    /// `limit` characters: the first and the last bullets on the path stay.
    public func pathLine(limit: Int = 48) -> String {
        Self.line(path.map { $0.text.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : $0.text }, limit: limit)
    }

    /// `parts` joined with " › ", cut in the middle (with "…") to about `limit` characters.
    public static func line(_ parts: [String], limit: Int) -> String {
        guard !parts.isEmpty else { return NSLocalizedString("Home", comment: "Title of the outline's top page") }
        let sep = " › "
        let whole = parts.joined(separator: sep)
        guard whole.count > limit, parts.count > 1 else { return whole }
        // Keep the first and as many of the last as fit.
        let first = parts[0]
        var tail: [String] = []
        for part in parts.dropFirst().reversed() {
            let candidate = ([first, "…"] + [part] + tail).joined(separator: sep)
            if candidate.count > limit && !tail.isEmpty { break }
            tail.insert(part, at: 0)
            if candidate.count > limit { break }
        }
        if tail.count == parts.count - 1 { return whole }
        return ([first, "…"] + tail).joined(separator: sep)
    }
}
