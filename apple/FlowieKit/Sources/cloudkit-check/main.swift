// Checks what can be checked without iCloud: a Flowie record makes a CKRecord and comes back
// the same, the system fields of a saved one restore its id, the outline answers CloudKit's
// saves and conflicts through its host, and a build without the iCloud entitlements says so
// instead of touching CloudKit. Run by scripts/apple-package-check.sh.
import CloudKit
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
await otherHost.onFetched([back])
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
