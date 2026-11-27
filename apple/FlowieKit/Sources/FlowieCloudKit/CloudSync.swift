import CloudKit
import Foundation
#if os(macOS)
import Security
#endif

/// What CloudKit sync needs from the outline: flowie-doc's `NativeSync` (wiki: Per-Node-Sync
/// §5.2), through flowie-ffi on iOS and through flowie-cloudkit's bridge on the Mac. The
/// outline decides what to send and merges what comes in; CKSyncEngine decides when.
public protocol CloudSyncHost: AnyObject, Sendable {
    /// Ids with something to send.
    func unsentIDs() async -> [String]
    /// The next records to save, at most `limit`. Each is in flight until `onSaved` answers.
    func nextSaves(limit: Int) async -> [CloudWire]
    func onSaved(_ outcomes: [CloudSaved]) async
    /// Records fetched from iCloud, to merge in.
    func onFetched(_ records: [CloudWire]) async
    /// Queue every record again: iCloud lost them (the zone was deleted, another account).
    func resendAll() async
    /// How sync is going, for the app to show.
    func cloudEvent(_ event: CloudEvent) async
}

/// Whether this device can sync through iCloud.
public enum CloudAccess: Sendable, Equatable {
    case available
    /// Not signed in to iCloud (or iCloud Drive is off for the account).
    case noAccount
    /// Parental controls or a device profile don't allow it.
    case restricted
    case temporarilyUnavailable
    /// This build isn't signed for Flowie's iCloud container: a build from source, or one
    /// signed without the iCloud entitlements.
    case notEntitled
    /// CKSyncEngine needs macOS 14 or iOS 17.
    case unsupported
    case failed(String)

    /// Flowie's iCloud container.
    public static let container = "iCloud.flowie.spokey.cn"

    /// Ask iCloud whether this device can sync (an account is signed in, nothing restricts it).
    public static func check(container identifier: String = container) async -> CloudAccess {
        guard entitled(identifier) else { return .notEntitled }
        guard #available(macOS 14.0, iOS 17.0, *) else { return .unsupported }
        do {
            switch try await CKContainer(identifier: identifier).accountStatus() {
            case .available: return .available
            case .noAccount: return .noAccount
            case .restricted: return .restricted
            case .temporarilyUnavailable: return .temporarilyUnavailable
            case .couldNotDetermine: return .failed("iCloud couldn’t say whether an account is signed in")
            @unknown default: return .failed("unknown account status")
            }
        } catch {
            return .failed(error.localizedDescription)
        }
    }

    /// Whether this process is signed with the entitlements for `identifier`. CloudKit ends a
    /// process that uses a container it isn't entitled to, so a Mac build run from source
    /// (`cargo run`, ad-hoc signed) must not touch CKContainer at all. iOS builds always carry
    /// them (apple/FlowieApp/Project.swift), and iOS has no API to read them.
    public static func entitled(_ identifier: String = container) -> Bool {
        #if os(macOS)
        guard let task = SecTaskCreateFromSelf(nil) else { return false }
        let services = SecTaskCopyValueForEntitlement(task, "com.apple.developer.icloud-services" as CFString, nil)
        let containers = SecTaskCopyValueForEntitlement(task, "com.apple.developer.icloud-container-identifiers" as CFString, nil)
        let cloudKit = (services as? [String])?.contains("CloudKit") == true || (services as? String) == "*"
        return cloudKit && (containers as? [String])?.contains(identifier) == true
        #else
        return true
        #endif
    }
}

/// What happened, for the app's sync status.
public enum CloudEvent: Sendable, Equatable {
    /// What iCloud can do now; sync runs only while it is `.available`.
    case access(CloudAccess)
    /// A fetch finished: this device has everything iCloud had.
    case fetched
    /// A send finished with every record saved.
    case sent
    /// A fetch or send failed (no network, iCloud busy); CKSyncEngine tries again.
    case failed(String)
}

/// Sync an outline with the user's private iCloud database through CKSyncEngine, which owns
/// scheduling, change tokens, retries, push and account changes. This only forwards its
/// callbacks to the host (wiki: Per-Node-Sync §5.2).
@available(macOS 14.0, iOS 17.0, *)
public final class CloudSync: CKSyncEngineDelegate, @unchecked Sendable {
    public static let zoneName = "Outline"
    /// Records per batch: CloudKit takes at most 400.
    static let batch = 400

    let host: CloudSyncHost
    let zone = CKRecordZone.ID(zoneName: CloudSync.zoneName)
    let stateURL: URL
    /// Called when the account changed under a running engine: sync stopped and should start
    /// again (with a new state).
    let onAccountChange: @Sendable () -> Void
    private let lock = NSLock()
    private var engineValue: CKSyncEngine?
    /// Sends wait for the first fetch (`releaseSends`), with the ids queued meanwhile.
    private var holding: Bool
    private var held = Set<String>()
    /// Records gone from iCloud (unknownItem): saved anew, without their system fields.
    private var gone = Set<String>()

    var engine: CKSyncEngine? {
        lock.withLock { engineValue }
    }

    /// Start syncing with `container`'s private database. CKSyncEngine's own state (change
    /// tokens, pending changes) is kept in the file at `stateURL`; `fresh` drops it, for a
    /// backend the outline didn't sync with last. `holdSends` sends nothing until
    /// `releaseSends`, for a first sync that looks at what iCloud has before sending.
    public init(host: CloudSyncHost, container: CKContainer, stateURL: URL, fresh: Bool, holdSends: Bool = false,
                onAccountChange: @escaping @Sendable () -> Void) {
        self.host = host
        self.stateURL = stateURL
        self.holding = holdSends
        self.onAccountChange = onAccountChange
        if fresh {
            try? FileManager.default.removeItem(at: stateURL)
        }
        let saved = (try? Data(contentsOf: stateURL)).flatMap { try? JSONDecoder().decode(CKSyncEngine.State.Serialization.self, from: $0) }
        let engine = CKSyncEngine(CKSyncEngine.Configuration(database: container.privateCloudDatabase, stateSerialization: saved, delegate: self))
        lock.withLock { engineValue = engine }
        if saved == nil {
            engine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: zone))])
        }
        Task {
            await self.queueUnsent()
            await self.fetchNow()
        }
    }

    /// Tell CKSyncEngine these records have something to send.
    public func queue(_ ids: [String]) {
        guard let engine, !ids.isEmpty else { return }
        let waiting = lock.withLock { () -> Bool in
            if holding { held.formUnion(ids) }
            return holding
        }
        if !waiting {
            engine.state.add(pendingRecordZoneChanges: ids.map { .saveRecord(CKRecord.ID(recordName: $0, zoneID: zone)) })
        }
    }

    /// Queue everything the outline has to send; after each save of the outline.
    public func queueUnsent() async {
        queue(await host.unsentIDs())
    }

    /// Send from now on, after a first sync's fetch.
    public func releaseSends() {
        let ids = lock.withLock { () -> [String] in
            holding = false
            defer { held.removeAll() }
            return Array(held)
        }
        queue(ids)
    }

    /// Fetch what changed in iCloud now (at launch, on a push), and say how it went.
    public func fetchNow() async {
        guard let engine else { return }
        do {
            try await engine.fetchChanges()
            await host.cloudEvent(.fetched)
        } catch {
            await host.cloudEvent(.failed(error.localizedDescription))
        }
    }

    public func stop() {
        lock.withLock { engineValue = nil }
    }

    public func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let update):
            if let data = try? JSONEncoder().encode(update.stateSerialization) {
                try? data.write(to: stateURL, options: .atomic)
            }
        case .fetchedDatabaseChanges(let changes):
            // The zone was deleted (the user deleted Flowie's iCloud data, or reset encrypted
            // data): make it again and send everything.
            if changes.deletions.contains(where: { $0.zoneID == zone }) {
                await zoneLost(syncEngine)
            }
        case .fetchedRecordZoneChanges(let changes):
            let records = changes.modifications.compactMap { CloudRecordCodec.wire(from: $0.record) }
            if !records.isEmpty {
                await host.onFetched(records)
            }
        case .sentRecordZoneChanges(let sent):
            var outcomes = sent.savedRecords.map { CloudSaved.saved(id: $0.recordID.recordName, etag: CloudRecordCodec.systemFields(of: $0)) }
            lock.withLock { gone.subtract(sent.savedRecords.map(\.recordID.recordName)) }
            var problem: String?
            var zoneMissing = false
            for failure in sent.failedRecordSaves {
                let id = failure.record.recordID.recordName
                switch failure.error.code {
                case .serverRecordChanged:
                    if let server = failure.error.serverRecord, let wire = CloudRecordCodec.wire(from: server) {
                        outcomes.append(.conflict(server: wire))
                    } else {
                        outcomes.append(.failed(id: id, error: failure.error.localizedDescription))
                    }
                case .unknownItem:
                    lock.withLock { _ = gone.insert(id) }
                    outcomes.append(.failed(id: id, error: failure.error.localizedDescription))
                case .zoneNotFound, .userDeletedZone:
                    zoneMissing = true
                    outcomes.append(.failed(id: id, error: failure.error.localizedDescription))
                default:
                    problem = problem ?? failure.error.localizedDescription
                    outcomes.append(.failed(id: id, error: failure.error.localizedDescription))
                }
            }
            await host.onSaved(outcomes)
            if zoneMissing {
                await zoneLost(syncEngine)
            } else {
                await queueUnsent()
            }
            await host.cloudEvent(problem.map { .failed($0) } ?? .sent)
        case .accountChange(let change):
            switch change.changeType {
            case .signIn:
                // A first start, or signed in again: whatever iCloud has, send everything.
                await host.resendAll()
                await queueUnsent()
            case .signOut, .switchAccounts:
                // This device's outline stays; sync starts again for the account signed in now
                // (if any), from a new state.
                stop()
                try? FileManager.default.removeItem(at: stateURL)
                onAccountChange()
            @unknown default:
                break
            }
        default:
            break
        }
    }

    private func zoneLost(_ syncEngine: CKSyncEngine) async {
        syncEngine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: zone))])
        lock.withLock { gone.removeAll() }
        await host.resendAll()
        // Their system fields name records iCloud no longer has: save them anew.
        let ids = await host.unsentIDs()
        lock.withLock { gone.formUnion(ids) }
        queue(ids)
    }

    public func nextRecordZoneChangeBatch(_ context: CKSyncEngine.SendChangesContext, syncEngine: CKSyncEngine) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let pending = syncEngine.state.pendingRecordZoneChanges.filter { context.options.scope.contains($0) }
        if lock.withLock({ holding }) {
            return nil
        }
        let saves = pending.isEmpty ? [] : await host.nextSaves(limit: CloudSync.batch)
        // The outline's outbox is what counts; the pending list only wakes CKSyncEngine. What
        // isn't in this batch is in flight, sent already or queued again after it.
        let ids = Set(saves.map(\.id))
        let stale = pending.filter { change in
            if case .saveRecord(let id) = change { return !ids.contains(id.recordName) }
            return false
        }
        syncEngine.state.remove(pendingRecordZoneChanges: stale)
        guard !saves.isEmpty else { return nil }
        let changes: [CKSyncEngine.PendingRecordZoneChange] = saves.map { .saveRecord(CKRecord.ID(recordName: $0.id, zoneID: zone)) }
        syncEngine.state.add(pendingRecordZoneChanges: changes)
        let byID = Dictionary(saves.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let gone = lock.withLock { self.gone }
        let zone = self.zone
        let batch = await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: changes) { id in
            byID[id.recordName].map { CloudRecordCodec.record(for: $0, zone: zone, fresh: gone.contains($0.id)) }
        }
        // Saves that didn't fit go back to the outline, to come again in the next batch.
        let included = Set(batch?.recordsToSave.map(\.recordID.recordName) ?? [])
        let left = saves.filter { !included.contains($0.id) }
        if !left.isEmpty {
            await host.onSaved(left.map { .failed(id: $0.id, error: "not in this batch") })
        }
        return batch
    }
}

/// Runs `CloudSync` while iCloud is available: checks the account at start and whenever it
/// changes, starts the engine when an account is there, and starts it again after a switch.
/// The iOS app and the Mac app's bridge each keep one per open outline.
@available(macOS 14.0, iOS 17.0, *)
public final class CloudController: @unchecked Sendable {
    let host: CloudSyncHost
    let containerID: String
    let stateURL: URL
    private let lock = NSLock()
    private var syncValue: CloudSync?
    private var fresh: Bool
    private var holdSends: Bool
    private var stopped = false
    private var observer: NSObjectProtocol?

    public var sync: CloudSync? {
        lock.withLock { syncValue }
    }

    /// `fresh`: the outline didn't sync with iCloud last (drop CKSyncEngine's saved state).
    /// `holdSends`: send nothing until `releaseSends` (a first sync).
    public init(host: CloudSyncHost, containerID: String = CloudAccess.container, stateURL: URL, fresh: Bool, holdSends: Bool = false) {
        self.host = host
        self.containerID = containerID
        self.stateURL = stateURL
        self.fresh = fresh
        self.holdSends = holdSends
    }

    /// Check the account and start syncing if it can. Listens for account changes from then on.
    public func start() {
        if observer == nil, CloudAccess.entitled(containerID) {
            observer = NotificationCenter.default.addObserver(forName: .CKAccountChanged, object: nil, queue: nil) { [weak self] _ in
                Task { await self?.check() }
            }
        }
        Task { await check() }
    }

    /// Check the account again, and start or stop the engine to match.
    public func check() async {
        let access = await CloudAccess.check(container: containerID)
        await host.cloudEvent(.access(access))
        let start = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            if access != .available {
                syncValue?.stop()
                syncValue = nil
                return false
            }
            return syncValue == nil
        }
        guard start else { return }
        let (fresh, hold) = lock.withLock { () -> (Bool, Bool) in
            defer { self.fresh = false }
            return (self.fresh, holdSends)
        }
        let sync = CloudSync(host: host, container: CKContainer(identifier: containerID), stateURL: stateURL, fresh: fresh,
                             holdSends: hold) { [weak self] in
            self?.accountChanged()
        }
        lock.withLock {
            if stopped {
                sync.stop()
            } else {
                syncValue = sync
            }
        }
    }

    private func accountChanged() {
        lock.withLock {
            syncValue = nil
            fresh = true
        }
        Task { await check() }
    }

    public func queue(_ ids: [String]) {
        sync?.queue(ids)
    }

    public func queueUnsent() async {
        await sync?.queueUnsent()
    }

    public func releaseSends() {
        lock.withLock { holdSends = false }
        sync?.releaseSends()
    }

    public func fetchNow() async {
        await sync?.fetchNow()
    }

    public func stop() {
        lock.withLock {
            stopped = true
            syncValue?.stop()
            syncValue = nil
        }
        if let observer {
            NotificationCenter.default.removeObserver(observer)
        }
    }
}
