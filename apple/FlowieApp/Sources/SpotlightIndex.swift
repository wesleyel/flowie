// The outline's pages in Spotlight (wiki: TODO-ios ios:32): bullets with children, starred
// bullets and the journal's days, not every bullet. Kept current a while after edits and sync,
// off the main thread, sending only the pages whose digest changed (flowie-ffi `index_pages`).
import Combine
import CoreSpotlight
import FlowieFFI
import FlowieUI
import Foundation
import os

/// Keeps Spotlight's copy of the pages in step with the outline. What it last indexed (each
/// page's digest) is kept beside the database: with none there (a new or replaced outline),
/// Spotlight's copy is cleared and everything is indexed again.
final class SpotlightIndex: @unchecked Sendable {
    static let domain = "pages"
    /// How long the outline stays unchanged before indexing (typing goes on meanwhile).
    static let quiet: TimeInterval = 2

    private let outline: Outline
    private let stateURL: URL
    private let index: CSSearchableIndex
    private let queue = DispatchQueue(label: "flowie.spotlight", qos: .utility)
    private let log = Logger(subsystem: "flowie.spokey.cn", category: "spotlight")
    private var watching: AnyCancellable?
    /// Touched on `queue` only: what Spotlight has (id → digest), nil until read; a run going
    /// on, and another asked for meanwhile.
    private var indexed: [String: UInt64]?
    private var running = false
    private var again = false

    @MainActor init(store: OutlineStore, stateURL: URL, index: CSSearchableIndex = .default()) {
        outline = store.outline
        self.stateURL = stateURL
        self.index = index
        // Every edit, star and sync bumps the revision; index once they stop for a while.
        watching = store.$revision
            .debounce(for: .seconds(Self.quiet), scheduler: DispatchQueue.main)
            .sink { [weak self] _ in self?.update() }
    }

    /// Index what changed since the last time, off the main thread.
    func update() {
        queue.async { [self] in
            guard !running else {
                again = true
                return
            }
            running = true
            run()
        }
    }

    /// On `queue`.
    private func run() {
        let started = Date()
        let pages = outline.indexPages()
        let fresh = indexed == nil && !FileManager.default.fileExists(atPath: stateURL.path)
        var known = indexed ?? load()
        let ids = Set(pages.map(\.id))
        let changed = pages.filter { known[$0.id] != $0.digest }
        let gone = known.keys.filter { !ids.contains($0) }
        log.debug("\(pages.count) pages read in \(Date().timeIntervalSince(started) * 1000, format: .fixed(precision: 1)) ms: \(changed.count) to index, \(gone.count) to remove\(fresh ? ", clearing first" : "")")
        guard fresh || !changed.isEmpty || !gone.isEmpty else {
            indexed = known
            return finish()
        }
        let items = changed.map(Self.item)
        let send = { [self] in
            index.deleteSearchableItems(withIdentifiers: gone) { [self] error in
                if let error { log.error("removing failed: \(error.localizedDescription)") }
                index.indexSearchableItems(items) { [self] error in
                    queue.async { [self] in
                        if let error {
                            // Tried again at the next change.
                            log.error("indexing failed: \(error.localizedDescription)")
                        } else {
                            for id in gone { known[id] = nil }
                            for page in changed { known[page.id] = page.digest }
                            save(known)
                            log.debug("indexed \(items.count), removed \(gone.count) in \(Date().timeIntervalSince(started) * 1000, format: .fixed(precision: 1)) ms")
                        }
                        indexed = known
                        finish()
                    }
                }
            }
        }
        if fresh {
            known = [:]
            index.deleteSearchableItems(withDomainIdentifiers: [Self.domain]) { [self] error in
                if let error { log.error("clearing failed: \(error.localizedDescription)") }
                queue.async(execute: send)
            }
        } else {
            send()
        }
    }

    /// On `queue`: a run ended; another if changes came meanwhile.
    private func finish() {
        running = false
        if again {
            again = false
            running = true
            run()
        }
    }

    /// Clear Spotlight's copy (the outline was replaced); the next update indexes it all.
    func clear() {
        queue.async { [self] in
            indexed = nil
            try? FileManager.default.removeItem(at: stateURL)
        }
        update()
    }

    private func load() -> [String: UInt64] {
        guard let data = try? Data(contentsOf: stateURL) else { return [:] }
        return (try? JSONDecoder().decode([String: UInt64].self, from: data)) ?? [:]
    }

    private func save(_ known: [String: UInt64]) {
        if let data = try? JSONEncoder().encode(known) {
            try? data.write(to: stateURL, options: .atomic)
        }
    }

    /// A page as Spotlight shows it: its title, the path to it, and its children's text to be
    /// found by. Tapping it opens the page (`FlowieApp`'s `CSSearchableItemActionType`).
    static func item(_ page: IndexedPage) -> CSSearchableItem {
        let attributes = CSSearchableItemAttributeSet(contentType: .text)
        let path = BulletView.line(page.path.map { $0.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : $0 }, limit: 80)
        if let day = page.day, let date = LaunchMemory.date(of: day) {
            attributes.title = date.formatted(date: .complete, time: .omitted)
            attributes.keywords = [day]
            attributes.contentCreationDate = date
        } else {
            attributes.title = page.title
        }
        attributes.displayName = attributes.title
        attributes.contentDescription = path
        attributes.textContent = page.content
        attributes.rankingHint = page.starred ? 1 : 0
        let item = CSSearchableItem(uniqueIdentifier: page.id, domainIdentifier: domain, attributeSet: attributes)
        item.associateAppEntity(PageEntity(id: page.id, title: attributes.title ?? page.title, path: path), priority: page.starred ? 1 : 0)
        return item
    }
}
