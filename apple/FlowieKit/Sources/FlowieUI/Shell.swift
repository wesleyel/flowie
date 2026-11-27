#if canImport(UIKit)
import SwiftUI
import UIKit
import FlowieFFI

/// The app's tabs (wiki: iOS-Cross-Platform §4.3).
public enum AppTab: String, Hashable, CaseIterable {
    case today, home, stars, search

    /// The page a tab starts on.
    var root: PageRef {
        switch self {
        case .today: return .day(LaunchMemory.today)
        case .home: return .home
        case .stars: return .stars
        case .search: return .search("")
        }
    }
}

/// Which tab shows, and each tab's history (wiki: Sliding-Panes §1): a browser's Back and
/// Forward, as the desktop's.
@MainActor
final class Navigation: ObservableObject {
    @Published var tab: AppTab = .home
    @Published private(set) var histories: [AppTab: PageHistory] = Dictionary(uniqueKeysWithValues: AppTab.allCases.map { ($0, PageHistory($0.root)) })
    weak var store: OutlineStore?

    func history(_ tab: AppTab) -> PageHistory {
        histories[tab] ?? PageHistory(tab.root)
    }

    private func change(_ tab: AppTab, _ body: (inout PageHistory) -> Void) {
        var history = history(tab)
        body(&history)
        histories[tab] = history
    }

    /// Open a page from the one at `position` of a tab's history.
    func open(_ entry: PageEntry, from position: Int, in tab: AppTab) {
        change(tab) { $0.open(entry, from: position) }
    }

    /// Show another page in an entry without a new one (the journal's days).
    func replace(at position: Int, with page: PageRef, in tab: AppTab) {
        change(tab) { $0.replace(at: position, with: page) }
    }

    /// Pages whose bullet is gone are left out first, as the desktop's Back skips them.
    private func prune(_ history: inout PageHistory) {
        guard let store else { return }
        history.prune { page in
            if case .bullet(let id) = page { return store.bullet(id) != nil }
            return true
        }
    }

    var canGoBack: Bool { history(tab).canGoBack }
    var canGoForward: Bool { history(tab).canGoForward }

    func back() {
        change(tab) { history in
            prune(&history)
            history.goBack()
        }
    }

    func forward() {
        change(tab) { history in
            prune(&history)
            history.goForward()
        }
    }

    /// Go to a page of the history, from the back or forward button's menu.
    func go(to position: Int) {
        change(tab) { history in
            let id = history.entries[min(position, history.entries.count - 1)].id
            prune(&history)
            history.go(to: history.position(of: id) ?? history.index)
        }
    }

    /// The page above the current one (⌘[, the desktop's Zoom Out), its row pointed out.
    func zoomOut() {
        let history = history(tab)
        guard case .bullet(let id) = history.current.page, let bullet = store?.bullet(id) else { return }
        open(PageEntry(bullet.path.last.map { .bullet($0.id) } ?? .home, opener: id), from: history.index, in: tab)
    }

    /// What a page of the history is called, in the back and forward buttons' menus.
    func title(_ page: PageRef) -> String {
        switch page {
        case .home: return NSLocalizedString("Home", comment: "Title of the outline's top page")
        case .bullet(let id): return store?.bullet(id)?.title ?? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text")
        case .day(let day): return DayNames.title(day)
        case .stars: return NSLocalizedString("Stars", comment: "Title of the Stars tab")
        case .search(let query):
            return query.isEmpty ? NSLocalizedString("Search", comment: "Title of the Search tab") : query
        }
    }

    /// The navigation stack of a tab: the pages after its first, up to the current one. Popping
    /// it (not done by the stack itself: its back button is ours) goes back.
    func path(_ tab: AppTab) -> [UUID] {
        let history = history(tab)
        return history.index == 0 ? [] : history.entries[1...history.index].map(\.id)
    }

    func binding(_ tab: AppTab) -> Binding<[UUID]> {
        Binding(get: { self.path(tab) }, set: { path in
            if path.count < self.path(tab).count {
                self.change(tab) { $0.go(to: path.count) }
            }
        })
    }

    /// Search for `query` in the Search tab (a tapped tag or date, as on the desktop).
    func showSearch(_ query: String) {
        histories[.search] = PageHistory(.search(query))
        tab = .search
    }

    /// A day of the journal, at the top of the Today tab.
    func showDay(_ day: String) {
        histories[.today] = PageHistory(.day(day))
        tab = .today
    }

    /// Today's journal, at the top of the Today tab.
    func showToday() {
        showDay(LaunchMemory.today)
    }

    /// Where the app is, to remember: the tab and the pages after its first ("" for Home).
    var place: LaunchMemory.Place {
        let history = history(tab)
        let path = history.entries[...history.index].dropFirst().compactMap { entry -> String? in
            switch entry.page {
            case .home: return ""
            case .bullet(let id): return id
            default: return nil
            }
        }
        return LaunchMemory.Place(tab: tab.rawValue, path: Array(path))
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
            // Pages deleted since are left off (and what was opened after them).
            var history = PageHistory(tab.root)
            for id in place.path {
                if id.isEmpty {
                    history.push(PageEntry(.home))
                } else if store.bullet(id) != nil {
                    history.push(PageEntry(.bullet(id)))
                } else {
                    break
                }
            }
            histories[tab] = history
            self.tab = tab
            LaunchMemory.armScroll()
        }
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
            Tab("Today", systemImage: "sun.max", value: AppTab.today) { stack(.today) }
            Tab("Home", systemImage: "house", value: AppTab.home) { stack(.home) }
            Tab("Stars", systemImage: "star", value: AppTab.stars) { stack(.stars) }
            Tab("Search", systemImage: "magnifyingglass", value: AppTab.search, role: .search) { stack(.search) }
        }
        .tabViewStyle(.sidebarAdaptable)
        .tabBarMinimizeBehavior(.onScrollDown)
        .sheet(isPresented: $settings.value) {
            SettingsView(store: store, showsSelfHosted: showsSelfHosted) { settings.value = false }
                .preferredColorScheme(scheme)
        }
        .preferredColorScheme(scheme)
        // Back, Forward and Zoom Out in the menu bar and with their keys (FlowieCommands).
        .focusedSceneObject(nav)
        .onAppear {
            guard !started.value else { return }
            started.value = true
            nav.store = store
            nav.start(LaunchMemory.start(), store: store)
            #if DEBUG
            ScreenshotArguments.apply(to: nav, store: store)
            #endif
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
        nav.showDay(capture.day)
    }

    private var scheme: ColorScheme? {
        switch appearance {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    private func stack(_ tab: AppTab) -> some View {
        NavigationStack(path: nav.binding(tab)) {
            pane(tab, at: 0)
                .navigationDestination(for: UUID.self) { id in
                    if let position = nav.history(tab).position(of: id) {
                        pane(tab, at: position)
                    }
                }
        }
    }

    private func pane(_ tab: AppTab, at position: Int) -> some View {
        let history = nav.history(tab)
        let entry = history.entries[min(position, history.entries.count - 1)]
        return PaneContent(store: store, nav: nav, tab: tab, position: position, entry: entry,
                           onSettings: tab == .home && position == 0 ? { settings.value = true } : nil)
    }
}

/// The page of an entry of a tab's history, with the history's back and forward buttons.
struct PaneContent: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    let tab: AppTab
    let position: Int
    let entry: PageEntry
    let onSettings: (() -> Void)?

    var body: some View {
        Group {
            switch entry.page {
            case .home, .bullet, .day:
                PageScreen(store: store, entry: entry, nav: nav, tab: tab, position: position, top: position == 0, onSettings: onSettings)
            case .stars:
                StarsScreen(store: store, nav: nav, tab: tab, position: position)
            case .search(let query):
                SearchScreen(store: store, nav: nav, tab: tab, position: position, query: query)
            }
        }
        // Another day of the journal is another page; a search stays one while it is typed.
        .id("\(entry.id) \(entry.page.identity)")
        .navigationBarBackButtonHidden(true)
        .toolbar { HistoryButtons(nav: nav, history: nav.history(tab)) }
    }
}

/// Back and Forward at the top left, each listing the history its way when held, as in Safari
/// (wiki: Sliding-Panes §4): for those who don't swipe.
struct HistoryButtons: ToolbarContent {
    let nav: Navigation
    let history: PageHistory

    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .topBarLeading) {
            if history.canGoBack {
                Menu {
                    list(history.backList)
                } label: {
                    Label(NSLocalizedString("Back", comment: "Goes back to the page before in the history"), systemImage: "chevron.backward")
                } primaryAction: {
                    nav.back()
                }
            }
            if history.canGoForward {
                Menu {
                    list(history.forwardList)
                } label: {
                    Label(NSLocalizedString("Forward", comment: "Goes forward to the next page in the history"), systemImage: "chevron.forward")
                } primaryAction: {
                    nav.forward()
                }
            }
        }
    }

    @ViewBuilder
    private func list(_ items: [(position: Int, entry: PageEntry)]) -> some View {
        ForEach(items, id: \.entry.id) { item in
            Button(nav.title(item.entry.page), systemImage: Self.symbol(item.entry.page)) {
                nav.go(to: item.position)
            }
        }
    }

    static func symbol(_ page: PageRef) -> String {
        switch page {
        case .home: return "house"
        case .bullet: return "circle.fill"
        case .day: return "sun.max"
        case .stars: return "star"
        case .search: return "magnifyingglass"
        }
    }
}

/// The history's keys, as on the desktop (`keymap.rs`), in the iPad menu bar and the list shown
/// while ⌘ is held: ⌥⌘← Back, ⌥⌘→ Forward, ⌘[ Zoom Out (⌘] Zoom In is the edited row's).
public struct FlowieCommands: Commands {
    @FocusedObject private var nav: Navigation?

    public init() {}

    public var body: some Commands {
        CommandMenu(Text("Go", comment: "Menu bar menu with Back, Forward and Zoom Out")) {
            Button(NSLocalizedString("Back", comment: "Goes back to the page before in the history")) { nav?.back() }
                .keyboardShortcut(.leftArrow, modifiers: [.command, .option])
                .disabled(!(nav?.canGoBack ?? false))
            Button(NSLocalizedString("Forward", comment: "Goes forward to the next page in the history")) { nav?.forward() }
                .keyboardShortcut(.rightArrow, modifiers: [.command, .option])
                .disabled(!(nav?.canGoForward ?? false))
            Button(NSLocalizedString("Zoom Out", comment: "Opens the page above the current one")) { nav?.zoomOut() }
                .keyboardShortcut("[", modifiers: .command)
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
    /// Where its entry is in the tab's history.
    let position: Int
    /// Whether this is a top page of its tab (not opened from another).
    let top: Bool
    let onSettings: (() -> Void)?

    init(store: OutlineStore, entry: PageEntry, nav: Navigation, tab: AppTab, position: Int, top: Bool = false, onSettings: (() -> Void)? = nil) {
        self.store = store
        _page = StateObject(wrappedValue: {
            let page = store.page(entry.page.kind ?? .home)
            page.reveal = entry.reveal
            return page
        }())
        self.nav = nav
        self.tab = tab
        self.position = position
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
        OutlinePage(page: page, open: { id, opener in
            nav.open(PageEntry(.bullet(id), opener: opener), from: position, in: tab)
        }, search: { query in nav.showSearch(query) })
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
                            nav.replace(at: position, with: .day(LaunchMemory.day(day, offset: -1)), in: tab)
                        }
                        Button(NSLocalizedString("Next Day", comment: "Journal toolbar button"), systemImage: "chevron.right") {
                            nav.replace(at: position, with: .day(LaunchMemory.day(day, offset: 1)), in: tab)
                        }
                    }
                }
                if store.readingMode {
                    // Reading mode is on (for the whole app): tapping turns it off.
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(NSLocalizedString("Turn Off Reading Mode", comment: "Navigation bar button shown while reading mode is on"), systemImage: "book") {
                            store.setReadingMode(false)
                        }
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    PageMenu(store: store, page: page) { day in
                        nav.replace(at: position, with: .day(day), in: tab)
                    }
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
/// reading mode, fold the page to a level, copy link. `showDay`: show another day in its place.
struct PageMenu: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var page: PageModel
    let showDay: (String) -> Void

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
            Toggle(isOn: Binding(get: { store.readingMode }, set: { store.setReadingMode($0) })) {
                Label(NSLocalizedString("Reading Mode", comment: "Page menu toggle: nothing can be edited while on"), systemImage: "book")
            }
            if page.rows.contains(where: \.hasChildren) {
                Section {
                    ForEach(FoldLevel.allCases, id: \.self) { level in
                        Button(level.title, systemImage: level.symbol) { page.expandToLevel(level.levels) }
                    }
                }
            }
            if let id = page.root {
                Button(NSLocalizedString("Copy Link", comment: "Menu item"), systemImage: "link") {
                    if let link = store.link(id) { UIPasteboard.general.string = link }
                }
            }
            if case .day(let day) = page.kind, day != LaunchMemory.today {
                Button(NSLocalizedString("Go to Today", comment: "Menu item"), systemImage: "sun.max") {
                    showDay(LaunchMemory.today)
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
    let tab: AppTab
    let position: Int

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
                            nav.open(PageEntry(.bullet(bullet.id), opener: bullet.id), from: position, in: tab)
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
    let tab: AppTab
    let position: Int
    @StateObject private var query: Local<String>

    init(store: OutlineStore, nav: Navigation, tab: AppTab, position: Int, query: String) {
        self.store = store
        self.nav = nav
        self.tab = tab
        self.position = position
        _query = StateObject(wrappedValue: Local(query))
    }

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
                        // The page it is on, pointing it out.
                        nav.open(PageEntry(bullet.path.last.map { .bullet($0.id) } ?? .home, opener: bullet.id, reveal: bullet.id), from: position, in: tab)
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
        // The history keeps what was searched for, to come back to.
        .onChange(of: query.value) { _, value in
            nav.replace(at: position, with: .search(value), in: tab)
        }
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
                        // Reading mode: nothing is edited, restoring included.
                        .disabled(store.readingMode)
                    }
                    .swipeActions {
                        Button(NSLocalizedString("Restore", comment: "Brings back a deleted bullet"), systemImage: "arrow.uturn.backward") {
                            store.restore(entry.id)
                        }
                        .tint(.accentColor)
                        .disabled(store.readingMode)
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
