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
    /// and next day, so turning days doesn't fill the history. How it was scrolled goes with it.
    public mutating func replace(at position: Int, with page: PageRef) {
        guard entries.indices.contains(position), entries[position].page != page else { return }
        entries[position].page = page
        entries[position].reveal = nil
        entries[position].topRow = nil
        entries[position].offset = 0
        entries[position].focus = nil
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
