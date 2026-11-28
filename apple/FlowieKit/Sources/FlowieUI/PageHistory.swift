import Foundation

/// A page a window's history can hold (wiki: Sliding-Panes §1): a page of the outline, or a list.
public enum PageRef: Hashable, Codable, Sendable {
    case home
    case bullet(String)
    /// A day of the journal (YYYY-MM-DD).
    case day(String)
    /// The starred bullets (the Stars tab).
    case stars
    /// Search, with what is searched for.
    case search(String)

    /// The outline page it shows, if it is one.
    public var kind: PageKind? {
        switch self {
        case .home: return .home
        case .bullet(let id): return .bullet(id)
        case .day(let day): return .day(day)
        case .stars, .search: return nil
        }
    }

    /// What a pane's view is kept by: a search stays the same page while its query is typed,
    /// another day of the journal is another page.
    public var identity: String {
        switch self {
        case .search: return "search"
        case .stars: return "stars"
        default: return kind?.key ?? ""
        }
    }
}

/// One place in a window's history: the page, and how it was left (the row at the top and how
/// far into it, the row with the caret), so going back puts it back as it was, as the desktop's
/// `back_pages` do.
public struct PageEntry: Hashable, Codable, Sendable, Identifiable {
    public var id: UUID
    public var page: PageRef
    /// The row of the page before it in the history that opened it (the left pane points it out).
    public var opener: String?
    /// A row to scroll to and point out when the page first shows (a search result).
    public var reveal: String?
    /// The row at the top of the page when it was left, and how far it was scrolled into it.
    public var topRow: String?
    public var offset: Double
    /// The row being typed in when the page was left, and the caret in it (UTF-16).
    public var focus: String?
    public var caret: Int

    public init(_ page: PageRef, opener: String? = nil, reveal: String? = nil) {
        id = UUID()
        self.page = page
        self.opener = opener
        self.reveal = reveal
        topRow = nil
        offset = 0
        focus = nil
        caret = 0
    }
}

/// How a page was left: the row at the top and how far it was scrolled into it (points), and
/// the row with the caret and where in it (UTF-16).
public struct PageMemory: Equatable, Sendable {
    public var topRow: String?
    public var offset: Double
    public var focus: String?
    public var caret: Int

    public init(topRow: String? = nil, offset: Double = 0, focus: String? = nil, caret: Int = 0) {
        self.topRow = topRow
        self.offset = offset
        self.focus = focus
        self.caret = caret
    }
}

extension PageEntry {
    /// How the page was left, to put it back so.
    public var memory: PageMemory {
        get { PageMemory(topRow: topRow, offset: offset, focus: focus, caret: caret) }
        set {
            topRow = newValue.topRow
            offset = newValue.offset
            focus = newValue.focus
            caret = newValue.caret
        }
    }
}

/// A window's browser-style history (wiki: Sliding-Panes §1): its pages `[p0 … pn]` and where it
/// is (`index`). One pane shows `entries[index]`; two panes show the page before it on the left.
/// Opening a page drops what was ahead of the pane that opened it; Back and Forward move along.
/// Plain data, no UI: the shell keeps one per window (and one per tab on iPhone).
public struct PageHistory: Codable, Equatable, Sendable {
    public private(set) var entries: [PageEntry]
    public private(set) var index: Int
    /// How many pages it keeps: the oldest go first.
    public static let limit = 100

    public init(_ first: PageEntry) {
        entries = [first]
        index = 0
    }

    public init(_ page: PageRef) {
        self.init(PageEntry(page))
    }

    /// A history as saved; nil when there is nothing in it.
    public init?(entries: [PageEntry], index: Int) {
        guard !entries.isEmpty else { return nil }
        self.entries = entries
        self.index = min(max(0, index), entries.count - 1)
    }

    public var current: PageEntry { entries[index] }

    /// The page before the current one, which two panes show on the left.
    public var previous: PageEntry? { index > 0 ? entries[index - 1] : nil }

    public var canGoBack: Bool { index > 0 }
    public var canGoForward: Bool { index < entries.count - 1 }

    /// What Back can go to, nearest first (the back button's menu), with each one's position.
    public var backList: [(position: Int, entry: PageEntry)] {
        (0..<index).reversed().map { ($0, entries[$0]) }
    }

    /// What Forward can go to, nearest first.
    public var forwardList: [(position: Int, entry: PageEntry)] {
        ((index + 1)..<entries.count).map { ($0, entries[$0]) }
    }

    /// Open a page from the one at `position`: what was after it goes, the page goes after it,
    /// and it becomes the current page. From the current page (the right pane, or the only one)
    /// that pushes; from the page before it (the left pane) it replaces the right pane. Opening
    /// the page that already follows `position` keeps the history ahead of it.
    public mutating func open(_ entry: PageEntry, from position: Int) {
        let position = min(max(0, position), entries.count - 1)
        if position + 1 < entries.count, entries[position + 1].page == entry.page, entry.reveal == nil {
            entries[position + 1].opener = entry.opener
            index = position + 1
            return
        }
        if entries[position].page == entry.page, entry.reveal == nil {
            // A link to the page it is on: nothing to open.
            return
        }
        entries.removeSubrange((position + 1)...)
        entries.append(entry)
        index = entries.count - 1
        trim()
    }

    /// Open a page from the current one.
    public mutating func push(_ entry: PageEntry) {
        open(entry, from: index)
    }

    @discardableResult
    public mutating func goBack() -> Bool {
        guard canGoBack else { return false }
        index -= 1
        return true
    }

    @discardableResult
    public mutating func goForward() -> Bool {
        guard canGoForward else { return false }
        index += 1
        return true
    }

    /// Go to a page of the history (from the back or forward button's menu).
    public mutating func go(to position: Int) {
        index = min(max(0, position), entries.count - 1)
    }

    /// Show another page in the entry at `position`, without a new entry: the journal's previous
    /// and next day, Up a Level, or a breadcrumb, so switching in place doesn't push onto the
    /// history or wipe out the pane beside it. How it was scrolled goes with it.
    public mutating func replace(at position: Int, with page: PageRef, opener: String? = nil, reveal: String? = nil) {
        guard entries.indices.contains(position) else { return }
        guard entries[position].page != page || reveal != nil else { return }
        entries[position].page = page
        entries[position].opener = opener
        entries[position].reveal = reveal
        entries[position].topRow = nil
        entries[position].offset = 0
        entries[position].focus = nil
        entries[position].caret = 0
    }

    /// Change the entry with this id, wherever it is.
    public mutating func update(_ id: UUID, _ change: (inout PageEntry) -> Void) {
        if let position = entries.firstIndex(where: { $0.id == id }) {
            change(&entries[position])
        }
    }

    public func position(of id: UUID) -> Int? {
        entries.firstIndex { $0.id == id }
    }

    /// Leave out pages that are gone (their bullet was deleted), as the desktop's Back skips
    /// them, and the same page twice in a row that leaves. The current page stays current if
    /// it is still there; otherwise the page before it is. Never empty: Home is left.
    public mutating func prune(keeping exists: (PageRef) -> Bool) {
        var kept: [PageEntry] = []
        var newIndex = 0
        for (position, entry) in entries.enumerated() {
            if exists(entry.page) && kept.last?.page != entry.page {
                kept.append(entry)
            }
            if position <= index, !kept.isEmpty {
                newIndex = kept.count - 1
            }
        }
        if kept.isEmpty {
            kept = [PageEntry(.home)]
            newIndex = 0
        }
        entries = kept
        index = min(newIndex, kept.count - 1)
    }

    private mutating func trim() {
        let extra = entries.count - Self.limit
        guard extra > 0 else { return }
        entries.removeFirst(extra)
        index -= extra
    }
}

/// Where the pages of a history go across a window (wiki: Sliding-Panes §1, §3): one page fills
/// it; in two panes the current page is on the right and the one before on the left, each half
/// wide. A move slides them a pane at a time, and between a history of one page and two panes
/// the first page narrows into the left pane (or widens back) as the next slides in (or out).
public struct PaneLayout: Equatable, Sendable {
    /// The window's width.
    public var width: Double
    /// Two panes show once the history holds more than one page.
    public var twoUp: Bool

    public init(width: Double, twoUp: Bool) {
        self.width = width
        self.twoUp = twoUp
    }

    /// How far the pages slide for one step back or forward, which a swipe follows.
    public var step: Double { twoUp ? width / 2 : width }

    /// The left edge and width of the page at `position` with the history at `index`.
    public func place(_ position: Int, at index: Int) -> (x: Double, width: Double) {
        guard twoUp else { return (Double(position - index) * width, width) }
        let half = width / 2
        guard index > 0 else {
            // Alone it fills the window; the pages after it wait past the right edge.
            return position <= 0 ? (Double(position) * half, width) : (width + Double(position - 1) * half, half)
        }
        return (Double(position - index + 1) * half, half)
    }

    /// The same, with the pages dragged `drag` across towards `target` (the page before or
    /// after), a step being `step`: the frames go part of the way there. With nowhere to go
    /// (the ends of the history), they move with the drag as they are.
    public func place(_ position: Int, at index: Int, dragged drag: Double, towards target: Int?) -> (x: Double, width: Double) {
        let from = place(position, at: index)
        guard let target, drag != 0 else { return (from.x + drag, from.width) }
        let to = place(position, at: target)
        let t = min(1, abs(drag) / step)
        return (from.x + (to.x - from.x) * t, from.width + (to.width - from.width) * t)
    }
}
