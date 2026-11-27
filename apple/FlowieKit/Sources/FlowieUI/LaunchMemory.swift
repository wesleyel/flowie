import Foundation

/// What the app remembers between launches, and the launch rule (wiki: iOS-Cross-Platform §4.3):
/// come back to the last page and scroll position, unless the date changed or the app was away
/// for over an hour; then open today's journal. A new user starts on Home.
public enum LaunchMemory {
    /// Where it is kept (tests use their own).
    public static var defaults: UserDefaults = .standard

    private static let tabKey = "launch.tab"
    private static let pathKey = "launch.path"
    private static let activeKey = "launch.active"
    private static let dayKey = "launch.day"
    private static let scrollKey = "launch.scroll"
    private static let zoomedKey = "hint.zoomed"
    /// How long the app can be away and still come back where it was.
    public static let away: TimeInterval = 3600

    /// The time now. Debug builds can be given another one, to check the launch rule: launch
    /// with `-FlowieNow 2026-11-28T09:00:00+08:00`.
    public static var now: Date {
        #if DEBUG
        if let text = defaults.string(forKey: "FlowieNow"), let date = ISO8601DateFormatter().date(from: text) {
            return date
        }
        #endif
        return Date()
    }

    private static func formatter() -> DateFormatter {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .current
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter
    }

    /// A day as journal pages are named (YYYY-MM-DD), in this device's time zone.
    public static func day(of date: Date) -> String {
        formatter().string(from: date)
    }

    /// Today, as journal pages are named.
    public static var today: String { day(of: now) }

    /// The day `offset` days from `day` (YYYY-MM-DD).
    public static func day(_ day: String, offset: Int) -> String {
        let formatter = formatter()
        guard let date = formatter.date(from: day),
              let moved = Calendar(identifier: .gregorian).date(byAdding: .day, value: offset, to: date)
        else { return day }
        return formatter.string(from: moved)
    }

    /// The date of a day (YYYY-MM-DD), at its start.
    public static func date(of day: String) -> Date? {
        formatter().date(from: day)
    }

    /// Where the app was: its tab and the pages pushed on it (bullet ids; "" for Home).
    public struct Place: Equatable {
        public var tab: String
        public var path: [String]

        public init(tab: String, path: [String]) {
            self.tab = tab
            self.path = path
        }
    }

    /// What to open.
    public enum Start: Equatable {
        /// Nothing remembered: a new user, on Home.
        case new
        /// Today's journal.
        case today
        /// Back where the app was.
        case place(Place)
    }

    /// The launch rule.
    public static func start(at now: Date = now) -> Start {
        guard let tab = defaults.string(forKey: tabKey) else { return .new }
        let active = defaults.double(forKey: activeKey)
        let day = defaults.string(forKey: dayKey)
        if day != self.day(of: now) || now.timeIntervalSince1970 - active > away {
            return .today
        }
        return .place(Place(tab: tab, path: defaults.stringArray(forKey: pathKey) ?? []))
    }

    /// Remember where the app is, leaving it now.
    public static func leave(_ place: Place, at now: Date = now) {
        defaults.set(place.tab, forKey: tabKey)
        defaults.set(place.path, forKey: pathKey)
        defaults.set(now.timeIntervalSince1970, forKey: activeKey)
        defaults.set(day(of: now), forKey: dayKey)
    }

    // MARK: Scroll position

    /// The scroll position to come back to, once, at launch.
    private static var pendingScroll: (page: String, row: String)?

    /// The top row on screen of a page (by `PageKind.key`), remembered as it is scrolled.
    public static func rememberScroll(_ page: String, row: String?) {
        if let row {
            defaults.set([page, row], forKey: scrollKey)
        } else {
            defaults.removeObject(forKey: scrollKey)
        }
    }

    /// Come back to the remembered scroll position when its page next shows (at launch).
    public static func armScroll() {
        if let saved = defaults.stringArray(forKey: scrollKey), saved.count == 2 {
            pendingScroll = (saved[0], saved[1])
        }
    }

    /// The row to scroll to on this page, if it is the one remembered at launch (once).
    public static func takeScroll(for page: String) -> String? {
        guard let pending = pendingScroll, pending.page == page else { return nil }
        pendingScroll = nil
        return pending.row
    }

    /// Whether a bullet has been opened as a page once: Home's hint for new users goes then.
    public static var zoomedOnce: Bool {
        get { defaults.bool(forKey: zoomedKey) }
        set { defaults.set(newValue, forKey: zoomedKey) }
    }
}
