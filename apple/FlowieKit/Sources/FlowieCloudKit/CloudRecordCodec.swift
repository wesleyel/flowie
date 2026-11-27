import CloudKit

/// A Flowie record as CloudKit carries it: its name (a bullet's id, or `prefs`), the version
/// it was last seen at, and the envelope as JSON (wiki: Per-Node-Sync §4.1). The Rust engine
/// reads and writes these; nothing here looks inside the envelope. An image file is one too,
/// with `file` and an empty envelope: a `Blob` record named after the file (§3.5).
public struct CloudWire: Sendable, Equatable, Codable {
    public var id: String
    /// The record's archived system fields; nil for a record never saved.
    public var etag: String?
    public var envelope: String
    public var file: CloudFile?

    public init(id: String, etag: String?, envelope: String, file: CloudFile? = nil) {
        self.id = id
        self.etag = etag
        self.envelope = envelope
        self.file = file
    }
}

/// An image file as a `Blob` record carries it: the file (a CKAsset; to send, in the image
/// folder; fetched, where CKSyncEngine downloaded it, readable while the record is handed
/// over), its size in pixels (0 when not known) and its MIME type.
public struct CloudFile: Sendable, Equatable, Codable {
    public var path: String
    public var width: UInt32
    public var height: UInt32
    public var mime: String

    public init(path: String, width: UInt32, height: UInt32, mime: String) {
        self.path = path
        self.width = width
        self.height = height
        self.mime = mime
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
/// database, one custom zone `Outline`, and two record types. `Record`, whose only field,
/// `envelope`, is an encrypted string: every kind of Flowie record (bullets, the synced
/// preferences, kinds added later) is a `Record`, its kind inside the envelope, so Flowie's
/// records can change without CloudKit's schema changing, which can only grow once it is
/// deployed to Production. And `Blob`, an image file (§3.5), named after it (its sha256 and
/// extension): `file`, a CKAsset (assets can't be encrypted fields, and CloudKit encrypts them
/// anyway), and `width`, `height` (Int64) and `mime` (String). A `Blob` is saved once and never
/// changes, since its name is its contents; nothing is ever deleted.
public enum CloudRecordCodec {
    public static let recordType = "Record"
    public static let field = "envelope"
    public static let blobType = "Blob"
    public static let blobFile = "file"
    public static let blobWidth = "width"
    public static let blobHeight = "height"
    public static let blobMime = "mime"

    /// The CKRecord to save for `wire` in `zone`: from its system fields when it has been
    /// saved before (so the save is conditional on that version, and CloudKit answers a stale
    /// one with serverRecordChanged), else a new one. `fresh` ignores the system fields: the
    /// record is gone from iCloud (unknownItem) and is saved anew.
    public static func record(for wire: CloudWire, zone: CKRecordZone.ID, fresh: Bool = false) -> CKRecord {
        let id = CKRecord.ID(recordName: wire.id, zoneID: zone)
        if let file = wire.file {
            // Always a new record: one there already is the same file (serverRecordChanged).
            let record = CKRecord(recordType: blobType, recordID: id)
            record[blobFile] = CKAsset(fileURL: URL(fileURLWithPath: file.path))
            record[blobWidth] = NSNumber(value: Int64(file.width))
            record[blobHeight] = NSNumber(value: Int64(file.height))
            record[blobMime] = file.mime as NSString
            return record
        }
        let restored = fresh ? nil : wire.etag.flatMap(restore).flatMap { $0.recordID == id ? $0 : nil }
        let record = restored ?? CKRecord(recordType: recordType, recordID: id)
        record.encryptedValues[field] = wire.envelope
        return record
    }

    /// The Flowie record in a fetched or saved CKRecord: a `Record` with an envelope, or a
    /// `Blob` with its file; nil for anything else. A later version may add record types to the
    /// zone, maybe under names that are also a bullet's id: this version never reads them, so
    /// they can't stand for a bullet. (Versions before `Blob` ignore it the same way.)
    public static func wire(from record: CKRecord) -> CloudWire? {
        if record.recordType == blobType {
            return blob(from: record)
        }
        guard record.recordType == recordType, let envelope = record.encryptedValues[field] as? String else { return nil }
        return CloudWire(id: record.recordID.recordName, etag: systemFields(of: record), envelope: envelope)
    }

    /// An image file's `Blob`, with the file CloudKit downloaded; nil without one. The outline
    /// keeps the file only if it is an image and matches the record's name.
    static func blob(from record: CKRecord) -> CloudWire? {
        guard let asset = record[blobFile] as? CKAsset, let url = asset.fileURL else { return nil }
        let size = { (key: String) -> UInt32 in UInt32(clamping: (record[key] as? NSNumber)?.int64Value ?? 0) }
        let file = CloudFile(path: url.path, width: size(blobWidth), height: size(blobHeight), mime: record[blobMime] as? String ?? "")
        return CloudWire(id: record.recordID.recordName, etag: nil, envelope: "", file: file)
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
