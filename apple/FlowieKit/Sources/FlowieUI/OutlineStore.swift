import Foundation
import FlowieFFI
import FlowieInbox

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
    /// Reading mode: nothing on the pages can be edited (rows don't take the caret, menus leave
    /// out what edits), while links, zooming in, folding and stars still work. This device's
    /// view setting for the whole app, kept across launches; a capture turns it off.
    @Published public private(set) var readingMode: Bool
    /// Which buttons stay in a page's top-right toolbar besides its "…" menu, kept on this
    /// device ("…" → Customize Toolbar…). Only Fold is on by default.
    @Published public private(set) var toolbarActions: Set<ToolbarAction>
    /// Where `readingMode` and `toolbarActions` are stored (swapped in tests).
    public static var defaults: UserDefaults = .standard
    /// Whether Home shows "Tap a bullet to open it" under its rows: only for an outline made
    /// on this device that nothing synced into. It goes for good once anything is edited here,
    /// sync (iCloud or a self-hosted server) brings changes, or a bullet is opened.
    @Published public private(set) var showsHomeHint: Bool
    /// Goes up with every change, so lists (stars, search results, Recently Deleted) read again.
    @Published public private(set) var revision = 0
    /// The pages alive now (weakly held).
    private let pages = NSHashTable<PageModel>.weakObjects()
    /// The last capture asked for (quick action, Control Center, Action Button): today's
    /// journal with the caret on a row (wiki: iOS-Cross-Platform §5). The shell shows the day;
    /// the page showing it puts the caret there once (`takeCapture`).
    @Published public private(set) var capture: JournalCapture?
    private var captureTaken = true
    /// The last page asked for from outside the window (the Open Page and Search intents,
    /// Spotlight, Handoff): the shell shows it.
    @Published public private(set) var opening: OpenRequest?
    private var openingTaken = true

    public init(outline: Outline) {
        self.outline = outline
        hideCompleted = outline.hideCompleted()
        readingMode = Self.defaults.bool(forKey: Self.readingModeKey)
        if let saved = Self.defaults.stringArray(forKey: Self.toolbarActionsKey) {
            toolbarActions = Set(saved.compactMap(ToolbarAction.init(rawValue:)))
        } else {
            toolbarActions = ToolbarAction.defaults
        }
        showsHomeHint = !LaunchMemory.homeHintGone
    }

    /// Hide Home's hint for new users, for good.
    func retireHomeHint() {
        guard showsHomeHint else { return }
        LaunchMemory.homeHintGone = true
        showsHomeHint = false
    }

    /// Sync brought changes: pages read them, and Home's hint for new users goes.
    func syncedIn() {
        retireHomeHint()
        reloadAll()
    }

    static let readingModeKey = "readingMode"
    static let toolbarActionsKey = "toolbarActions"

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
    /// on screen read it when they come back), and lists read again. `writes`: false for a
    /// view change (folding).
    func edited(_ outcome: EditOutcome, from source: PageModel?, writes: Bool = true) {
        if writes {
            retireHomeHint()
        }
        for page in pages.allObjects where page !== source {
            page.take(outcome)
        }
        revision += 1
        if outline.pumpSync() {
            syncedIn()
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
    /// day read it. False when `text` is blank (nothing changed). Reading mode stays as it is:
    /// asked for explicitly, this writes past it, and no row takes the caret.
    @discardableResult
    public func appendToToday(_ text: String) throws -> Bool {
        let outcome = try outline.journalAppend(date: LaunchMemory.today, text: text)
        guard outcome.focus != nil else { return false }
        edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
        try outline.flush()
        return true
    }

    /// The share extension's shares waiting in `inbox`, each added to the end of the journal
    /// page of the day it was shared (wiki: iOS-Cross-Platform §5), oldest first, each one undo
    /// step. A file is deleted only once the outline has its share saved; a share already in
    /// the outline (its file outlived the save, say the app was ended between the two) adds
    /// nothing, as the outline knows it by its id. A row being typed in stays, as with
    /// `appendToToday`. Returns how many shares were added.
    @discardableResult
    public func importShares(from inbox: ShareInbox) throws -> Int {
        let items = inbox.pending()
        guard !items.isEmpty else { return 0 }
        var added = 0
        for item in items {
            let outcome = try outline.journalShare(date: item.journalDay, key: item.id, text: item.text, url: item.url ?? "")
            if outcome.focus != nil {
                edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
                added += 1
            }
        }
        try outline.flush()
        for item in items {
            try inbox.remove(item)
        }
        return added
    }

    /// The capture entries: today's journal with the caret on its empty last row, made only
    /// when there isn't one (so two devices don't each add one, §4.3). The user asked to write:
    /// reading mode turns off.
    public func captureToday() throws {
        try captureToday(typed: "")
    }

    /// A capture with `typed` already in its row and the caret after it (debug builds'
    /// screenshots, `ScreenshotArguments`).
    func captureToday(typed: String) throws {
        setReadingMode(false)
        finishEditing()
        let day = LaunchMemory.today
        var outcome = try outline.journalCapture(date: day)
        guard let focus = outcome.focus else {
            edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
            return
        }
        if !typed.isEmpty {
            outcome.changedIds += try outline.setText(id: focus.id, text: typed).changedIds
        }
        edited(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil), from: nil)
        captureTaken = false
        capture = JournalCapture(day: day, row: focus.id, caret: typed.utf16.count, serial: (capture?.serial ?? 0) + 1)
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
        return Focus(id: capture.row, start: capture.caret, end: capture.caret)
    }

    // MARK: Opening from outside

    /// Show `page` in the window, as a new page of its history (the Open Page and Search
    /// intents, Spotlight, Handoff).
    public func show(_ page: PageRef) {
        openingTaken = false
        opening = OpenRequest(page: page, serial: (opening?.serial ?? 0) + 1)
    }

    /// The page asked for and not yet shown; taking it clears it.
    func takeOpening() -> PageRef? {
        guard !openingTaken, let opening else { return nil }
        openingTaken = true
        return opening.page
    }

    /// The page of bullet `id`: a journal page opens as its day.
    public func pageRef(bullet id: String) -> PageRef {
        outline.journalDate(id: id).map { .day($0) } ?? .bullet(id)
    }

    // MARK: Settings and lists

    /// Turn reading mode on or off. Turning it on ends typing first (the keyboard goes).
    public func setReadingMode(_ on: Bool) {
        guard on != readingMode else { return }
        if on {
            finishEditing()
        }
        readingMode = on
        Self.defaults.set(on, forKey: Self.readingModeKey)
    }

    /// Show or hide `action` in a page's top-right toolbar ("…" → Customize Toolbar…).
    public func setToolbar(_ action: ToolbarAction, _ shown: Bool) {
        var next = toolbarActions
        if shown { next.insert(action) } else { next.remove(action) }
        guard next != toolbarActions else { return }
        toolbarActions = next
        Self.defaults.set(ToolbarAction.allCases.filter(next.contains).map(\.rawValue), forKey: Self.toolbarActionsKey)
    }

    /// Put the top-right toolbar back to its default (only Fold shown).
    public func resetToolbar() {
        toolbarActions = ToolbarAction.defaults
        Self.defaults.removeObject(forKey: Self.toolbarActionsKey)
    }

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

    /// The searches saved on the desktop (they sync), for the iPad sidebar.
    public func savedSearches() -> [String] {
        outline.savedSearches()
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
    /// it, to go to. Nothing in reading mode.
    @discardableResult
    public func restore(_ id: String) -> BulletView? {
        guard !readingMode else { return nil }
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
            syncedIn()
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

/// A capture asked for: the day of the journal, its row to put the caret on and where in the row
/// (UTF-16). `serial` tells one capture from the next on the same row.
public struct JournalCapture: Equatable, Sendable {
    public let day: String
    public let row: String
    public let caret: Int
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
    /// How many levels of bullets the page has when unfolded (`0`..`3`, where `3` means three or
    /// more), leaving out completed ones when hidden: Fold shows at `2`+, Expand All only at `3`.
    @Published public private(set) var maxLevels = 0
    /// Whether the page has any foldable children (`2` or more visible levels).
    public var canFold: Bool { maxLevels >= 2 }
    /// Whether the page goes deeper than two levels (so Expand All goes further than Show 2 Levels).
    public var hasDeepLevels: Bool { maxLevels > 2 }
    /// The page's own bullet (none: Home, or a day not made yet): its title and the path to it.
    @Published public private(set) var info: BulletView?
    /// What points at the page (the bullets linking to it, then its mirrors), listed read-only
    /// under its rows as on the desktop. Read again after every change: an edit anywhere can
    /// add or take away a link.
    @Published public private(set) var backlinks: [BacklinkView] = []
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
    /// Reading mode (`OutlineStore.readingMode`): the page's edits do nothing, except folding.
    public var isReadOnly: Bool { store.readingMode }
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
                maxLevels = 0
                foldState = nil
                info = nil
                readBacklinks()
                return
            }
        }
        do {
            rows = try store.outline.pageRows(root: root)
            maxLevels = Int((try? store.outline.pageLevels(root: root)) ?? 0)
        } catch {
            // The page's bullet is gone (deleted, maybe on another device).
            rows = []
            maxLevels = 0
        }
        if !hasDeepLevels, foldState == .expandAll {
            foldState = .two
        }
        readInfo()
    }

    func readInfo() {
        info = root.flatMap { store.bullet($0) }
        readBacklinks()
    }

    private func readBacklinks() {
        let fresh = root.flatMap { try? store.outline.backlinks(id: $0) } ?? []
        if fresh != backlinks {
            backlinks = fresh
        }
    }

    /// Another page's edit, or sync, changed the outline.
    func take(_ outcome: EditOutcome) {
        guard isVisible else {
            stale = true
            return
        }
        apply(EditOutcome(changedIds: outcome.changedIds, structureChanged: outcome.structureChanged, focus: nil))
    }

    /// Run an edit, then show what it changed (or why it failed). In reading mode only a view
    /// change (folding: `writes` false) runs; this is the one gate every edit of the page's
    /// content goes through.
    private func edit(writes: Bool = true, _ change: () throws -> EditOutcome) {
        guard !writes || !isReadOnly else { return }
        do {
            let outcome = try change()
            store.problem = nil
            apply(outcome)
            store.edited(outcome, from: self, writes: writes)
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
            } else {
                readBacklinks()
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

    /// Fold a bullet with children, or unfold it (one undo step). The fold button goes neutral.
    public func toggleFold(_ row: RowView) {
        guard row.hasChildren else { return }
        foldState = nil
        edit(writes: false) { try store.outline.setCollapsed(id: row.id, collapsed: !row.collapsed) }
    }

    /// The state the fold button shows: the last one it (or a ⌃-number key) folded the page to
    /// here, nil once a row was folded or unfolded by hand, or an undo may have changed folding.
    @Published public private(set) var foldState: FoldLevel?

    /// Fold the page to `level`, as the desktop's Show N Levels (`levels` deep: 1 folds every
    /// top-level row, `.max` unfolds everything). One undo step; works in reading mode.
    public func fold(to level: FoldLevel) {
        guard !isUnmadeDay else { return }
        edit(writes: false) { try store.outline.expandToLevel(root: root, levels: level.levels) }
        foldState = (!hasDeepLevels && level == .expandAll) ? .two : level
    }

    /// How many children a row has as the page would show them (completed ones left out when
    /// hidden), for VoiceOver; a collapsed row's are read from the outline. Nil when unknown.
    public func childCount(_ row: RowView) -> Int? {
        guard row.hasChildren else { return 0 }
        if row.collapsed {
            return (try? store.outline.pageRows(root: row.id))?.filter { $0.depth == 0 }.count
        }
        guard let index = rows.firstIndex(where: { $0.id == row.id }) else { return nil }
        return RowNavigation.childCount(of: index, in: rows.map { Int($0.depth) })
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

    /// Where a bullet dragged over the page lands in the gap below `above` (none: the top of
    /// the page), over `below` (none: the end), with its bullet `depth` levels in, as on the
    /// desktop. `id` is nil for something new (text from another app). Nil when it can't go
    /// there: under itself, on a day not made yet, or in reading mode.
    public func dropPlace(_ id: String?, above: RowView?, below: RowView?, depth: Int) -> DropPlace? {
        guard !isReadOnly, !isUnmadeDay else { return nil }
        return try? store.outline.dropPlace(
            root: root, dragged: id, above: above?.id, aboveDepth: above?.depth ?? 0,
            belowDepth: below?.depth, depth: UInt16(clamping: max(0, depth)))
    }

    /// Move a dragged bullet, with what is under it, to `place` (from this page or another),
    /// as the desktop's drag and drop: its new parent unfolds, it keeps its own folding. One
    /// undo step.
    public func drop(_ id: String, at place: DropPlace) {
        edit { try store.outline.dropBullet(id: id, place: place) }
    }

    /// Add `texts` (dropped from another app) as new bullets at `place`, one per text, in
    /// order.
    public func insert(_ texts: [String], at place: DropPlace) {
        edit {
            var outcome = EditOutcome(changedIds: [], structureChanged: false, focus: nil)
            for (i, text) in texts.enumerated() {
                let made = try store.outline.create(parent: place.parent, index: place.index + UInt32(i), text: text)
                outcome.changedIds += made.changedIds
                outcome.structureChanged = outcome.structureChanged || made.structureChanged
            }
            return outcome
        }
    }

    /// Move a bullet and what is under it to the Trash.
    public func delete(_ row: RowView) {
        edit { try store.outline.delete(id: row.id) }
    }

    /// Undo the last step; the caret goes back where it was, when that is known.
    public func undo() {
        foldState = nil
        edit { store.outline.undo() }
    }

    public func redo() {
        foldState = nil
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

/// A page asked for from outside the window; `serial` tells two asks for one page apart.
public struct OpenRequest: Equatable, Sendable {
    public let page: PageRef
    let serial: Int
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

/// The states of the page's fold button and ⌃-number keys, in the order a tap steps through
/// them: Collapse All (the desktop's Show 1 Level), Show 2 Levels, and — only when the page has
/// more than two levels — Expand All.
public enum FoldLevel: CaseIterable {
    case collapseAll, two, expandAll

    /// The states offered on a page (`deep`: more than two levels).
    public static func available(deep: Bool) -> [FoldLevel] {
        deep ? [.collapseAll, .two, .expandAll] : [.collapseAll, .two]
    }

    /// The state a tap on the fold button goes to from `state` (`nil`: rows were folded or
    /// unfolded by hand since, or the page wasn't folded from the button yet; `deep`: whether
    /// the page has more than two levels).
    public static func after(_ state: FoldLevel?, deep: Bool = true) -> FoldLevel {
        switch state {
        case nil, .expandAll: .collapseAll
        case .collapseAll: .two
        case .two: deep ? .expandAll : .collapseAll
        }
    }

    var levels: UInt32 {
        switch self {
        case .collapseAll: 1
        case .two: 2
        case .expandAll: .max
        }
    }

    var title: String {
        switch self {
        case .collapseAll: NSLocalizedString("Collapse All", comment: "Page menu item: fold every top-level bullet")
        case .two: NSLocalizedString("Show 2 Levels", comment: "Page menu item: show top-level bullets and their children")
        case .expandAll: NSLocalizedString("Expand All", comment: "Page menu item: unfold every bullet on the page")
        }
    }

    var symbol: String {
        switch self {
        case .collapseAll: "list.bullet"
        case .two: "list.bullet.indent"
        case .expandAll: "arrow.up.and.line.horizontal.and.arrow.down"
        }
    }

    /// The fold button's symbol when the page isn't in one of the states.
    static let neutralSymbol = "arrow.down.and.line.horizontal.and.arrow.up"

    /// The desktop's key, with ⌃.
    var key: String {
        switch self {
        case .collapseAll: "1"
        case .two: "2"
        case .expandAll: "0"
        }
    }
}

/// A button that can sit in a page's top-right toolbar besides its "…" menu, turned on or off
/// from "…" → Customize Toolbar… and kept on this device. Only Fold is on by default.
public enum ToolbarAction: String, CaseIterable, Sendable {
    case upLevel, star, readingMode, hideCompleted, twoPanes, copyLink, fold

    public static let defaults: Set<ToolbarAction> = [.fold]

    /// Order listed in the Customize Toolbar sheet (Fold first, as the default).
    public static let sheetOrder: [ToolbarAction] = [.fold, .upLevel, .star, .readingMode, .hideCompleted, .twoPanes, .copyLink]

    public var title: String {
        switch self {
        case .fold: NSLocalizedString("Fold", comment: "Navigation bar button that folds the page a step further each tap")
        case .upLevel: NSLocalizedString("Up a Level", comment: "Navigation bar button that opens the page of the bullet above this one")
        case .star: NSLocalizedString("Star", comment: "Menu item")
        case .readingMode: NSLocalizedString("Reading Mode", comment: "Page menu toggle: nothing can be edited while on")
        case .hideCompleted: NSLocalizedString("Hide Completed", comment: "Menu item")
        case .twoPanes: NSLocalizedString("Two Panes", comment: "Page menu toggle: the page before shows beside the current one (iPad landscape)")
        case .copyLink: NSLocalizedString("Copy Link", comment: "Menu item")
        }
    }

    public var symbol: String {
        switch self {
        case .fold: FoldLevel.neutralSymbol
        case .upLevel: "arrow.turn.left.up"
        case .star: "star"
        case .readingMode: "eyeglasses"
        case .hideCompleted: "checklist"
        case .twoPanes: "rectangle.split.2x1"
        case .copyLink: "link"
        }
    }
}
