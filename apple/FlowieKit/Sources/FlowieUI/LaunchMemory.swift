import Foundation

/// What the app remembers between launches, and the launch rule (wiki: iOS-Cross-Platform §4.3):
/// each window comes back to its history of pages, scrolled as they were (wiki: Sliding-Panes
/// §1), unless the date changed or it was away for over an hour; then it opens today's journal.
/// A new user starts on Home.
public enum LaunchMemory {
    /// Where it is kept (tests use their own).
    public static var defaults: UserDefaults = .standard

    /// The screenshots' launch arguments (`start`).
    private static let tabKey = "launch.tab"
    private static let pathKey = "launch.path"
    private static let activeKey = "launch.active"
    private static let dayKey = "launch.day"
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

    /// Where a window was: its tab (iPhone and narrow windows), its history of pages (wiki:
    /// Sliding-Panes §1, each page with how it was scrolled and where the caret was), and whether
    /// it was turned to one pane in landscape.
    public struct Place: Codable, Equatable {
        public var tab: String
        public var history: PageHistory
        public var singlePane: Bool

        public init(tab: String, history: PageHistory, singlePane: Bool = false) {
            self.tab = tab
            self.history = history
            self.singlePane = singlePane
        }
    }

    /// What to open.
    public enum Start: Equatable {
        /// Nothing remembered: a new user, on Home.
        case new
        /// Today's journal.
        case today
        /// Back where the window was.
        case place(Place)
    }

    /// What is kept for each window (by its scene's id): where it was, and when it was left.
    private struct Saved: Codable {
        var place: Place
        var active: Double
        var day: String
    }

    private static let windowsKey = "launch.windows"
    private static let lastWindowKey = "launch.window"

    private static var saved: [String: Saved] {
        guard let data = defaults.data(forKey: windowsKey) else { return [:] }
        return (try? JSONDecoder().decode([String: Saved].self, from: data)) ?? [:]
    }

    /// The launch rule, for a window: its own place, or for a window not seen before (or whose
    /// id the system forgot) the last one left. The App Store screenshots give a tab and the
    /// pages opened on it as launch arguments (`-launch.tab home -launch.path '(id1,id2)'`, ""
    /// for Home), which win.
    public static func start(window: String = "", at now: Date = now) -> Start {
        let windows = saved
        let record = windows[window] ?? defaults.string(forKey: lastWindowKey).flatMap { windows[$0] }
        let arguments = defaults.volatileDomain(forName: UserDefaults.argumentDomain)
        let active: Double
        let day: String?
        let place: Place
        if let tab = arguments[tabKey] as? String {
            var history = PageHistory((AppTabRoot(rawValue: tab) ?? .home).page)
            for id in arguments[pathKey] as? [String] ?? [] {
                history.push(PageEntry(id.isEmpty ? .home : .bullet(id)))
            }
            place = Place(tab: tab, history: history)
            active = defaults.double(forKey: activeKey)
            day = defaults.string(forKey: dayKey)
        } else if let record {
            place = record.place
            active = record.active
            day = record.day
        } else {
            return .new
        }
        if day != self.day(of: now) || now.timeIntervalSince1970 - active > away {
            return .today
        }
        return .place(place)
    }

    /// Remember where a window is, leaving it now.
    public static func leave(_ place: Place, window: String = "", at now: Date = now) {
        var windows = saved
        windows[window] = Saved(place: place, active: now.timeIntervalSince1970, day: day(of: now))
        // Windows not left for a week are forgotten.
        windows = windows.filter { now.timeIntervalSince1970 - $0.value.active < 7 * 24 * 3600 }
        if let data = try? JSONEncoder().encode(windows) {
            defaults.set(data, forKey: windowsKey)
        }
        defaults.set(window, forKey: lastWindowKey)
    }

    /// The first page of a tab, by the tab's name, for the screenshots' launch arguments.
    private enum AppTabRoot: String {
        case today, home, stars, search

        var page: PageRef {
            switch self {
            case .today: return .day(LaunchMemory.today)
            case .home: return .home
            case .stars: return .stars
            case .search: return .search("")
            }
        }
    }

    /// Whether a bullet has been opened as a page once: Home's hint for new users goes then.
    public static var zoomedOnce: Bool {
        get { defaults.bool(forKey: zoomedKey) }
        set { defaults.set(newValue, forKey: zoomedKey) }
    }
}
