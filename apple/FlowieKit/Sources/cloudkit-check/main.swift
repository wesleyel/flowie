// Checks what can be checked without iCloud: a Flowie record makes a CKRecord and comes back
// the same, and the system fields of a saved one restore its id. Run by
// scripts/apple-package-check.sh.
import CloudKit
import FlowieCloudKit
import FlowieFFI
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok {
        FileHandle.standardError.write("FAILED: \(what)\n".data(using: .utf8)!)
        exit(1)
    }
}

let outline = Outline.inMemory()
outline.beginNativeSync(remote: "icloud")
let id = try outline.create(parent: nil, index: 0, text: "to iCloud").focus!.id
check(outline.unsentIds(limit: 10) == [id], "the new bullet waits to be sent")
let saves = outline.nextSaves(limit: 10)
check(saves.count == 1 && saves[0].id == id && saves[0].etag == nil, "one new record to save")

let zone = CKRecordZone.ID(zoneName: "Outline")
let record = CloudRecordCodec.record(for: saves[0], zone: zone)
check(record.recordID.recordName == id && record.recordType == CloudRecordCodec.recordType, "record id and type")
guard let back = CloudRecordCodec.wire(from: record) else { check(false, "the record reads back"); exit(1) }
check(back.envelope == saves[0].envelope && back.id == id && back.etag != nil, "the envelope comes back as sent")

// Saved: the system fields become the etag; the next save starts from them.
outline.onSaved(outcomes: [.saved(id: id, etag: back.etag!)])
check(outline.unsentIds(limit: 10).isEmpty, "nothing left to send")
_ = try outline.setText(id: id, text: "edited")
let next = outline.nextSaves(limit: 10)
check(next.count == 1 && next[0].etag == back.etag, "the next save is conditional on the saved version")
let again = CloudRecordCodec.record(for: next[0], zone: zone)
check(again.recordID == record.recordID, "restored from its system fields")
check((again.encryptedValues[CloudRecordCodec.field] as? String) == next[0].envelope, "with the new envelope")
print("FlowieCloudKit check: ok")
