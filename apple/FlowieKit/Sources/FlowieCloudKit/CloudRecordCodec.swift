import CloudKit
import FlowieFFI

/// A Flowie record as a CKRecord, and back. The envelope (per-node-sync.md §4.1) goes in one
/// encrypted field, so CloudKit's schema never changes when Flowie's records do. A record's
/// etag is its archived system fields: saving from them makes the save conditional on that
/// version, and CloudKit answers a stale one with serverRecordChanged.
public enum CloudRecordCodec {
    public static let recordType = "Record"
    public static let field = "envelope"

    /// The CKRecord to save for `wire` in `zone`: from its system fields when it has been
    /// saved before, else a new one.
    public static func record(for wire: WireRecord, zone: CKRecordZone.ID) -> CKRecord {
        let record = wire.etag.flatMap(restore) ?? CKRecord(recordType: recordType, recordID: CKRecord.ID(recordName: wire.id, zoneID: zone))
        record.encryptedValues[field] = wire.envelope
        return record
    }

    /// The Flowie record in a fetched or saved CKRecord; nil for one that isn't Flowie's.
    public static func wire(from record: CKRecord) -> WireRecord? {
        guard record.recordType == recordType, let envelope = record.encryptedValues[field] as? String else { return nil }
        return WireRecord(id: record.recordID.recordName, etag: systemFields(of: record), envelope: envelope)
    }

    /// A record's system fields (ids, change tag), archived as text.
    public static func systemFields(of record: CKRecord) -> String {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData.base64EncodedString()
    }

    static func restore(_ etag: String) -> CKRecord? {
        guard let data = Data(base64Encoded: etag), let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }
}
