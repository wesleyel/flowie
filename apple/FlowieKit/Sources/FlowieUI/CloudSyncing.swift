import Foundation
import FlowieCloudKit
import FlowieFFI

/// Where iCloud sync stands, for Settings and the first-sync banner.
public enum ICloudStatus: Equatable, Sendable {
    /// Not started (a self-hosted server is syncing instead, or the app hasn't asked yet).
    case off
    /// Asking iCloud whether an account is signed in.
    case checking
    /// iCloud can't be used now, and why.
    case unavailable(CloudAccess)
    case syncing
    case synced(Date)
    /// The last fetch or send failed; CKSyncEngine tries again.
    case failed(String)

    /// Whether the outline syncs through iCloud (not a self-hosted server, and iCloud can be
    /// used): image files don't come with it yet (wiki: Per-Node-Sync §3.5).
    public var isOn: Bool {
        switch self {
        case .off, .unavailable: return false
        case .checking, .syncing, .synced, .failed: return true
        }
    }
}

/// The outline as CloudKit sync's host: flowie-ffi's native sync calls (flowie-doc's
/// `NativeSync`). Called on CKSyncEngine's queue; `Outline` locks itself, and what the app
/// shows is updated on the main actor.
public final class OutlineCloudHost: CloudSyncHost, @unchecked Sendable {
    let outline: Outline
    weak var store: OutlineStore?

    public init(outline: Outline, store: OutlineStore?) {
        self.outline = outline
        self.store = store
    }

    public func unsentIDs() async -> [String] {
        outline.unsentIds(limit: 10_000)
    }

    public func nextSaves(limit: Int) async -> [CloudWire] {
        outline.nextSaves(limit: UInt32(limit)).map { CloudWire(id: $0.id, etag: $0.etag, envelope: $0.envelope) }
    }

    public func onSaved(_ outcomes: [CloudSaved]) async {
        var merged = false
        outline.onSaved(outcomes: outcomes.map { outcome in
            switch outcome {
            case .saved(let id, let etag):
                return .saved(id: id, etag: etag)
            case .conflict(let server):
                merged = true
                return .conflict(server: WireRecord(id: server.id, etag: server.etag, envelope: server.envelope))
            case .failed(let id, let error):
                return .failed(id: id, error: error)
            }
        })
        if merged {
            await changed()
        }
    }

    public func onFetched(_ records: [CloudWire]) async {
        let wires = records.map { WireRecord(id: $0.id, etag: $0.etag, envelope: $0.envelope) }
        if outline.onFetched(records: wires, cursor: "icloud") {
            await changed()
        }
    }

    public func resendAll() async {
        outline.resendAll()
    }

    public func cloudEvent(_ event: CloudEvent) async {
        await MainActor.run { [weak store] in store?.cloudEvent(event) }
    }

    private func changed() async {
        await MainActor.run { [weak store] in store?.reloadAll() }
    }
}

extension OutlineStore {
    /// Sync through iCloud from now on, while an account is signed in (CKSyncEngine; wiki:
    /// Per-Node-Sync §5.2). CKSyncEngine's state is kept at `stateURL`. Stops a self-hosted
    /// server's sync: one backend at a time.
    public func startICloud(stateURL: URL) {
        guard #available(macOS 14.0, iOS 17.0, *) else {
            iCloud = .unavailable(.unsupported)
            return
        }
        stopICloud()
        if syncStatus != .off {
            stopSync()
        }
        cloudStateURL = stateURL
        let fresh = outline.beginNativeSync(remote: "icloud")
        // A new install (nothing here yet) shows that it is fetching the outline.
        fetchingFirst = outline.isEmpty()
        iCloud = .checking
        let controller = CloudController(host: OutlineCloudHost(outline: outline, store: self), stateURL: stateURL, fresh: fresh)
        cloud = controller
        controller.start()
    }

    /// Stop syncing through iCloud (what was sent and fetched stays).
    public func stopICloud() {
        guard #available(macOS 14.0, iOS 17.0, *), let controller = cloud as? CloudController else { return }
        controller.stop()
        cloud = nil
        outline.endNativeSync()
        fetchingFirst = false
        iCloud = .off
    }

    /// Fetch from iCloud now (a push came in, or the app came to the front).
    public func fetchFromICloud() async {
        guard #available(macOS 14.0, iOS 17.0, *), let controller = cloud as? CloudController else { return }
        await controller.fetchNow()
    }

    /// Hand iCloud sync what changed here (after each edit).
    func queueForICloud() {
        guard #available(macOS 14.0, iOS 17.0, *), let controller = cloud as? CloudController else { return }
        controller.queue(outline.unsentIds(limit: 10_000))
    }

    func cloudEvent(_ event: CloudEvent) {
        switch event {
        case .access(.available):
            if case .unavailable = iCloud { iCloud = .syncing }
            if iCloud == .checking { iCloud = .syncing }
        case .access(let access):
            iCloud = .unavailable(access)
            fetchingFirst = false
        case .fetched, .sent:
            fetchingFirst = false
            iCloud = outline.unsentIds(limit: 1).isEmpty ? .synced(Date()) : .syncing
        case .failed(let error):
            iCloud = .failed(error)
        }
    }
}
