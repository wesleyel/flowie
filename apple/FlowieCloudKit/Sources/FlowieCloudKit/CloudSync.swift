import CloudKit
import FlowieFFI

/// Sync an outline with the user's private iCloud database through CKSyncEngine, which owns
/// scheduling, change tokens, retries, push and account changes. This only forwards its
/// callbacks to flowie-ffi's engine (per-node-sync.md §5.2).
@available(macOS 14.0, iOS 17.0, *)
public final class CloudSync: CKSyncEngineDelegate, @unchecked Sendable {
    public static let zoneName = "Outline"
    /// Records per batch: CloudKit takes at most 400.
    static let batch: UInt32 = 400

    let outline: Outline
    let zone = CKRecordZone.ID(zoneName: CloudSync.zoneName)
    let stateURL: URL
    /// Called (on CKSyncEngine's queue) when records from iCloud changed the outline.
    let onChange: @Sendable () -> Void
    /// Set once in `init`, cleared by `stop`; CKSyncEngine calls in on its own queue.
    private let lock = NSLock()
    private var engineValue: CKSyncEngine?
    var engine: CKSyncEngine? {
        get { lock.withLock { engineValue } }
        set { lock.withLock { engineValue = newValue } }
    }

    /// Start syncing `outline` with `container`'s private database. CKSyncEngine's own state
    /// is kept in the file at `stateURL`.
    public init(outline: Outline, container: CKContainer, stateURL: URL, onChange: @escaping @Sendable () -> Void) {
        self.outline = outline
        self.stateURL = stateURL
        self.onChange = onChange
        outline.beginNativeSync(remote: "icloud")
        let saved = (try? Data(contentsOf: stateURL)).flatMap { try? JSONDecoder().decode(CKSyncEngine.State.Serialization.self, from: $0) }
        let engine = CKSyncEngine(CKSyncEngine.Configuration(database: container.privateCloudDatabase, stateSerialization: saved, delegate: self))
        self.engine = engine
        if saved == nil {
            engine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: zone))])
        }
        queueUnsent()
    }

    /// Tell CKSyncEngine what has changed here; call after each save of the outline.
    public func queueUnsent() {
        guard let engine else { return }
        let ids = outline.unsentIds(limit: 10_000)
        engine.state.add(pendingRecordZoneChanges: ids.map { .saveRecord(CKRecord.ID(recordName: $0, zoneID: zone)) })
    }

    public func stop() {
        outline.endNativeSync()
        engine = nil
    }

    public func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let update):
            if let data = try? JSONEncoder().encode(update.stateSerialization) {
                try? data.write(to: stateURL, options: .atomic)
            }
        case .fetchedRecordZoneChanges(let changes):
            let records = changes.modifications.compactMap { CloudRecordCodec.wire(from: $0.record) }
            if outline.onFetched(records: records, cursor: "") {
                onChange()
            }
        case .sentRecordZoneChanges(let sent):
            var outcomes = sent.savedRecords.map { SaveOutcome.saved(id: $0.recordID.recordName, etag: CloudRecordCodec.systemFields(of: $0)) }
            for failure in sent.failedRecordSaves {
                let id = failure.record.recordID.recordName
                if failure.error.code == .serverRecordChanged, let server = failure.error.serverRecord, let wire = CloudRecordCodec.wire(from: server) {
                    outcomes.append(.conflict(server: wire))
                } else {
                    outcomes.append(.failed(id: id, error: failure.error.localizedDescription))
                }
            }
            outline.onSaved(outcomes: outcomes)
            queueUnsent()
            if sent.failedRecordSaves.contains(where: { $0.error.code == .serverRecordChanged }) {
                onChange()
            }
        case .accountChange:
            // Signed out or another account: stop; the app offers to set sync up again.
            stop()
        default:
            break
        }
    }

    public func nextRecordZoneChangeBatch(_ context: CKSyncEngine.SendChangesContext, syncEngine: CKSyncEngine) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let pending = syncEngine.state.pendingRecordZoneChanges.filter { context.options.scope.contains($0) }
        guard !pending.isEmpty else { return nil }
        let saves = outline.nextSaves(limit: CloudSync.batch)
        let byID = Dictionary(saves.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let zone = self.zone
        return await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: pending) { id in
            guard let wire = byID[id.recordName] else {
                // Nothing to send for it any more (sent already, or nothing changed).
                syncEngine.state.remove(pendingRecordZoneChanges: [.saveRecord(id)])
                return nil
            }
            return CloudRecordCodec.record(for: wire, zone: zone)
        }
    }
}
