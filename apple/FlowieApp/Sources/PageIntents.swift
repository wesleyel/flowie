// Pages for Shortcuts, Siri and Spotlight (wiki: TODO-ios ios:32): a page of the outline as an
// App Entity, found with the outline's search, "Open Page" and "Search Flowie". Like quick
// capture they run in the app, which alone opens the outline (wiki: iOS-Cross-Platform §5).
import AppIntents
import FlowieFFI
import FlowieUI

/// A bullet's page: its title and the path to it.
struct PageEntity: AppEntity {
    static let typeDisplayRepresentation = TypeDisplayRepresentation(name: "Page")
    static let defaultQuery = PageQuery()

    /// The bullet's id (32 hex digits, as in a `flowie:` link).
    let id: String
    let title: String
    /// The bullets above it, as one line ("Home" at the top).
    let path: String

    init(_ bullet: BulletView) {
        id = bullet.id
        title = bullet.title
        path = bullet.pathLine()
    }

    init(id: String, title: String, path: String) {
        self.id = id
        self.title = title
        self.path = path
    }

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: "\(path)", image: .init(systemName: "list.bullet.indent"))
    }
}

/// Pages by id, by what they say (the Search tab's search), and the starred ones to pick from.
struct PageQuery: EntityStringQuery {
    @MainActor func entities(for identifiers: [PageEntity.ID]) async throws -> [PageEntity] {
        let store = try Launch.shared.openStore()
        return identifiers.compactMap { store.bullet($0).map(PageEntity.init) }
    }

    @MainActor func entities(matching string: String) async throws -> [PageEntity] {
        try Launch.shared.openStore().search(string, limit: 50).map(PageEntity.init)
    }

    @MainActor func suggestedEntities() async throws -> [PageEntity] {
        try Launch.shared.openStore().starred().map(PageEntity.init)
    }
}

/// Open the app on a page, as a new page of the window's history (Back returns).
struct OpenPage: OpenIntent {
    static let title: LocalizedStringResource = "Open Page"
    static let description = IntentDescription("Opens a page of your outline in Flowie.")
    @available(iOS 27, *)
    static var allowedExecutionTargets: IntentExecutionTargets { .main }

    @Parameter(title: "Page", requestValueDialog: "Which page?")
    var target: PageEntity

    static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$target)")
    }

    init() {}

    init(target: PageEntity) {
        self.target = target
    }

    @MainActor func perform() async throws -> some IntentResult {
        let store = try Launch.shared.openStore()
        guard store.bullet(target.id) != nil else { throw PageFailure.gone }
        store.show(store.pageRef(bullet: target.id))
        return .result()
    }
}

/// Open the Search tab searching for something, with the desktop's filters.
struct SearchFlowie: AppIntent {
    static let title: LocalizedStringResource = "Search Flowie"
    static let description = IntentDescription("Searches your outline in Flowie, with the same filters as the Search tab.")
    static let supportedModes: IntentModes = .foreground(.immediate)
    @available(iOS 27, *)
    static var allowedExecutionTargets: IntentExecutionTargets { .main }

    @Parameter(title: "Search For", requestValueDialog: "What do you want to search for?")
    var query: String

    static var parameterSummary: some ParameterSummary {
        Summary("Search Flowie for \(\.$query)")
    }

    @MainActor func perform() async throws -> some IntentResult {
        try Launch.shared.openStore().show(.search(query.trimmingCharacters(in: .whitespacesAndNewlines)))
        return .result()
    }
}

/// Why a page couldn't be opened.
enum PageFailure: LocalizedError {
    /// Deleted since it was picked.
    case gone

    var errorDescription: String? {
        String(localized: "That page is no longer in your outline.")
    }
}
