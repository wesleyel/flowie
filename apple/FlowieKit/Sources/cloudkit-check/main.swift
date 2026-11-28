// Checks what can be checked without iCloud: a Flowie record makes a CKRecord and comes back
// the same, so does an image file as a `Blob`, the system fields of a saved one restore its id, the outline answers CloudKit's
// saves and conflicts through its host, and a build without the iCloud entitlements says so
// instead of touching CloudKit. Run by scripts/apple-package-check.sh.
import CloudKit
import CryptoKit
import FlowieCloudKit
import FlowieFFI
import FlowieUI
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok {
        FileHandle.standardError.write("FAILED: \(what)\n".data(using: .utf8)!)
        exit(1)
    }
}

let zone = CKRecordZone.ID(zoneName: CloudSync.zoneName)

// This process isn't signed for iCloud: CloudKit is never asked (it would end the process).
check(!CloudAccess.entitled(), "an unsigned build isn't entitled to iCloud")
check(await CloudAccess.check() == .notEntitled, "and says iCloud is unavailable in this build")

let outline = Outline.inMemory()
check(outline.beginNativeSync(remote: "icloud"), "a new outline starts fresh with iCloud")
let host = OutlineCloudHost(outline: outline, store: nil)
let id = try outline.create(parent: nil, index: 0, text: "to iCloud").focus!.id
check(await host.unsentIDs() == [id], "the new bullet waits to be sent")
let saves = await host.nextSaves(limit: 10)
check(saves.count == 1 && saves[0].id == id && saves[0].etag == nil, "one new record to save")

let record = CloudRecordCodec.record(for: saves[0], zone: zone)
check(record.recordID.recordName == id && record.recordType == CloudRecordCodec.recordType, "record id and type")
guard let back = CloudRecordCodec.wire(from: record) else { check(false, "the record reads back"); exit(1) }
check(back.envelope == saves[0].envelope && back.id == id && back.etag != nil, "the envelope comes back as sent")

// A later version's record types (image files as `Blob`) are never read, not even one named
// like a bullet and carrying its envelope; nor is a `Record` without one.
let blob = CKRecord(recordType: "Blob", recordID: record.recordID)
blob.encryptedValues[CloudRecordCodec.field] = saves[0].envelope
check(CloudRecordCodec.wire(from: blob) == nil, "a Blob is no Flowie record")
check(CloudRecordCodec.wire(from: CKRecord(recordType: CloudRecordCodec.recordType, recordID: record.recordID)) == nil,
      "a Record without an envelope is none either")
check(CloudRecordCodec.wires(from: [blob, record]) == [back], "only the Record is passed on")
let later = CKRecord(recordType: "Attachment", recordID: CKRecord.ID(recordName: "x", zoneID: zone))
check(CloudRecordCodec.wires(from: [later]).isEmpty, "nor a record type this version doesn't know")

// An image file is a `Blob` named after it: its file an asset, its size and type beside it.
let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("flowie-cloudkit-check-\(UUID().uuidString)")
try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: scratch) }
let gif = Data([0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 6, 0, 4, 0, 1])
let gifName = SHA256.hash(data: gif).map { String(format: "%02x", $0) }.joined() + ".gif"
let asset = scratch.appendingPathComponent("asset")
try gif.write(to: asset)
let fileWire = CloudWire(id: gifName, etag: nil, envelope: "", file: CloudFile(path: asset.path, width: 6, height: 4, mime: "image/gif"))
let blobRecord = CloudRecordCodec.record(for: fileWire, zone: zone, fresh: false)
check(blobRecord.recordType == CloudRecordCodec.blobType && blobRecord.recordID.recordName == gifName, "a Blob named after the file")
check((blobRecord[CloudRecordCodec.blobFile] as? CKAsset)?.fileURL == asset, "with the file as its asset")
check((blobRecord[CloudRecordCodec.blobWidth] as? NSNumber)?.int64Value == 6 && (blobRecord[CloudRecordCodec.blobHeight] as? NSNumber)?.int64Value == 4
      && blobRecord[CloudRecordCodec.blobMime] as? String == "image/gif", "and its size and type in plain fields")
check(blobRecord.encryptedValues[CloudRecordCodec.field] == nil, "and no envelope")
check(CloudRecordCodec.wire(from: blobRecord) == fileWire, "it reads back as the same file")
check(CloudRecordCodec.wires(from: [blobRecord, record]) == [fileWire, back], "both kinds are passed on")
check(CloudRecordCodec.wire(from: CKRecord(recordType: CloudRecordCodec.blobType, recordID: blobRecord.recordID)) == nil,
      "a Blob without its file is none")
var stale = fileWire
stale.etag = back.etag
check(CloudRecordCodec.record(for: stale, zone: zone).recordChangeTag == nil, "a Blob is always saved as a new record")

// The outline keeps a fetched file that matches its name, in the image folder, and drops one
// that doesn't.
let db = scratch.appendingPathComponent("outline.sqlite")
let disk = try Outline.open(db: db.path)
_ = disk.beginNativeSync(remote: "icloud")
let diskHost = OutlineCloudHost(outline: disk, store: nil)
let kept = URL(fileURLWithPath: db.path + ".images").appendingPathComponent(String(gifName.prefix(2))).appendingPathComponent(gifName)
var wrong = fileWire
wrong.id = String(repeating: "0", count: 64) + ".gif"
check(await diskHost.onFetched([wrong])[wrong.id]?.hasPrefix("dropped: the file is \(gifName)") == true, "and says why")
check(!FileManager.default.fileExists(atPath: URL(fileURLWithPath: db.path + ".images").appendingPathComponent("00").path),
      "a file that isn't what its name says is dropped")
check(await diskHost.onFetched([fileWire]) == [gifName: "kept"], "a file that matches is kept")
check(FileManager.default.contents(atPath: kept.path) == gif, "the file is kept under its name")
let diskRows = try disk.page(root: nil)
check(disk.unsentIds(limit: 10).isEmpty && diskRows.isEmpty, "and is no bullet")
check(await diskHost.missingFiles(limit: 10).isEmpty, "no row is missing its file")

// Saved: the system fields become the etag; the next save starts from them.
await host.onSaved([.saved(id: id, etag: back.etag!)])
check(await host.unsentIDs().isEmpty, "nothing left to send")
_ = try outline.setText(id: id, text: "edited")
let next = await host.nextSaves(limit: 10)
check(next.count == 1 && next[0].etag == back.etag, "the next save is conditional on the saved version")
let again = CloudRecordCodec.record(for: next[0], zone: zone)
check(again.recordID == record.recordID, "restored from its system fields")
check((again.encryptedValues[CloudRecordCodec.field] as? String) == next[0].envelope, "with the new envelope")
check(CloudRecordCodec.record(for: next[0], zone: zone, fresh: true).recordChangeTag == nil, "a record gone from iCloud is saved anew")
var foreign = next[0]
foreign.etag = "17"
check(CloudRecordCodec.record(for: foreign, zone: zone).recordID == record.recordID, "an etag from a server is no system fields")

// Another device saved first: CloudKit answers with its record, which merges in.
let other = Outline.inMemory()
_ = other.beginNativeSync(remote: "icloud")
let otherHost = OutlineCloudHost(outline: other, store: nil)
check(await otherHost.onFetched([back]).isEmpty, "a bullet's record is no file")
_ = try other.setNote(id: id, note: "from the other device")
let theirs = await otherHost.nextSaves(limit: 10)
check(theirs.count == 1, "the other device sends its note")
await host.onSaved([.conflict(server: CloudWire(id: id, etag: "their version", envelope: theirs[0].envelope))])
let row = try outline.page(root: nil)[0]
check(row.text == "edited" && row.note == "from the other device", "both edits are kept")
check(await host.unsentIDs() == [id], "and the merged record goes out again")

// The zone was deleted: everything is sent again.
await host.onSaved([.saved(id: id, etag: back.etag!)])
await host.resendAll()
check(await host.unsentIDs() == [id], "everything waits to be sent again")
print("FlowieCloudKit check: ok")
