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
    /// Target history index pre-mounted without animation one frame before a button slide, so
    /// the incoming pane lays out at its final width before sliding in.
    @Published private(set) var staged: Int?
    /// The last move went back: pages come in from the left.
    private(set) var wentBack = false
    /// When the left pane was expanded to fill the window (two-finger spread), the right pane's
    /// entry id so a two-finger pinch puts the pair back as they were.
    private var expandedFromLeft: UUID?
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
            expandedFromLeft = nil
            move()
        }
        histories[key] = history
    }

    /// Pre-mount the incoming pane unanimated when starting from rest, then slide on the next
    /// run-loop turn so its UIKit list never animates from an initial width.
    private func slide(_ key: HistoryKey, staging target: Int?, _ body: @escaping (inout PageHistory) -> Void) {
        if moving {
            staged = nil
            withAnimation(.snappy) {
                change(key, body)
                focused = nil
            }
            return
        }
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            staged = target
            move(settling: false)
        }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            withAnimation(.snappy) {
                self.staged = nil
                self.change(key, body)
                self.focused = nil
            }
        }
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

    /// Show another page in an entry without a new one (the journal's days, Up a Level, a
    /// breadcrumb, or a search typed).
    func replace(at position: Int, with page: PageRef, opener: String? = nil, reveal: String? = nil, in key: HistoryKey) {
        let history = history(key)
        if history.entries.indices.contains(position), history.entries[position].page != page {
            memories[history.entries[position].id] = nil
        }
        change(key) { $0.replace(at: position, with: page, opener: opener, reveal: reveal) }
    }

    func clearReveal(_ id: UUID, in key: HistoryKey) {
        guard var history = histories[key], let pos = history.position(of: id), history.entries[pos].reveal != nil else { return }
        history.update(id) { $0.reveal = nil }
        histories[key] = history
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
        let current = history(key)
        guard current.canGoBack else { return }
        slide(key, staging: current.index - 1) { history in
            self.prune(&history)
            history.goBack()
        }
    }

    func forward() {
        let current = history(key)
        guard current.canGoForward else { return }
        slide(key, staging: current.index + 1) { history in
            self.prune(&history)
            history.goForward()
        }
    }

    /// Go to a page of the history, from the back or forward button's menu.
    func go(to position: Int) {
        let current = history(key)
        let clamped = min(max(0, position), current.entries.count - 1)
        guard clamped != current.index else { return }
        let id = current.entries[clamped].id
        slide(key, staging: clamped) { history in
            self.prune(&history)
            history.go(to: history.position(of: id) ?? history.index)
        }
    }

    /// Switch the focused pane in place to the page above it (⌘[, the desktop's Zoom Out),
    /// pointing out the bullet just left.
    func zoomOut() {
        let key = key
        let position = focusedPosition(key)
        guard case .bullet(let id) = history(key).entries[position].page,
              let store, let bullet = store.bullet(id) else { return }
        let parent = bullet.path.last.map { store.pageRef(bullet: $0.id) } ?? .home
        let opener: String? = if case .bullet(let pid) = parent { pid } else { nil }
        replace(at: position, with: parent, opener: opener, reveal: id, in: key)
    }

    var canExpandToSinglePane: Bool {
        twoPanes && history(.window).index > 0
    }

    var canCollapseToTwoPanes: Bool {
        guard wide && canSplit && singlePane else { return false }
        let history = history(.window)
        return history.index > 0 || history.canGoForward
    }

    /// Two-finger spread on the left or right pane: fill the window with that pane's page.
    func expandPane(left: Bool) {
        guard canExpandToSinglePane else { return }
        let current = history(.window)
        let rememberedRight = left ? current.current.id : nil
        move()
        withAnimation(.snappy) {
            if left {
                var next = current
                next.goBack()
                histories[.window] = next
            }
            expandedFromLeft = rememberedRight
            singlePane = true
            focused = nil
        }
    }

    /// Two-finger pinch in single-pane mode: restore two panes side by side.
    func collapseToTwoPanes() {
        guard canCollapseToTwoPanes else { return }
        let current = history(.window)
        let restoreRight: Bool
        if let rightID = expandedFromLeft, current.index + 1 < current.entries.count, current.entries[current.index + 1].id == rightID {
            restoreRight = true
        } else {
            restoreRight = current.index == 0 && current.canGoForward
        }
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            staged = restoreRight ? current.index + 1 : current.index
            move(settling: false)
        }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            withAnimation(.snappy) {
                self.staged = nil
                if restoreRight {
                    var next = self.history(.window)
                    next.goForward()
                    self.histories[.window] = next
                }
                self.expandedFromLeft = nil
                self.singlePane = false
                self.focused = nil
                self.move()
            }
        }
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

    /// A page asked for from outside the window (an intent, Spotlight, Handoff): a search in
    /// the Search tab, a day in Today, another page opened in the tab it belongs in, or in a
    /// wide window's history; nothing new when it shows already.
    func show(_ page: PageRef) {
        switch page {
        case .search(let query):
            showSearch(query)
        case .day(let day):
            showDay(day)
        case .home, .bullet, .stars:
            if !wide { tab = AppTab(for: page) }
            let history = history(key)
            if history.current.page != page {
                open(PageEntry(page), from: history.index, in: key)
            }
        }
    }

    /// How each page was left (scrolled, the caret), by entry: kept aside so scrolling doesn't
    /// redraw the shell, and put into the history when it is saved or a page comes back.
    private var memories: [UUID: PageMemory] = [:]

    func remember(_ id: UUID, _ memory: PageMemory) {
        memories[id] = memory
    }

    func memory(of entry: PageEntry) -> PageMemory {
        memories[entry.id] ?? entry.memory
    }

    /// A history with how its pages were left.
    private func remembered(_ key: HistoryKey) -> PageHistory {
        var history = history(key)
        for entry in history.entries {
            if let memory = memories[entry.id] {
                history.update(entry.id) { $0.memory = memory }
            }
        }
        return history
    }

    /// Where the window is, to remember (§4.3): its history, the tab it is in, one pane or two.
    var place: LaunchMemory.Place {
        let history = remembered(key)
        return LaunchMemory.Place(tab: (wide ? AppTab(for: history.current.page) : tab).rawValue, history: history, singlePane: singlePane)
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
            var history = place.history
            // Pages deleted since are left out.
            history.prune { page in
                if case .bullet(let id) = page { return store.bullet(id) != nil }
                return true
            }
            let tab = AppTab(rawValue: place.tab) ?? AppTab(for: history.current.page)
            histories[.tab(tab)] = history
            histories[.window] = history
            self.tab = tab
            singlePane = place.singlePane
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
    /// The page this window was opened for (Open in New Window, a bullet dragged to the
    /// screen's edge), when that came before the window started.
    @StateObject private var opened = Local<PageRef?>(nil)
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var sizeClass
    /// This window's own id, kept by the system with the scene: its history is saved under it.
    @SceneStorage("flowie.window") private var window = ""
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
            if window.isEmpty { window = UUID().uuidString }
            nav.start(LaunchMemory.start(window: window), store: store)
            if let page = opened.value {
                opened.value = nil
                nav.start(.place(LaunchMemory.Place(tab: AppTab(for: page).rawValue, history: PageHistory(page))), store: store)
            }
            #if DEBUG
            ScreenshotArguments.apply(to: nav, store: store)
            #endif
            showCapture()
            showOpening()
        }
        .onChange(of: store.capture) { _, _ in showCapture() }
        .onChange(of: store.opening) { _, _ in showOpening() }
        // Handoff: the page with the focus, to go on with on another iPhone or iPad.
        .userActivity(PageActivity.type, element: PageActivity.handsOff(handedOff) ? handedOff : nil) { page, activity in
            PageActivity.fill(activity, page: page, title: nav.title(page))
        }
        .onContinueUserActivity(PageActivity.type) { activity in
            guard let page = PageActivity.page(of: activity) else { return }
            if case .bullet(let id) = page, store.bullet(id) == nil { return }
            if PageActivity.opensWindow(activity) {
                // This window was opened for the page: its history starts there.
                if started.value {
                    nav.start(.place(LaunchMemory.Place(tab: AppTab(for: page).rawValue, history: PageHistory(page))), store: store)
                } else {
                    opened.value = page
                }
            } else {
                store.show(page)
            }
        }
        .onChange(of: scenePhase) { old, phase in
            switch phase {
            case .background:
                LaunchMemory.leave(nav.place, window: window)
            case .active where old == .background:
                // Back after a while, or on another day: today's journal (§4.3).
                if case .today = LaunchMemory.start(window: window) {
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

    /// The page with the focus, which Handoff offers.
    private var handedOff: PageRef {
        let key = nav.key
        let history = nav.history(key)
        return history.entries[min(nav.focusedPosition(key), history.entries.count - 1)].page
    }

    /// A page asked for from outside (§5): shown once.
    private func showOpening() {
        guard started.value, let page = store.takeOpening() else { return }
        settings.value = false
        nav.sidebar = false
        nav.show(page)
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

/// Handoff of the page shown (wiki: TODO-ios ios:32): an iPhone or iPad signed in to the same
/// iCloud account opens it. The activity names the page (`PageRef`, as JSON) and, for a bullet's
/// page, its id as the desktop's links do (`flowie:<id>`), so a Mac app could take it too.
enum PageActivity {
    /// Declared in the app's Info.plist (`NSUserActivityTypes`, Project.swift).
    static let type = "flowie.spokey.cn.page"

    /// Pages worth going on with elsewhere: the outline's, and a search typed.
    static func handsOff(_ page: PageRef) -> Bool {
        switch page {
        case .home, .bullet, .day: return true
        case .search(let query): return !query.trimmingCharacters(in: .whitespaces).isEmpty
        case .stars: return false
        }
    }

    static func fill(_ activity: NSUserActivity, page: PageRef, title: String) {
        activity.title = title
        activity.isEligibleForHandoff = true
        // Spotlight has the pages already (the app's index), and Siri's suggestions the intents.
        activity.isEligibleForSearch = false
        activity.isEligibleForPrediction = false
        var info: [String: Any] = [:]
        if let data = try? JSONEncoder().encode(page) {
            info["page"] = data
        }
        if case .bullet(let id) = page {
            info["id"] = id
            activity.targetContentIdentifier = "flowie:\(id)"
        } else {
            activity.targetContentIdentifier = nil
        }
        activity.userInfo = info
        activity.requiredUserInfoKeys = Set(info.keys)
        activity.needsSave = true
    }

    /// An activity that opens `page` in a new window of its own, its history starting there
    /// (wiki: Sliding-Panes §6): Open in New Window, or a bullet dragged to the screen's edge.
    static func newWindow(_ page: PageRef, title: String) -> NSUserActivity {
        let activity = NSUserActivity(activityType: type)
        fill(activity, page: page, title: title)
        activity.isEligibleForHandoff = false
        activity.userInfo?["window"] = true
        activity.requiredUserInfoKeys?.insert("window")
        return activity
    }

    /// Whether an activity opens its page in a new window (`newWindow`), not Handoff's.
    static func opensWindow(_ activity: NSUserActivity) -> Bool {
        activity.userInfo?["window"] as? Bool ?? false
    }

    /// Whether this device opens pages in windows of their own: an iPad.
    @MainActor static var opensWindows: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && UIApplication.shared.supportsMultipleScenes
    }

    /// Open `page` in a new window, beside `scene`'s.
    @MainActor static func openWindow(_ page: PageRef, title: String, from scene: UIScene?) {
        let options = UIScene.ActivationRequestOptions()
        options.requestingScene = scene
        let request = UISceneSessionActivationRequest(role: .windowApplication, userActivity: newWindow(page, title: title), options: options)
        UIApplication.shared.activateSceneSession(for: request) { error in
            NSLog("Flowie: couldn't open a window: \(error)")
        }
    }

    /// The page an activity names: from `page`, or a bare `id` (from another kind of app).
    static func page(of activity: NSUserActivity) -> PageRef? {
        if let data = activity.userInfo?["page"] as? Data, let page = try? JSONDecoder().decode(PageRef.self, from: data) {
            return page
        }
        return (activity.userInfo?["id"] as? String).map { .bullet($0) }
    }
}

/// A history's pages side by side (wiki: Sliding-Panes §3): the current page, and the one before
/// it on the left when the window is wide, landscape and at least 1000pt across. A history of
/// one page fills the window; in landscape it narrows into the left pane as a second page comes
/// in, and widens back when the history is down to one again (§1). Pages on either side of
/// those on screen are kept ready (off screen), so Back and Forward slide them in, and a
/// sideways swipe drags them along (§2): let go past half a pane, or quickly, and it goes; at
/// the ends of the history the pages pull like a rubber band, and a swipe right at the start
/// brings out the sidebar.
struct PagesView: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var nav: Navigation
    let key: HistoryKey
    /// Home's settings button, in the tabs.
    let onSettings: (() -> Void)?
    /// How far the pages are dragged across.
    @StateObject private var drag = Local<CGFloat>(0)

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let history = nav.history(key)
            let layout = PaneLayout(width: size.width, twoUp: key == .window && nav.twoPanes)
            let two = layout.twoUp && history.index > 0
            let first = two ? history.index - 1 : history.index
            let step = layout.step
            // A swipe goes part of the way to the page before or after, if there is one.
            let target = drag.value > 0 ? (history.canGoBack ? history.index - 1 : nil) : (drag.value < 0 && history.canGoForward ? history.index + 1 : nil)
            // While the pages move, those beside the ones on screen (plus any staged target) are
            // kept in the tree to slide in without mounting mid-animation.
            let baseRange = nav.moving ? max(0, first - 1)...min(history.entries.count - 1, history.index + 1) : first...history.index
            let entries: [PageEntry] = {
                var indices = Set(baseRange)
                if let staged = nav.staged {
                    let willSplit = key == .window && nav.wide && nav.canSplit
                    let low = willSplit ? max(0, staged - 1) : staged
                    for i in low...min(history.entries.count - 1, staged) where history.entries.indices.contains(i) {
                        indices.insert(i)
                    }
                    if willSplit, history.index > 0 {
                        indices.insert(history.index - 1)
                    }
                }
                return indices.sorted().map { history.entries[$0] }
            }()
            ZStack(alignment: .topLeading) {
                ForEach(entries) { entry in
                    let position = history.position(of: entry.id) ?? 0
                    let shown = position >= first && position <= history.index
                    let left = two && position == first
                    let place = layout.place(position, at: history.index, dragged: drag.value, towards: target)
                    let contentWidth = drag.value == 0 ? layout.place(position, at: history.index).width : place.width
                    PaneContent(
                        store: store, nav: nav,
                        slot: PaneSlot(
                            key: key, position: position, leading: position == first,
                            highlight: left ? history.entries[position + 1].opener : nil,
                            focused: shown && nav.focusedPosition(key) == position,

                            onSettings: onSettings),
                        entry: entry
                    )
                    .frame(width: contentWidth, height: size.height)
                    .transaction { transaction in
                        // Size the pane's UIKit list once at its resting width instead of
                        // reflowing text across every animation frame.
                        if drag.value == 0 {
                            transaction.animation = nil
                        }
                    }
                    .frame(width: place.width, height: size.height, alignment: .topLeading)
                    .background(Color(uiColor: .systemBackground))
                    .clipped()
                    .overlay(alignment: .leading) {
                        if layout.twoUp && position > 0 {
                            Rectangle().fill(Color(uiColor: .separator)).frame(width: 1 / UIScreen.main.scale).ignoresSafeArea()
                        }
                    }
                    .offset(x: place.x)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                    .transition(.asymmetric(insertion: .offset(x: nav.wentBack ? -step : step), removal: .identity))
                }
            }
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .clipped()
            .contentShape(Rectangle())
            .gesture(PageSwipe(changed: { dx in dragged(dx, width: step) }, ended: { dx, speed in
                let feedback = UISelectionFeedbackGenerator()
                if let back = settle(dx, speed: speed, width: step) {
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
            .gesture(PagePinch(
                canExpand: { key == .window && nav.canExpandToSinglePane },
                canCollapse: { key == .window && nav.canCollapseToTwoPanes },
                trigger: { expand, leftHalf in
                    UISelectionFeedbackGenerator().selectionChanged()
                    if expand {
                        nav.expandPane(left: leftHalf)
                    } else {
                        nav.collapseToTwoPanes()
                    }
                }
            ))
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
    @StateObject private var customizingToolbar = Local(false)
    @ObservedObject var nav: Navigation
    let slot: PaneSlot
    let entry: PageEntry

    init(store: OutlineStore, entry: PageEntry, nav: Navigation, slot: PaneSlot) {
        self.entry = entry
        self.store = store
        _page = StateObject(wrappedValue: {
            let page = store.page(entry.page.kind ?? .home)
            // A search result is pointed out the first time only: back to it, it is as it was left.
            page.reveal = nav.memory(of: entry).topRow == nil ? entry.reveal : nil
            return page
        }())
        self.nav = nav
        self.slot = slot
    }

    /// The first page of a tab: a large title, and on Home the settings entry.
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

    /// Switch this pane in place (Up a Level, breadcrumbs), keeping the other pane and the rest
    /// of the history untouched.
    private func replaceWith(_ target: PageRef, reveal: String? = nil) {
        let opener: String? = if case .bullet(let id) = target { id } else { nil }
        nav.replace(at: slot.position, with: target, opener: opener, reveal: reveal, in: slot.key)
    }

    private func upLevel() {
        guard case .bullet = page.kind, let info = page.info else { return }
        let parent = info.path.last.map { store.pageRef(bullet: $0.id) } ?? .home
        replaceWith(parent, reveal: info.id)
    }

    var body: some View {
        OutlinePage(
            page: page,
            open: { id, opener in open(.bullet(id), opener: opener) },
            search: { query in nav.showSearch(query, from: slot.position) },
            openHome: { open(.home) },
            highlight: slot.highlight,
            focused: slot.focused,
            onFocus: { [nav, entry] in
                if nav.focused != entry.id { nav.focused = entry.id }
            },
            memory: nav.memory(of: entry),
            remember: { [nav, entry] memory in nav.remember(entry.id, memory) }
        )
            // The page keeps its rows above the keyboard itself (an inset), and
            // its list scrolls under the tab bar.
            .ignoresSafeArea(.keyboard)
            .ignoresSafeArea(.container, edges: .bottom)
            .navigationTitle(title)
            .modifier(Subtitle(text: subtitle))
            .navigationBarTitleDisplayMode(top ? .large : .inline)
            .modifier(Breadcrumbs(info: page.root.flatMap { page.kind == .bullet($0) ? page.info : nil }, store: store) { target, reveal in
                replaceWith(target, reveal: reveal)
            })
            .onAppear {
                if entry.reveal != nil {
                    nav.clearReveal(entry.id, in: slot.key)
                }
            }
            // While typing, the keyboard bar takes the bottom: with a hardware keyboard it
            // would sit on the tab bar.
            .toolbar(page.isEditing ? .hidden : .automatic, for: .tabBar)
            .toolbar {
                if case .day(let day) = page.kind {
                    ToolbarItemGroup(placement: .topBarLeading) {
                        Button(NSLocalizedString("Previous Day", comment: "Journal toolbar button"), systemImage: "calendar.day.timeline.left") {
                            nav.replace(at: slot.position, with: .day(LaunchMemory.day(day, offset: -1)), in: slot.key)
                        }
                        Button(NSLocalizedString("Next Day", comment: "Journal toolbar button"), systemImage: "calendar.day.timeline.right") {
                            nav.replace(at: slot.position, with: .day(LaunchMemory.day(day, offset: 1)), in: slot.key)
                        }
                    }
                }
                ForEach(ToolbarAction.allCases.filter(store.toolbarActions.contains), id: \.self) { action in
                    toolbarButton(for: action)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    PageMenu(
                        store: store,
                        page: page,
                        nav: nav,
                        onUpLevel: upLevel,
                        onCustomizeToolbar: { customizingToolbar.value = true },
                        onSettings: top && page.kind == .home ? slot.onSettings : nil
                    ) { day in
                        nav.replace(at: slot.position, with: .day(day), in: slot.key)
                    }
                }
            }
            .sheet(isPresented: $customizingToolbar.value) {
                ToolbarSettingsView(store: store, showsTwoPanes: UIDevice.current.userInterfaceIdiom == .pad) {
                    customizingToolbar.value = false
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

    @ToolbarContentBuilder
    private func toolbarButton(for action: ToolbarAction) -> some ToolbarContent {
        switch action {
        case .upLevel:
            if case .bullet = page.kind, page.info != nil {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(NSLocalizedString("Up a Level", comment: "Navigation bar button that opens the page of the bullet above this one"), systemImage: "arrow.turn.left.up", action: upLevel)
                }
            }
        case .star:
            if let info = page.info {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(
                        info.starred ? NSLocalizedString("Unstar", comment: "Menu item") : NSLocalizedString("Star", comment: "Menu item"),
                        systemImage: info.starred ? "star.fill" : "star"
                    ) {
                        store.setStarred(info.id, !info.starred)
                    }
                }
            }
        case .readingMode:
            ToolbarItem(placement: .topBarTrailing) {
                Button(
                    store.readingMode
                        ? NSLocalizedString("Turn Off Reading Mode", comment: "Navigation bar button shown while reading mode is on")
                        : NSLocalizedString("Reading Mode", comment: "Page menu toggle: nothing can be edited while on"),
                    systemImage: "eyeglasses"
                ) {
                    store.setReadingMode(!store.readingMode)
                }
                .tint(store.readingMode ? .accentColor : nil)
            }
        case .hideCompleted:
            ToolbarItem(placement: .topBarTrailing) {
                Button(
                    store.hideCompleted ? NSLocalizedString("Show Completed", comment: "Menu item") : NSLocalizedString("Hide Completed", comment: "Menu item"),
                    systemImage: store.hideCompleted ? "eye.slash" : "checklist"
                ) {
                    store.setHideCompleted(!store.hideCompleted)
                }
                .tint(store.hideCompleted ? .accentColor : nil)
            }
        case .twoPanes:
            if nav.wide && nav.canSplit {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(NSLocalizedString("Two Panes", comment: "Page menu toggle: the page before shows beside the current one (iPad landscape)"), systemImage: "rectangle.split.2x1") {
                        withAnimation(.snappy) { nav.singlePane.toggle() }
                    }
                    .tint(!nav.singlePane ? .accentColor : nil)
                }
            }
        case .copyLink:
            if let id = page.root {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(NSLocalizedString("Copy Link", comment: "Menu item"), systemImage: "link") {
                        if let link = store.link(id) { UIPasteboard.general.string = link }
                    }
                }
            }
        case .fold:
            if page.canFold {
                ToolbarItem(placement: .topBarTrailing) {
                    FoldButton(page: page)
                }
            }
        }
    }
}

/// The breadcrumbs of a bullet's page: its title is a menu of the pages above it, each shown in
/// place in the same pane when chosen (wiki: Sliding-Panes §3), its row there pointed out.
private struct Breadcrumbs: ViewModifier {
    let info: BulletView?
    let store: OutlineStore
    let select: (PageRef, String) -> Void

    func body(content: Content) -> some View {
        if let info {
            content.toolbarTitleMenu {
                Button(NSLocalizedString("Home", comment: "Title of the outline's top page"), systemImage: "house") {
                    select(.home, info.path.first?.id ?? info.id)
                }
                ForEach(Array(info.path.enumerated()), id: \.element.id) { index, crumb in
                    Button(crumb.text.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : crumb.text, systemImage: "circle.fill") {
                        select(store.pageRef(bullet: crumb.id), index + 1 < info.path.count ? info.path[index + 1].id : info.id)
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

/// Fold the whole page, beside its "…" menu: each tap goes to the next of Collapse All, Show 2
/// Levels and (when the page goes deeper than two levels) Expand All, the symbol showing the
/// state the page is in (a neutral one once rows were folded by hand, the next tap collapsing
/// all). Works in reading mode.
struct FoldButton: View {
    @ObservedObject var page: PageModel

    var body: some View {
        let next = FoldLevel.after(page.foldState, deep: page.hasDeepLevels)
        Button {
            page.fold(to: next)
        } label: {
            Label(NSLocalizedString("Fold", comment: "Navigation bar button that folds the page a step further each tap"), systemImage: page.foldState?.symbol ?? FoldLevel.neutralSymbol)
        }
        .accessibilityValue(page.foldState?.title ?? "")
        .accessibilityHint(next.title)
    }
}

/// A page's "…" menu, at its top right: actions not pinned to the toolbar, submenus for folding
/// and view options, and "Customize Toolbar…" to pick which buttons stay beside "…".
struct PageMenu: View {
    @ObservedObject var store: OutlineStore
    @ObservedObject var page: PageModel
    @ObservedObject var nav: Navigation
    let onUpLevel: () -> Void
    let onCustomizeToolbar: () -> Void
    let onSettings: (() -> Void)?
    let showDay: (String) -> Void

    var body: some View {
        Menu {
            if case .bullet = page.kind, page.info != nil, !store.toolbarActions.contains(.upLevel) {
                Button(NSLocalizedString("Up a Level", comment: "Navigation bar button that opens the page of the bullet above this one"), systemImage: "arrow.turn.left.up", action: onUpLevel)
            }
            if let info = page.info, !store.toolbarActions.contains(.star) {
                Button(
                    info.starred ? NSLocalizedString("Unstar", comment: "Menu item") : NSLocalizedString("Star", comment: "Menu item"),
                    systemImage: info.starred ? "star.slash" : "star"
                ) {
                    store.setStarred(info.id, !info.starred)
                }
            }
            if let id = page.root, !store.toolbarActions.contains(.copyLink) {
                Button(NSLocalizedString("Copy Link", comment: "Menu item"), systemImage: "link") {
                    if let link = store.link(id) { UIPasteboard.general.string = link }
                }
            }
            if case .day(let day) = page.kind, day != LaunchMemory.today {
                Button(NSLocalizedString("Go to Today", comment: "Menu item"), systemImage: "sun.max") {
                    showDay(LaunchMemory.today)
                }
            }
            if page.canFold {
                Menu(NSLocalizedString("Folding", comment: "Submenu in the page … menu for folding levels"), systemImage: "list.bullet.indent") {
                    ForEach(FoldLevel.available(deep: page.hasDeepLevels), id: \.self) { level in
                        Button {
                            page.fold(to: level)
                        } label: {
                            Label(level.title, systemImage: page.foldState == level ? "checkmark" : level.symbol)
                        }
                    }
                }
            }
            Menu(NSLocalizedString("View Options", comment: "Submenu in the page … menu for reading mode, completed rows and panes"), systemImage: "eyeglasses") {
                Toggle(isOn: Binding(get: { store.readingMode }, set: { store.setReadingMode($0) })) {
                    Label(NSLocalizedString("Reading Mode", comment: "Page menu toggle: nothing can be edited while on"), systemImage: "eyeglasses")
                }
                Toggle(isOn: Binding(get: { store.hideCompleted }, set: { store.setHideCompleted($0) })) {
                    Label(NSLocalizedString("Hide Completed", comment: "Menu item"), systemImage: "checklist")
                }
                if nav.wide && nav.canSplit {
                    // Landscape only: portrait has one pane anyway (wiki: Sliding-Panes §3).
                    Toggle(isOn: Binding(get: { !nav.singlePane }, set: { two in withAnimation(.snappy) { nav.singlePane = !two } })) {
                        Label(NSLocalizedString("Two Panes", comment: "Page menu toggle: the page before shows beside the current one (iPad landscape)"), systemImage: "rectangle.split.2x1")
                    }
                }
            }
            Divider()
            Button(NSLocalizedString("Customize Toolbar…", comment: "Page menu item: opens a sheet to pick top-right toolbar buttons"), systemImage: "slider.horizontal.3", action: onCustomizeToolbar)
            if let onSettings {
                Button(NSLocalizedString("Settings", comment: "Opens the settings sheet"), systemImage: "gearshape", action: onSettings)
            }
        } label: {
            Label(NSLocalizedString("More", comment: "The page menu's button"), systemImage: "ellipsis")
        }
    }
}

/// Half-height card opened from "…" → Customize Toolbar…: toggles for which buttons sit beside
/// "…" in the top-right navigation bar (default: only Fold).
struct ToolbarSettingsView: View {
    @ObservedObject var store: OutlineStore
    let showsTwoPanes: Bool
    let close: () -> Void

    private var items: [ToolbarAction] {
        ToolbarAction.sheetOrder.filter { showsTwoPanes || $0 != .twoPanes }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ForEach(items, id: \.self) { action in
                        Toggle(isOn: Binding(
                            get: { store.toolbarActions.contains(action) },
                            set: { store.setToolbar(action, $0) }
                        )) {
                            Label(action.title, systemImage: action.symbol)
                        }
                    }
                } footer: {
                    Text("Choose which buttons stay in the top-right toolbar. Everything else stays in the … menu.")
                }
                if store.toolbarActions != ToolbarAction.defaults {
                    Section {
                        Button(NSLocalizedString("Reset to Default", comment: "Customize Toolbar sheet button")) {
                            store.resetToolbar()
                        }
                    }
                }
            }
            .navigationTitle(NSLocalizedString("Customize Toolbar", comment: "Title of the Customize Toolbar sheet"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(NSLocalizedString("Done", comment: "Closes the settings sheet"), systemImage: "checkmark", action: close)
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
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
