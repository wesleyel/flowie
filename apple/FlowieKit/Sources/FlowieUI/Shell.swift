#if canImport(UIKit)
import SwiftUI
import UIKit
import FlowieFFI

/// The app's tabs (wiki: iOS-Cross-Platform §4.3), on iPhone and in narrow iPad windows.
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

    /// The tab a page belongs in, when a wide window's history moves into the tabs.
    init(for page: PageRef) {
        switch page {
        case .day: self = .today
        case .stars: self = .stars
        case .search: self = .search
        case .home, .bullet: self = .home
        }
    }
}

/// Whose history: a tab's (iPhone, narrow iPad windows), or the window's (an iPad window wide
/// enough for the sidebar, where there are no tabs).
enum HistoryKey: Hashable {
    case tab(AppTab)
    case window
}

/// Where the shell is (wiki: Sliding-Panes): each history (a browser's Back and Forward, as the
/// desktop's), which tab shows, whether the window is wide (the sidebar instead of tabs), shows
/// two panes, and which pane has the focus.
@MainActor
final class Navigation: ObservableObject {
    @Published var tab: AppTab = .home
    @Published private(set) var histories: [HistoryKey: PageHistory] = Dictionary(uniqueKeysWithValues: AppTab.allCases.map { (.tab($0), PageHistory($0.root)) })
    /// An iPad window of regular width: one history, the sidebar drawer instead of tabs.
    @Published private(set) var wide = false
    /// The sidebar drawer is out (wide windows).
    @Published var sidebar = false
    /// The window could show two panes (landscape, wide enough), and does unless the "…" menu
    /// turned them off.
    @Published var canSplit = false
    @Published var singlePane = false
    /// The pane with the focus (its entry); nil: the current page's.
    @Published var focused: UUID?
    /// The pages are moving (dragged, or sliding after Back or Forward): the pages beside those
    /// on screen are there to slide in or out. Settled, they go (and VoiceOver can't reach them).
    @Published private(set) var moving = false
    /// The last move went back: pages come in from the left.
    private(set) var wentBack = false
    private var settle: DispatchWorkItem?
    weak var store: OutlineStore?

    /// The history the window navigates now.
    var key: HistoryKey { wide ? .window : .tab(tab) }

    /// Whether two panes show now.
    var twoPanes: Bool { wide && canSplit && !singlePane }

    func history(_ key: HistoryKey) -> PageHistory {
        if let history = histories[key] { return history }
        if case .tab(let tab) = key { return PageHistory(tab.root) }
        return PageHistory(.home)
    }

    private func change(_ key: HistoryKey, _ body: (inout PageHistory) -> Void) {
        var history = history(key)
        body(&history)
        let before = self.history(key)
        if history.index != before.index || history.current.id != before.current.id {
            wentBack = history.index < before.index
            move()
        }
        histories[key] = history
    }

    /// The pages start moving; `settling`: and stop once they have slid (the animation's length).
    func move(settling: Bool = true) {
        settle?.cancel()
        if !moving { moving = true }
        guard settling else { return }
        let task = DispatchWorkItem { [weak self] in self?.moving = false }
        settle = task
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: task)
    }

    /// The window became wide or narrow (Split View, Stage Manager): its history moves between
    /// the tab it is in and the window, so the pages shown stay.
    func setWide(_ wide: Bool) {
        guard wide != self.wide else { return }
        if wide {
            histories[.window] = history(.tab(tab))
        } else {
            let history = history(.window)
            tab = AppTab(for: history.current.page)
            histories[.tab(tab)] = history
            sidebar = false
        }
        self.wide = wide
        focused = nil
    }

    /// The first page on screen: the left pane's when two show.
    func firstShown(_ key: HistoryKey) -> Int {
        let history = history(key)
        return key == .window && twoPanes ? max(0, history.index - 1) : history.index
    }

    /// The page with the focus, by its place in the history.
    func focusedPosition(_ key: HistoryKey) -> Int {
        let history = history(key)
        if let focused, let position = history.position(of: focused), position >= firstShown(key), position <= history.index {
            return position
        }
        return history.index
    }

    /// Open a page from the one at `position` of a history (wiki: Sliding-Panes §1).
    func open(_ entry: PageEntry, from position: Int, in key: HistoryKey) {
        withAnimation(.snappy) {
            change(key) { $0.open(entry, from: position) }
            focused = nil
        }
    }

    /// Show another page in an entry without a new one (the journal's days, a search typed).
    func replace(at position: Int, with page: PageRef, in key: HistoryKey) {
        change(key) { $0.replace(at: position, with: page) }
    }

    /// Pages whose bullet is gone are left out first, as the desktop's Back skips them.
    private func prune(_ history: inout PageHistory) {
        guard let store else { return }
        history.prune { page in
            if case .bullet(let id) = page { return store.bullet(id) != nil }
            return true
        }
    }

    var canGoBack: Bool { history(key).canGoBack }
    var canGoForward: Bool { history(key).canGoForward }

    func back() {
        withAnimation(.snappy) {
            change(key) { history in
                prune(&history)
                history.goBack()
            }
            focused = nil
        }
    }

    func forward() {
        withAnimation(.snappy) {
            change(key) { history in
                prune(&history)
                history.goForward()
            }
            focused = nil
        }
    }

    /// Go to a page of the history, from the back or forward button's menu.
    func go(to position: Int) {
        withAnimation(.snappy) {
            change(key) { history in
                let id = history.entries[min(position, history.entries.count - 1)].id
                prune(&history)
                history.go(to: history.position(of: id) ?? history.index)
            }
            focused = nil
        }
    }

    /// The page above the focused one (⌘[, the desktop's Zoom Out), its row pointed out.
    func zoomOut() {
        let key = key
        let position = focusedPosition(key)
        guard case .bullet(let id) = history(key).entries[position].page, let bullet = store?.bullet(id) else { return }
        open(PageEntry(bullet.path.last.map { .bullet($0.id) } ?? .home, opener: id), from: position, in: key)
    }

    /// Give the focus to the left or the right pane (⌃⌘← ⌃⌘→).
    func focusPane(left: Bool) {
        guard twoPanes else { return }
        let history = history(.window)
        guard history.index > 0 else { return }
        focused = history.entries[left ? history.index - 1 : history.index].id
    }

    /// Open a page from the sidebar: in the right pane, as a new page.
    func openFromSidebar(_ page: PageRef) {
        sidebar = false
        let history = history(.window)
        if history.current.page == page {
            return
        }
        open(PageEntry(page), from: history.index, in: .window)
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

    /// Search for `query` (a tapped tag or date, as on the desktop): in the Search tab, or in a
    /// wide window from the pane it was tapped in.
    func showSearch(_ query: String, from position: Int? = nil) {
        if wide {
            open(PageEntry(.search(query)), from: position ?? history(.window).index, in: .window)
        } else {
            histories[.tab(.search)] = PageHistory(.search(query))
            tab = .search
        }
    }

    /// A day of the journal: at the top of the Today tab, or opened in a wide window.
    func showDay(_ day: String) {
        if wide {
            let history = history(.window)
            if history.current.page != .day(day) {
                open(PageEntry(.day(day)), from: history.index, in: .window)
            }
        } else {
            histories[.tab(.today)] = PageHistory(.day(day))
            tab = .today
        }
    }

    /// Today's journal.
    func showToday() {
        showDay(LaunchMemory.today)
    }

    /// Where the app is, to remember: the tab and the pages after its first ("" for Home).
    var place: LaunchMemory.Place {
        let history = history(key)
        let path = history.entries[...history.index].dropFirst().compactMap { entry -> String? in
            switch entry.page {
            case .home: return ""
            case .bullet(let id): return id
            default: return nil
            }
        }
        return LaunchMemory.Place(tab: AppTab(for: history.entries[0].page).rawValue, path: Array(path))
    }

    /// Open what the launch rule says (§4.3), the outline checked for pages since gone.
    func start(_ start: LaunchMemory.Start, store: OutlineStore) {
        switch start {
        case .new:
            tab = .home
            histories[.window] = PageHistory(.home)
        case .today:
            tab = .today
            histories[.window] = PageHistory(.day(LaunchMemory.today))
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
            histories[.tab(tab)] = history
            histories[.window] = history
            self.tab = tab
            LaunchMemory.armScroll()
        }
    }
}

/// The app's shell. On iPhone and in narrow iPad windows: Today, Home and Stars tabs, and
/// Search at the right (§4.3), each a history of pages; settings in a sheet from Home. In wide
/// iPad windows: one history in panes, two side by side in landscape (wiki: Sliding-Panes), and
/// a sidebar drawer with settings at its foot. `showsSelfHosted`: debug and TestFlight builds,
/// which keep the self-hosted sync settings (App Store builds sync only through iCloud).
public struct FlowieShell: View {
    @ObservedObject var store: OutlineStore
    let showsSelfHosted: Bool
    @StateObject private var nav = Navigation()
    @StateObject private var settings = Local(false)
    @StateObject private var started = Local(false)
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var sizeClass
    @AppStorage("appearance") private var appearance = "system"

    public init(store: OutlineStore, showsSelfHosted: Bool) {
        self.store = store
        self.showsSelfHosted = showsSelfHosted
    }

    /// Wide enough for the sidebar: an iPad window of regular width (not an iPhone in landscape).
    private var wide: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && sizeClass == .regular
    }

    public var body: some View {
        Group {
            if nav.wide {
                ZStack(alignment: .leading) {
                    PagesView(store: store, nav: nav, key: .window, onSettings: nil)
                    Sidebar(store: store, nav: nav) { settings.value = true }
                }
            } else {
                TabView(selection: $nav.tab) {
                    Tab("Today", systemImage: "sun.max", value: AppTab.today) { tab(.today) }
                    Tab("Home", systemImage: "house", value: AppTab.home) { tab(.home) }
                    Tab("Stars", systemImage: "star", value: AppTab.stars) { tab(.stars) }
                    Tab("Search", systemImage: "magnifyingglass", value: AppTab.search, role: .search) { tab(.search) }
                }
                .tabBarMinimizeBehavior(.onScrollDown)
            }
        }
        .sheet(isPresented: $settings.value) {
            SettingsView(store: store, showsSelfHosted: showsSelfHosted) { settings.value = false }
                .preferredColorScheme(scheme)
        }
        .preferredColorScheme(scheme)
        // Back, Forward, Zoom Out, the panes and the sidebar in the menu bar and with their
        // keys (FlowieCommands).
        .focusedSceneObject(nav)
        .onChange(of: wide) { _, wide in nav.setWide(wide) }
        .onAppear {
            guard !started.value else { return }
            started.value = true
            nav.store = store
            nav.setWide(wide)
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

    /// A capture was asked for (§5): its day, where the page puts the caret on its row.
    private func showCapture() {
        guard started.value, let capture = store.capture else { return }
        settings.value = false
        nav.sidebar = false
        nav.showDay(capture.day)
    }

    private var scheme: ColorScheme? {
        switch appearance {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    private func tab(_ tab: AppTab) -> some View {
        PagesView(store: store, nav: nav, key: .tab(tab), onSettings: tab == .home ? { settings.value = true } : nil)
    }
}

/// A history's pages side by side (wiki: Sliding-Panes §3): the current page, and the one before
/// it on the left when the window is wide, landscape and at least 1000pt across. A history of
/// one page shows it alone, centered. Pages on either side of those on screen are kept ready
/// (off screen), so Back and Forward slide them in, and a sideways swipe drags them along
/// (§2): let go past half a pane, or quickly, and it goes; at the ends of the history the pages
/// pull like a rubber band, and a swipe right at the start brings out the sidebar.
struct PagesView: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    let key: HistoryKey
    /// Home's settings button, in the tabs.
    let onSettings: (() -> Void)?
    /// How far the pages are dragged across.
    @StateObject private var drag = Local<CGFloat>(0)

    /// How wide a page alone in a wide window is at most, to keep its lines readable.
    static let readable: CGFloat = 700

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let history = nav.history(key)
            let two = key == .window && nav.twoPanes && history.index > 0
            let first = two ? history.index - 1 : history.index
            let width = two ? size.width / 2 : size.width
            // While the pages move, those beside the ones on screen are there to slide in.
            let range = nav.moving ? max(0, first - 1)...min(history.entries.count - 1, history.index + 1) : first...history.index
            ZStack(alignment: .topLeading) {
                ForEach(history.entries[range]) { entry in
                    let position = history.position(of: entry.id) ?? 0
                    let shown = position >= first && position <= history.index
                    let left = two && position == first
                    PaneContent(
                        store: store, nav: nav,
                        slot: PaneSlot(
                            key: key, position: position, leading: position == first,
                            highlight: left ? history.entries[position + 1].opener : nil,
                            focused: shown && nav.focusedPosition(key) == position,

                            onSettings: onSettings),
                        entry: entry
                    )
                    .frame(maxWidth: key == .window && !two && nav.canSplit ? Self.readable : .infinity)
                    .frame(width: width, height: size.height)
                    .background(Color(uiColor: .systemBackground))
                    .overlay(alignment: .leading) {
                        if two && position == history.index {
                            Rectangle().fill(Color(uiColor: .separator)).frame(width: 1 / UIScreen.main.scale).ignoresSafeArea()
                        }
                    }
                    .offset(x: CGFloat(position - first) * width + drag.value)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                    .transition(.asymmetric(insertion: .offset(x: nav.wentBack ? -width : width), removal: .identity))
                }
            }
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .clipped()
            .contentShape(Rectangle())
            .gesture(PageSwipe(changed: { dx in dragged(dx, width: width) }, ended: { dx, speed in
                let feedback = UISelectionFeedbackGenerator()
                if let back = settle(dx, speed: speed, width: width) {
                    feedback.selectionChanged()
                    withAnimation(.snappy) {
                        drag.value = 0
                        if back {
                            if nav.history(key).canGoBack { goBack() } else { nav.sidebar = true }
                        } else {
                            goForward()
                        }
                    }
                } else {
                    withAnimation(.snappy) { drag.value = 0 }
                    nav.move()
                }
            }))
            // VoiceOver's three-finger swipes go back and forward too.
            .accessibilityScrollAction { edge in
                switch edge {
                case .leading: if nav.history(key).canGoBack { goBack() }
                case .trailing: if nav.history(key).canGoForward { goForward() }
                default: break
                }
            }
            .onAppear { updateSplit(size) }
            .onChange(of: size) { _, size in updateSplit(size) }
        }
        .ignoresSafeArea(.keyboard)
    }

    /// The pages follow the finger; past the ends of the history (nothing to go to), they pull
    /// less and less. At the start of a wide window's history, the sidebar is to the left.
    private func dragged(_ dx: CGFloat, width: CGFloat) {
        nav.move(settling: false)
        let history = nav.history(key)
        let open = dx > 0 ? (history.canGoBack || nav.wide) : history.canGoForward
        drag.value = open ? max(-width, min(width, dx)) : rubberBand(dx, width: width)
    }

    /// Where a swipe let go goes: back (true), forward (false), or nowhere.
    private func settle(_ dx: CGFloat, speed: CGFloat, width: CGFloat) -> Bool? {
        let history = nav.history(key)
        let far = abs(dx) > width / 2 || (abs(speed) > 600 && speed * dx > 0)
        guard far else { return nil }
        if dx > 0 { return history.canGoBack || nav.wide ? true : nil }
        return history.canGoForward ? false : nil
    }

    private func goBack() {
        nav.back()
    }

    private func goForward() {
        nav.forward()
    }

    /// Two panes only in iPad landscape: wider than tall and at least 1000pt (§3).
    private func updateSplit(_ size: CGSize) {
        guard key == .window else { return }
        let can = size.width > size.height && size.width >= 1000
        if nav.canSplit != can {
            nav.canSplit = can
        }
    }
}

/// Where a pane is: its history, the place of its page in it, and what it shows besides.
struct PaneSlot {
    let key: HistoryKey
    let position: Int
    /// The first pane on screen: it has the sidebar, Back and Forward buttons.
    let leading: Bool
    /// The row of this page that opened the right pane.
    let highlight: String?
    /// It has the focus: the keyboard's commands act on it.
    let focused: Bool

    let onSettings: (() -> Void)?
}

/// The page of an entry of a history in its own navigation bar, with the sidebar, Back and
/// Forward buttons in the first pane.
struct PaneContent: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    let slot: PaneSlot
    let entry: PageEntry

    var body: some View {
        NavigationStack {
            Group {
                switch entry.page {
                case .home, .bullet, .day:
                    PageScreen(store: store, entry: entry, nav: nav, slot: slot)
                case .stars:
                    StarsScreen(store: store, nav: nav, slot: slot)
                case .search(let query):
                    SearchScreen(store: store, nav: nav, slot: slot, query: query)
                }
            }
            // Another day of the journal is another page; a search stays one while it is typed.
            .id(entry.page.identity)
            .toolbar {
                if slot.leading {
                    if nav.wide {
                        ToolbarItem(placement: .topBarLeading) {
                            Button(NSLocalizedString("Show Sidebar", comment: "Opens the sidebar drawer"), systemImage: "sidebar.leading") {
                                withAnimation(.snappy) { nav.sidebar = true }
                            }
                        }
                    }
                    HistoryButtons(nav: nav, history: nav.history(slot.key))
                }
            }
        }
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

/// The sidebar drawer of wide windows (wiki: Sliding-Panes §3): Today, Home, Search, each
/// starred page and the saved searches, opened as a new page; settings at its foot. It slides
/// in over the pages from the left, which dim; a tap on them puts it away.
struct Sidebar: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    let onSettings: () -> Void

    static let width: CGFloat = 320

    var body: some View {
        ZStack(alignment: .leading) {
            if nav.sidebar {
                Color.black.opacity(0.2)
                    .ignoresSafeArea()
                    .onTapGesture { withAnimation(.snappy) { nav.sidebar = false } }
                    .accessibilityAddTraits(.isButton)
                    .accessibilityLabel(NSLocalizedString("Hide Sidebar", comment: "Puts the sidebar drawer away"))
                    .transition(.opacity)
                list
                    .frame(width: Self.width)
                    .background(Color(uiColor: .systemBackground))
                    .shadow(color: .black.opacity(0.15), radius: 12)
                    .transition(.move(edge: .leading))
            }
        }
    }

    private var list: some View {
        // Read again as stars and saved searches change.
        let _ = store.revision
        let stars = store.starred()
        let searches = store.savedSearches()
        return NavigationStack {
            List {
                Section {
                    row(NSLocalizedString("Today", comment: "Title of today's journal page"), "sun.max", .day(LaunchMemory.today))
                    row(NSLocalizedString("Home", comment: "Title of the outline's top page"), "house", .home)
                    row(NSLocalizedString("Search", comment: "Title of the Search tab"), "magnifyingglass", .search(""))
                }
                if !stars.isEmpty {
                    Section(NSLocalizedString("Stars", comment: "Title of the Stars tab")) {
                        ForEach(stars, id: \.id) { bullet in
                            row(bullet.title, "star", .bullet(bullet.id))
                        }
                    }
                }
                if !searches.isEmpty {
                    Section(NSLocalizedString("Saved Searches", comment: "Sidebar section: the searches saved on the desktop")) {
                        ForEach(searches, id: \.self) { query in
                            row(query, "magnifyingglass", .search(query))
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationTitle("Flowie")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(NSLocalizedString("Hide Sidebar", comment: "Puts the sidebar drawer away"), systemImage: "sidebar.leading") {
                        withAnimation(.snappy) { nav.sidebar = false }
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(NSLocalizedString("Settings", comment: "Opens the settings sheet"), systemImage: "gearshape") {
                        nav.sidebar = false
                        onSettings()
                    }
                }
            }
        }
    }

    private func row(_ title: String, _ symbol: String, _ page: PageRef) -> some View {
        Button {
            withAnimation(.snappy) { nav.openFromSidebar(page) }
        } label: {
            Label(title, systemImage: symbol)
        }
        .foregroundStyle(.primary)
    }
}

/// The history's keys, as on the desktop (`keymap.rs`), in the iPad menu bar and the list shown
/// while ⌘ is held: ⌥⌘← Back, ⌥⌘→ Forward, ⌘[ Zoom Out (⌘] Zoom In is the edited row's); and
/// the panes' own: ⌃⌘← ⌃⌘→ the focus to the left or right pane, ⌘\ the sidebar.
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
            Divider()
            Button(NSLocalizedString("Focus Left Pane", comment: "Menu bar item: the keyboard acts on the left pane")) { nav?.focusPane(left: true) }
                .keyboardShortcut(.leftArrow, modifiers: [.command, .control])
                .disabled(!(nav?.twoPanes ?? false))
            Button(NSLocalizedString("Focus Right Pane", comment: "Menu bar item: the keyboard acts on the right pane")) { nav?.focusPane(left: false) }
                .keyboardShortcut(.rightArrow, modifiers: [.command, .control])
                .disabled(!(nav?.twoPanes ?? false))
            Button(nav?.sidebar == true
                   ? NSLocalizedString("Hide Sidebar", comment: "Puts the sidebar drawer away")
                   : NSLocalizedString("Show Sidebar", comment: "Opens the sidebar drawer")) {
                if let nav { withAnimation(.snappy) { nav.sidebar.toggle() } }
            }
            .keyboardShortcut("\\", modifiers: .command)
            .disabled(!(nav?.wide ?? false))
        }
    }
}

// MARK: Pages

/// A page of the outline with its title, the path to it, and its "…" menu (§4.3, §4.4). Top
/// pages of the tabs (Today, Home) have large titles; others a small one, the bullet's text,
/// with the path to it below, cut in the middle: a menu of the pages above it, each opened in
/// this pane when chosen (the breadcrumbs).
struct PageScreen: View {
    @ObservedObject var store: OutlineStore
    @StateObject private var page: PageModel
    @ObservedObject var nav: Navigation
    let slot: PaneSlot

    init(store: OutlineStore, entry: PageEntry, nav: Navigation, slot: PaneSlot) {
        self.store = store
        _page = StateObject(wrappedValue: {
            let page = store.page(entry.page.kind ?? .home)
            page.reveal = entry.reveal
            return page
        }())
        self.nav = nav
        self.slot = slot
    }

    /// The first page of a tab: a large title, and on Home the settings button.
    private var top: Bool {
        if case .tab = slot.key { return slot.position == 0 }
        return false
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
        case .bullet: return page.info?.pathLine(limit: 40)
        case .day(let day): return DayNames.subtitle(day)
        }
    }

    private func open(_ page: PageRef, opener: String? = nil) {
        nav.open(PageEntry(page, opener: opener), from: slot.position, in: slot.key)
    }

    var body: some View {
        OutlinePage(
            page: page,
            open: { id, opener in open(.bullet(id), opener: opener) },
            search: { query in nav.showSearch(query, from: slot.position) },
            highlight: slot.highlight,
            focused: slot.focused,
            onFocus: { [nav, slot] in
                let id = nav.history(slot.key).entries[min(slot.position, nav.history(slot.key).entries.count - 1)].id
                if nav.focused != id { nav.focused = id }
            }
        )
            // The page keeps its rows above the keyboard itself (an inset), and
            // its list scrolls under the tab bar.
            .ignoresSafeArea(.keyboard)
            .ignoresSafeArea(.container, edges: .bottom)
            .navigationTitle(title)
            .modifier(Subtitle(text: subtitle))
            .navigationBarTitleDisplayMode(top ? .large : .inline)
            .modifier(Breadcrumbs(info: page.kind == .home ? nil : page.info) { page, opener in open(page, opener: opener) })
            // While typing, the keyboard bar takes the bottom: with a hardware keyboard it
            // would sit on the tab bar.
            .toolbar(page.isEditing ? .hidden : .automatic, for: .tabBar)
            .toolbar {
                if case .day(let day) = page.kind {
                    ToolbarItemGroup(placement: .topBarLeading) {
                        Button(NSLocalizedString("Previous Day", comment: "Journal toolbar button"), systemImage: "chevron.left") {
                            nav.replace(at: slot.position, with: .day(LaunchMemory.day(day, offset: -1)), in: slot.key)
                        }
                        Button(NSLocalizedString("Next Day", comment: "Journal toolbar button"), systemImage: "chevron.right") {
                            nav.replace(at: slot.position, with: .day(LaunchMemory.day(day, offset: 1)), in: slot.key)
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
                    PageMenu(store: store, page: page, nav: nav) { day in
                        nav.replace(at: slot.position, with: .day(day), in: slot.key)
                    }
                }
                if top, case .home = page.kind, let onSettings = slot.onSettings {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(NSLocalizedString("Settings", comment: "Opens the settings sheet"), systemImage: "gearshape", action: onSettings)
                    }
                }
            }
            .safeAreaInset(edge: .top) {
                if let problem = store.problem {
                    Text(problem).font(.footnote).foregroundStyle(.red).padding(8)
                } else if store.fetchingFirst && (top || page.kind == .home) {
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

/// The breadcrumbs of a bullet's page: its title is a menu of the pages above it, each opened
/// in the same pane when chosen (wiki: Sliding-Panes §3), its row there pointed out.
private struct Breadcrumbs: ViewModifier {
    let info: BulletView?
    let open: (PageRef, String) -> Void

    func body(content: Content) -> some View {
        if let info {
            content.toolbarTitleMenu {
                Button(NSLocalizedString("Home", comment: "Title of the outline's top page"), systemImage: "house") {
                    open(.home, info.path.first?.id ?? info.id)
                }
                ForEach(Array(info.path.enumerated()), id: \.element.id) { index, crumb in
                    Button(crumb.text.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : crumb.text, systemImage: "circle.fill") {
                        open(.bullet(crumb.id), index + 1 < info.path.count ? info.path[index + 1].id : info.id)
                    }
                }
            }
        } else {
            content
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
/// reading mode, fold the page to a level, copy link, and in iPad landscape one pane or two.
/// `showDay`: show another day in its place.
struct PageMenu: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var page: PageModel
    @ObservedObject var nav: Navigation
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
            if nav.wide && nav.canSplit {
                // Landscape only: portrait has one pane anyway (wiki: Sliding-Panes §3).
                Toggle(isOn: Binding(get: { !nav.singlePane }, set: { two in withAnimation(.snappy) { nav.singlePane = !two } })) {
                    Label(NSLocalizedString("Two Panes", comment: "Page menu toggle: the page before shows beside the current one (iPad landscape)"), systemImage: "rectangle.split.2x1")
                }
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
    let slot: PaneSlot

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
                            nav.open(PageEntry(.bullet(bullet.id), opener: bullet.id), from: slot.position, in: slot.key)
                        } label: {
                            BulletLine(bullet: bullet)
                        }
                        .foregroundStyle(.primary)
                        // The left pane: the star that opened the right one.
                        .listRowBackground(slot.highlight == bullet.id ? Color.accentColor.opacity(0.15) : nil)
                        // Sideways swipes go back and forward (wiki: Sliding-Panes §2): Unstar is
                        // in the menu held open.
                        .contextMenu {
                            Button(NSLocalizedString("Unstar", comment: "Menu item"), systemImage: "star.slash") {
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
    let slot: PaneSlot
    @StateObject private var query: Local<String>

    init(store: OutlineStore, nav: Navigation, slot: PaneSlot, query: String) {
        self.store = store
        self.nav = nav
        self.slot = slot
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
                        nav.open(PageEntry(bullet.path.last.map { .bullet($0.id) } ?? .home, opener: bullet.id, reveal: bullet.id), from: slot.position, in: slot.key)
                    } label: {
                        BulletLine(bullet: bullet)
                    }
                    .foregroundStyle(.primary)
                    .listRowBackground(slot.highlight == bullet.id ? Color.accentColor.opacity(0.15) : nil)
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
            nav.replace(at: slot.position, with: .search(value), in: slot.key)
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
