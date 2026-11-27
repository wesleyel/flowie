#if canImport(UIKit)
import SwiftUI
import UIKit
import FlowieFFI

/// The app's tabs (wiki: iOS-Cross-Platform §4.3).
public enum AppTab: String, Hashable {
    case today, home, stars, search
}

/// A page pushed on a tab's navigation stack: a bullet's page (nil: Home), and maybe a row on
/// it to scroll to and point out (a search result).
public struct Route: Hashable {
    public var id: String?
    public var reveal: String?

    public init(id: String?, reveal: String? = nil) {
        self.id = id
        self.reveal = reveal
    }

    /// The pages down to `bullet`'s own page, each pushed in turn, so Back goes up one level
    /// and holding Back lists the whole path.
    static func into(_ bullet: BulletView) -> [Route] {
        bullet.path.map { Route(id: $0.id) } + [Route(id: bullet.id)]
    }

    /// The pages down to the page `bullet` is on, pointing it out there.
    static func toward(_ bullet: BulletView) -> [Route] {
        var routes = bullet.path.map { Route(id: $0.id) }
        if routes.isEmpty {
            routes = [Route(id: nil)]
        }
        routes[routes.count - 1].reveal = bullet.id
        return routes
    }
}

/// Which tab shows, each tab's navigation stack, and the day the Today tab shows.
@MainActor
final class Navigation: ObservableObject {
    @Published var tab: AppTab = .home
    @Published var today: [Route] = []
    @Published var home: [Route] = []
    @Published var stars: [Route] = []
    @Published var search: [Route] = []
    /// The day the Today tab shows (YYYY-MM-DD).
    @Published var day = LaunchMemory.today
    /// What the Search tab searches for.
    let query = Local("")

    func path(_ tab: AppTab) -> [Route] {
        switch tab {
        case .today: return today
        case .home: return home
        case .stars: return stars
        case .search: return search
        }
    }

    func setPath(_ path: [Route], for tab: AppTab) {
        switch tab {
        case .today: today = path
        case .home: home = path
        case .stars: stars = path
        case .search: search = path
        }
    }

    func binding(_ tab: AppTab) -> Binding<[Route]> {
        Binding(get: { self.path(tab) }, set: { self.setPath($0, for: tab) })
    }

    func push(_ route: Route, in tab: AppTab) {
        setPath(path(tab) + [route], for: tab)
    }

    func push(_ routes: [Route], in tab: AppTab) {
        setPath(path(tab) + routes, for: tab)
    }

    /// Search for `query` in the Search tab (a tapped tag or date, as on the desktop).
    func showSearch(_ query: String) {
        self.query.value = query
        search = []
        tab = .search
    }

    /// Where the app is, to remember.
    var place: LaunchMemory.Place {
        LaunchMemory.Place(tab: tab.rawValue, path: path(tab).map { $0.id ?? "" })
    }

    /// Open what the launch rule says (§4.3), the outline checked for pages since gone.
    func start(_ start: LaunchMemory.Start, store: OutlineStore) {
        switch start {
        case .new:
            tab = .home
        case .today:
            showToday()
        case .place(let place):
            let tab = AppTab(rawValue: place.tab) ?? .home
            // Pages deleted since are left off (and what was pushed after them).
            var routes: [Route] = []
            for id in place.path {
                if id.isEmpty {
                    routes.append(Route(id: nil))
                } else if store.bullet(id) != nil {
                    routes.append(Route(id: id))
                } else {
                    break
                }
            }
            self.tab = tab
            setPath(routes, for: tab)
            day = LaunchMemory.today
            LaunchMemory.armScroll()
        }
    }

    /// Today's journal, at the top of the Today tab.
    func showToday() {
        day = LaunchMemory.today
        today = []
        tab = .today
    }
}

/// The app's shell: Today, Home and Stars tabs, and Search at the right (§4.3); settings in a
/// sheet from Home. `showsSelfHosted`: debug and TestFlight builds, which keep the self-hosted
/// sync settings (App Store builds sync only through iCloud).
public struct FlowieShell: View {
    @ObservedObject var store: OutlineStore
    let showsSelfHosted: Bool
    @StateObject private var nav = Navigation()
    @StateObject private var settings = Local(false)
    @StateObject private var started = Local(false)
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage("appearance") private var appearance = "system"

    public init(store: OutlineStore, showsSelfHosted: Bool) {
        self.store = store
        self.showsSelfHosted = showsSelfHosted
    }

    public var body: some View {
        TabView(selection: $nav.tab) {
            Tab("Today", systemImage: "sun.max", value: AppTab.today) {
                stack(.today) { TodayScreen(store: store, nav: nav) }
            }
            Tab("Home", systemImage: "house", value: AppTab.home) {
                stack(.home) {
                    PageScreen(store: store, kind: .home, nav: nav, tab: .home, top: true, onSettings: { settings.value = true })
                }
            }
            Tab("Stars", systemImage: "star", value: AppTab.stars) {
                stack(.stars) { StarsScreen(store: store, nav: nav) }
            }
            Tab("Search", systemImage: "magnifyingglass", value: AppTab.search, role: .search) {
                stack(.search) { SearchScreen(store: store, nav: nav, query: nav.query) }
            }
        }
        .tabViewStyle(.sidebarAdaptable)
        .tabBarMinimizeBehavior(.onScrollDown)
        .sheet(isPresented: $settings.value) {
            SettingsView(store: store, showsSelfHosted: showsSelfHosted) { settings.value = false }
                .preferredColorScheme(scheme)
        }
        .preferredColorScheme(scheme)
        .onAppear {
            guard !started.value else { return }
            started.value = true
            nav.start(LaunchMemory.start(), store: store)
            showCapture()
        }
        .onChange(of: store.capture) { _, _ in showCapture() }
        .onChange(of: scenePhase) { old, phase in
            switch phase {
            case .background:
                LaunchMemory.leave(nav.place)
            case .active where old == .background:
                // Back after a while, or on another day: today's journal (§4.3).
                if case .today = LaunchMemory.start() {
                    nav.showToday()
                }
            default:
                break
            }
        }
    }

    /// A capture was asked for (§5): its day at the top of the Today tab, where the page puts
    /// the caret on its row.
    private func showCapture() {
        guard started.value, let capture = store.capture else { return }
        settings.value = false
        nav.day = capture.day
        nav.today = []
        nav.tab = .today
    }

    private var scheme: ColorScheme? {
        switch appearance {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    private func stack<Root: View>(_ tab: AppTab, @ViewBuilder root: () -> Root) -> some View {
        NavigationStack(path: nav.binding(tab)) {
            root().navigationDestination(for: Route.self) { route in
                PageScreen(store: store, kind: route.id.map { .bullet($0) } ?? .home, reveal: route.reveal, nav: nav, tab: tab)
            }
        }
    }
}

// MARK: Pages

/// A page of the outline with its title, the path to it, and its "…" menu (§4.3, §4.4). Top
/// pages (Today, Home) have large titles; pages zoomed into have a small one, the bullet's text,
/// with the path to it below, cut in the middle.
struct PageScreen: View {
    @ObservedObject var store: OutlineStore
    @StateObject private var page: PageModel
    @ObservedObject var nav: Navigation
    let tab: AppTab
    /// Whether this is a top page of its tab (not pushed).
    let top: Bool
    let onSettings: (() -> Void)?

    init(store: OutlineStore, kind: PageKind, reveal: String? = nil, nav: Navigation, tab: AppTab, top: Bool = false, onSettings: (() -> Void)? = nil) {
        self.store = store
        _page = StateObject(wrappedValue: {
            let page = store.page(kind)
            page.reveal = reveal
            return page
        }())
        self.nav = nav
        self.tab = tab
        self.top = top
        self.onSettings = onSettings
    }

    private var title: String {
        switch page.kind {
        case .home: return NSLocalizedString("Home", comment: "Title of the outline's top page")
        case .bullet: return page.info?.title ?? ""
        case .day(let day): return DayNames.title(day)
        }
    }

    private var subtitle: String? {
        switch page.kind {
        case .home: return nil
        case .bullet: return top ? nil : page.info?.pathLine(limit: 40)
        case .day(let day): return DayNames.subtitle(day)
        }
    }

    var body: some View {
        OutlinePage(page: page, open: { id in nav.push(Route(id: id), in: tab) }, search: { query in nav.showSearch(query) })
            // The page keeps its rows above the keyboard itself (an inset), and
            // its list scrolls under the tab bar.
            .ignoresSafeArea(.keyboard)
            .ignoresSafeArea(.container, edges: .bottom)
            .navigationTitle(title)
            .modifier(Subtitle(text: subtitle))
            .navigationBarTitleDisplayMode(top ? .large : .inline)
            // While typing, the keyboard bar takes the bottom: with a hardware keyboard it
            // would sit on the tab bar.
            .toolbar(page.isEditing ? .hidden : .automatic, for: .tabBar)
            .toolbar {
                if case .day(let day) = page.kind {
                    ToolbarItemGroup(placement: .topBarLeading) {
                        Button(NSLocalizedString("Previous Day", comment: "Journal toolbar button"), systemImage: "chevron.left") {
                            nav.day = LaunchMemory.day(day, offset: -1)
                        }
                        Button(NSLocalizedString("Next Day", comment: "Journal toolbar button"), systemImage: "chevron.right") {
                            nav.day = LaunchMemory.day(day, offset: 1)
                        }
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    PageMenu(store: store, page: page, nav: nav)
                }
                if let onSettings {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(NSLocalizedString("Settings", comment: "Opens the settings sheet"), systemImage: "gearshape", action: onSettings)
                    }
                }
            }
            .safeAreaInset(edge: .top) {
                if let problem = store.problem {
                    Text(problem).font(.footnote).foregroundStyle(.red).padding(8)
                } else if top && store.fetchingFirst {
                    // A new install of someone with a Mac: their outline is on its way (§4.3).
                    HStack(spacing: 8) {
                        ProgressView()
                        Text("Syncing your outline from iCloud…")
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .padding(8)
                }
            }
    }
}

/// `navigationSubtitle`, when there is one.
private struct Subtitle: ViewModifier {
    let text: String?

    func body(content: Content) -> some View {
        if let text {
            content.navigationSubtitle(text)
        } else {
            content
        }
    }
}

/// A page's "…" menu, the only button at its top right (§4.4): star, show or hide completed,
/// copy link.
struct PageMenu: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var page: PageModel
    @ObservedObject var nav: Navigation

    var body: some View {
        Menu {
            if let info = page.info {
                Button(
                    info.starred ? NSLocalizedString("Unstar", comment: "Menu item") : NSLocalizedString("Star", comment: "Menu item"),
                    systemImage: info.starred ? "star.slash" : "star"
                ) {
                    store.setStarred(info.id, !info.starred)
                }
            }
            Button(
                store.hideCompleted ? NSLocalizedString("Show Completed", comment: "Menu item") : NSLocalizedString("Hide Completed", comment: "Menu item"),
                systemImage: store.hideCompleted ? "eye" : "eye.slash"
            ) {
                store.setHideCompleted(!store.hideCompleted)
            }
            if let id = page.root {
                Button(NSLocalizedString("Copy Link", comment: "Menu item"), systemImage: "link") {
                    if let link = store.link(id) { UIPasteboard.general.string = link }
                }
            }
            if case .day(let day) = page.kind, day != LaunchMemory.today {
                Button(NSLocalizedString("Go to Today", comment: "Menu item"), systemImage: "sun.max") {
                    nav.day = LaunchMemory.today
                }
            }
        } label: {
            Label(NSLocalizedString("More", comment: "The page menu's button"), systemImage: "ellipsis")
        }
    }
}

/// What the journal's days are called.
enum DayNames {
    static func title(_ day: String) -> String {
        let today = LaunchMemory.today
        switch day {
        case today: return NSLocalizedString("Today", comment: "Title of today's journal page")
        case LaunchMemory.day(today, offset: -1): return NSLocalizedString("Yesterday", comment: "Title of yesterday's journal page")
        case LaunchMemory.day(today, offset: 1): return NSLocalizedString("Tomorrow", comment: "Title of tomorrow's journal page")
        default:
            guard let date = LaunchMemory.date(of: day) else { return day }
            return date.formatted(.dateTime.month(.abbreviated).day().weekday(.abbreviated))
        }
    }

    static func subtitle(_ day: String) -> String? {
        guard let date = LaunchMemory.date(of: day) else { return nil }
        return date.formatted(date: .complete, time: .omitted)
    }
}

/// The Today tab: a day of the journal (today at first), the days before and after a tap away.
struct TodayScreen: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation

    var body: some View {
        PageScreen(store: store, kind: .day(nav.day), nav: nav, tab: .today, top: true)
            .id(nav.day)
    }
}

// MARK: Lists

/// A bullet in a list: its text and the path to it.
struct BulletLine: View {
    let bullet: BulletView

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: bullet.title)
                .strikethrough(bullet.completed)
                .foregroundStyle(bullet.completed ? .secondary : .primary)
                .lineLimit(2)
            Text(verbatim: bullet.pathLine())
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

/// The Stars tab: the starred bullets; tapping one opens its page.
struct StarsScreen: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation

    var body: some View {
        let stars = store.starred()
        Group {
            if stars.isEmpty {
                ContentUnavailableView(
                    NSLocalizedString("No Stars Yet", comment: "Stars tab when empty"),
                    systemImage: "star",
                    description: Text("Star a page from its … menu, or hold a bullet to star it.")
                )
            } else {
                List {
                    ForEach(stars, id: \.id) { bullet in
                        Button {
                            nav.push(Route.into(bullet), in: .stars)
                        } label: {
                            BulletLine(bullet: bullet)
                        }
                        .foregroundStyle(.primary)
                        .swipeActions {
                            Button(NSLocalizedString("Unstar", comment: "Menu item"), systemImage: "star.slash", role: .destructive) {
                                store.setStarred(bullet.id, false)
                            }
                        }
                    }
                }
                .listStyle(.plain)
            }
        }
        .navigationTitle(NSLocalizedString("Stars", comment: "Title of the Stars tab"))
        .navigationBarTitleDisplayMode(.large)
    }
}

/// The Search tab: search with the desktop's filter syntax; tapping a result opens the page
/// it is on, pointing it out.
struct SearchScreen: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    @ObservedObject var query: Local<String>

    /// Filters to start from: what to type, and what it finds.
    private let filters: [(String, String)] = [
        ("#", NSLocalizedString("#tag: bullets with a tag", comment: "Search filter help")),
        ("is:open ", NSLocalizedString("is:open: not completed", comment: "Search filter help")),
        ("is:done ", NSLocalizedString("is:done: completed", comment: "Search filter help")),
        ("has:note ", NSLocalizedString("has:note: with a note", comment: "Search filter help")),
        ("date:today ", NSLocalizedString("date:today: mentioning today’s date", comment: "Search filter help")),
        ("changed:7d ", NSLocalizedString("changed:7d: changed in the last 7 days", comment: "Search filter help")),
        ("-", NSLocalizedString("-word: without a word", comment: "Search filter help")),
        ("\"", NSLocalizedString("\"a phrase\": the words together", comment: "Search filter help")),
    ]

    var body: some View {
        let text = query.value.trimmingCharacters(in: .whitespaces)
        let hits = text.isEmpty ? [] : store.search(query.value)
        List {
            if text.isEmpty {
                Section(NSLocalizedString("Filters", comment: "Search: heading of the filter list")) {
                    ForEach(filters, id: \.0) { insert, help in
                        Button {
                            query.value += (query.value.isEmpty || query.value.hasSuffix(" ") ? "" : " ") + insert
                        } label: {
                            Text(verbatim: help).foregroundStyle(.primary)
                        }
                    }
                }
            } else {
                ForEach(hits, id: \.id) { bullet in
                    Button {
                        nav.push(Route.toward(bullet), in: .search)
                    } label: {
                        BulletLine(bullet: bullet)
                    }
                    .foregroundStyle(.primary)
                }
            }
        }
        .listStyle(.plain)
        .overlay {
            if !text.isEmpty && hits.isEmpty {
                ContentUnavailableView.search(text: text)
            }
        }
        .navigationTitle(NSLocalizedString("Search", comment: "Title of the Search tab"))
        // Always shown: on the tab view, the search tab didn't turn the tab bar into a search
        // field (iOS 27.2 Simulator), and the field stayed hidden.
        .searchable(text: $query.value, placement: .navigationBarDrawer(displayMode: .always), prompt: Text("Search or filter"))
        .autocorrectionDisabled()
        .textInputAutocapitalization(.never)
    }
}

/// Where Move To sends a bullet: under a bullet, or to Home (nil).
struct MoveTarget {
    let id: String?
}

/// Move To (§2.1): find where a bullet goes, by name, as the desktop's palette finds bullets.
struct MoveToPicker: View {
    @ObservedObject var store: OutlineStore
    let moving: RowView
    let done: (MoveTarget?) -> Void
    @StateObject private var query = Local("")

    private var showsHome: Bool {
        let q = query.value.trimmingCharacters(in: .whitespaces).lowercased()
        let home = NSLocalizedString("Home", comment: "Title of the outline's top page").lowercased()
        return q.isEmpty || "home".hasPrefix(q) || home.contains(q)
    }

    var body: some View {
        NavigationStack {
            List {
                if showsHome {
                    Button {
                        done(MoveTarget(id: nil))
                    } label: {
                        Label(NSLocalizedString("Home", comment: "Title of the outline's top page"), systemImage: "house")
                    }
                    .foregroundStyle(.primary)
                }
                ForEach(store.quickFind(query.value, excluding: [moving.id]), id: \.id) { bullet in
                    Button {
                        done(MoveTarget(id: bullet.id))
                    } label: {
                        BulletLine(bullet: bullet)
                    }
                    .foregroundStyle(.primary)
                }
            }
            .listStyle(.plain)
            .navigationTitle(NSLocalizedString("Move To", comment: "Title of the Move To picker"))
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query.value, placement: .navigationBarDrawer(displayMode: .always), prompt: Text("Find a bullet"))
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(NSLocalizedString("Cancel", comment: "Closes a picker without choosing"), systemImage: "xmark") { done(nil) }
                }
            }
        }
    }
}

// MARK: Settings

/// Settings, in a sheet from Home (§4.4): sync, Recently Deleted, appearance, about.
struct SettingsView: View {
    @ObservedObject var store: OutlineStore
    let showsSelfHosted: Bool
    let close: () -> Void
    @AppStorage("appearance") private var appearance = "system"

    private var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(short) (\(build))"
    }

    /// iCloud sync's state, in a few words.
    private var iCloudStatus: String {
        switch store.iCloud {
        case .off: return NSLocalizedString("Off", comment: "iCloud sync status")
        case .checking: return NSLocalizedString("Checking…", comment: "iCloud sync status")
        case .syncing: return NSLocalizedString("Syncing…", comment: "iCloud sync status")
        case .synced: return NSLocalizedString("Up to Date", comment: "iCloud sync status")
        case .failed: return NSLocalizedString("Waiting to Sync", comment: "iCloud sync status")
        case .unavailable(let access):
            switch access {
            case .noAccount: return NSLocalizedString("Not Signed In", comment: "iCloud sync status")
            case .restricted: return NSLocalizedString("Restricted", comment: "iCloud sync status")
            case .notEntitled: return NSLocalizedString("Unavailable in This Build", comment: "iCloud sync status")
            default: return NSLocalizedString("Unavailable", comment: "iCloud sync status")
            }
        }
    }

    /// What iCloud sync's state means, and what to do about it.
    private var iCloudFooter: String {
        switch store.iCloud {
        case .off:
            return NSLocalizedString("Your outline syncs with your Mac through iCloud when this iPhone is signed in to iCloud.", comment: "Settings footer")
        case .checking, .syncing:
            return NSLocalizedString("Your outline syncs with your Mac through your iCloud account.", comment: "Settings footer")
        case .synced(let date):
            return String(format: NSLocalizedString("Your outline syncs with your Mac through your iCloud account. Last synced %@.", comment: "Settings footer; %@ is a time"),
                          date.formatted(date: .omitted, time: .shortened))
        case .failed(let error):
            return String(format: NSLocalizedString("Can’t reach iCloud right now (%@). Your edits are kept on this iPhone and sent when it can.", comment: "Settings footer; %@ is the error"), error)
        case .unavailable(let access):
            switch access {
            case .noAccount:
                return NSLocalizedString("Sign in to iCloud in the Settings app to sync your outline with your Mac. Your outline stays on this iPhone meanwhile.", comment: "Settings footer")
            case .restricted:
                return NSLocalizedString("iCloud is restricted on this iPhone (Screen Time or a device profile), so your outline stays on this iPhone.", comment: "Settings footer")
            case .notEntitled:
                return NSLocalizedString("This build of Flowie isn’t signed for iCloud, so your outline stays on this device.", comment: "Settings footer")
            case .unsupported:
                return NSLocalizedString("iCloud sync needs a newer version of iOS.", comment: "Settings footer")
            case .temporarilyUnavailable:
                return NSLocalizedString("iCloud is temporarily unavailable. Flowie tries again by itself.", comment: "Settings footer")
            case .failed(let error):
                return String(format: NSLocalizedString("Couldn’t check iCloud (%@). Flowie tries again by itself.", comment: "Settings footer; %@ is the error"), error)
            case .available:
                return ""
            }
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent(NSLocalizedString("iCloud Sync", comment: "Settings row")) {
                        Text(iCloudStatus).foregroundStyle(.secondary)
                    }
                    if showsSelfHosted {
                        NavigationLink(NSLocalizedString("Self-Hosted Server", comment: "Settings row")) {
                            SyncSettingsView(store: store)
                                .navigationTitle(NSLocalizedString("Self-Hosted Server", comment: "Settings row"))
                        }
                    }
                } header: {
                    Text("Sync")
                } footer: {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(iCloudFooter)
                        if showsSelfHosted {
                            Text("The self-hosted server is for testing, and isn’t in App Store builds. While it syncs, iCloud is off.")
                        }
                    }
                }
                Section {
                    NavigationLink {
                        RecentlyDeletedView(store: store)
                    } label: {
                        Label(NSLocalizedString("Recently Deleted", comment: "Settings row"), systemImage: "trash")
                    }
                }
                Section(NSLocalizedString("Appearance", comment: "Settings section")) {
                    Picker(NSLocalizedString("Appearance", comment: "Settings section"), selection: $appearance) {
                        Text("System").tag("system")
                        Text("Light").tag("light")
                        Text("Dark").tag("dark")
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                }
                Section(NSLocalizedString("About", comment: "Settings section")) {
                    LabeledContent(NSLocalizedString("Version", comment: "Settings row"), value: version)
                    Link(NSLocalizedString("Website", comment: "Settings row"), destination: URL(string: "https://wesleyel.github.io/flowie/")!)
                    Link(NSLocalizedString("Privacy Policy", comment: "Settings row"), destination: URL(string: "https://wesleyel.github.io/flowie/privacy.html")!)
                }
            }
            .navigationTitle(NSLocalizedString("Settings", comment: "Opens the settings sheet"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(NSLocalizedString("Done", comment: "Closes the settings sheet"), systemImage: "checkmark", action: close)
                }
            }
        }
    }
}

/// Recently Deleted (§2.1): what is in the Trash, to restore. Nothing is deleted for good from
/// here: entries go by themselves after 30 days.
struct RecentlyDeletedView: View {
    @ObservedObject var store: OutlineStore

    var body: some View {
        let entries = store.trashed()
        List {
            Section {
                ForEach(entries, id: \.id) { entry in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: entry.text.trimmingCharacters(in: .whitespaces).isEmpty
                                ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : entry.text)
                                .lineLimit(2)
                            Text(verbatim: detail(entry))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(NSLocalizedString("Restore", comment: "Brings back a deleted bullet")) {
                            store.restore(entry.id)
                        }
                        .buttonStyle(.borderless)
                    }
                    .swipeActions {
                        Button(NSLocalizedString("Restore", comment: "Brings back a deleted bullet"), systemImage: "arrow.uturn.backward") {
                            store.restore(entry.id)
                        }
                        .tint(.accentColor)
                    }
                }
            } footer: {
                Text("Deleted bullets are kept for 30 days.")
            }
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView(NSLocalizedString("Nothing Deleted", comment: "Recently Deleted when empty"), systemImage: "trash")
            }
        }
        .navigationTitle(NSLocalizedString("Recently Deleted", comment: "Settings row"))
    }

    private func detail(_ entry: TrashView) -> String {
        let when = RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: Double(entry.deletedAt) / 1000), relativeTo: Date())
        let count = String.localizedStringWithFormat(NSLocalizedString("%lld bullets", comment: "How many bullets a deleted entry holds"), Int(entry.count))
        return "\(count) · \(when)"
    }
}
#endif
