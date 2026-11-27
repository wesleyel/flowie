import CloudKit

/// A Flowie record as CloudKit carries it: its name (a bullet's id, or `prefs`), the version
/// it was last seen at, and the envelope as JSON (wiki: Per-Node-Sync §4.1). The Rust engine
/// reads and writes these; nothing here looks inside the envelope.
public struct CloudWire: Sendable, Equatable, Codable {
    public var id: String
    /// The record's archived system fields; nil for a record never saved.
    public var etag: String?
    public var envelope: String

    public init(id: String, etag: String?, envelope: String) {
        self.id = id
        self.etag = etag
        self.envelope = envelope
    }
}

/// How CloudKit answered one save.
public enum CloudSaved: Sendable, Equatable {
    case saved(id: String, etag: String)
    /// serverRecordChanged: iCloud has a newer version; here it is.
    case conflict(server: CloudWire)
    /// Not saved for another reason; it is sent again later.
    case failed(id: String, error: String)
}

/// The CloudKit schema Flowie uses (docs/sync.md, wiki: Per-Node-Sync §5.2): in the private
/// database, one custom zone `Outline`, and one record type `Record` whose only field,
/// `envelope`, is an encrypted string. Every kind of Flowie record (bullets, the synced
/// preferences, kinds added later) is a `Record`; its kind is inside the envelope. So
/// Flowie's records can change without CloudKit's schema changing, which can only grow once
/// it is deployed to Production.
public enum CloudRecordCodec {
    public static let recordType = "Record"
    public static let field = "envelope"

    /// The CKRecord to save for `wire` in `zone`: from its system fields when it has been
    /// saved before (so the save is conditional on that version, and CloudKit answers a stale
    /// one with serverRecordChanged), else a new one. `fresh` ignores the system fields: the
    /// record is gone from iCloud (unknownItem) and is saved anew.
    public static func record(for wire: CloudWire, zone: CKRecordZone.ID, fresh: Bool = false) -> CKRecord {
        let id = CKRecord.ID(recordName: wire.id, zoneID: zone)
        let restored = fresh ? nil : wire.etag.flatMap(restore).flatMap { $0.recordID == id ? $0 : nil }
        let record = restored ?? CKRecord(recordType: recordType, recordID: id)
        record.encryptedValues[field] = wire.envelope
        return record
    }

    /// The Flowie record in a fetched or saved CKRecord; nil for one that isn't a `Record`
    /// with an envelope. A later version may add record types to the zone (image files as
    /// `Blob`, wiki: Per-Node-Sync §3.5) under names that are also a bullet's id: this version
    /// never reads them, so they can't stand for a bullet.
    public static func wire(from record: CKRecord) -> CloudWire? {
        guard record.recordType == recordType, let envelope = record.encryptedValues[field] as? String else { return nil }
        return CloudWire(id: record.recordID.recordName, etag: systemFields(of: record), envelope: envelope)
    }

    /// The Flowie records among fetched ones, the rest left out.
    public static func wires(from records: [CKRecord]) -> [CloudWire] {
        records.compactMap(wire)
    }

    /// A record's system fields (ids, change tag), archived as text.
    public static func systemFields(of record: CKRecord) -> String {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData.base64EncodedString()
    }

    /// The record an etag was archived from; nil for text that isn't one (an etag from
    /// another backend, kept from before the outline synced with iCloud).
    static func restore(_ etag: String) -> CKRecord? {
        guard let data = Data(base64Encoded: etag), let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }
}
