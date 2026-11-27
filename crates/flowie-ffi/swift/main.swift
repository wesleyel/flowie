// A Swift program over the generated bindings: opens an outline, edits it, reads a page.
// Built and run by scripts/ffi-swift-smoke.sh; exits non-zero if anything is off.
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok {
        FileHandle.standardError.write("FAILED: \(what)\n".data(using: .utf8)!)
        exit(1)
    }
}

final class Waker: SyncObserver {
    func wake() {}
}

let dir = FileManager.default.temporaryDirectory.appendingPathComponent("flowie-ffi-smoke-\(UUID().uuidString)")
try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
let db = dir.appendingPathComponent("outline.sqlite").path

let outline = try Outline.open(db: db)
let first = try outline.create(parent: nil, index: 0, text: "From Swift")
let child = try outline.create(parent: first, index: 0, text: "A child, 中文 ✓")
try outline.setCompleted(id: child, completed: true)
let page = try outline.page(root: nil)
check(page.map(\.text) == ["From Swift", "A child, 中文 ✓"], "page rows: \(page.map(\.text))")
check(page[1].depth == 1 && page[1].completed && page[0].hasChildren, "row details")
try outline.delete(id: child)
check(try outline.page(root: nil).count == 1, "delete")
check(outline.undo(), "undo")
check(try outline.page(root: nil).count == 2, "undo brings it back")
do {
    try outline.setText(id: "not-an-id", text: "x")
    check(false, "an unknown id throws")
} catch FlowieError.NoSuchBullet {
}
// Return splits at the caret (UTF-8 byte offsets), Backspace at the start joins back.
let caret = try outline.enter(id: first, start: 4, end: 4, root: nil)
// The children stay with the first half; the rest goes after them.
check(try outline.page(root: nil).map(\.text) == ["From", "A child, 中文 ✓", " Swift"], "Return splits")
// Backspace joins it to the row shown above it: the child.
check(try outline.backspaceAtStart(id: caret.id, root: nil)?.id == child, "Backspace joins")
check(try outline.page(root: nil).map(\.text) == ["From", "A child, 中文 ✓ Swift"], "joined")
check(outline.undo() && outline.undo(), "two undo steps")
check(try outline.page(root: nil)[0].text == "From Swift", "undone")
check(outline.syncStatus() == .off, "sync is off")
try outline.flush()

// Reopened, it has what was saved.
let again = try Outline.open(db: db)
check(try again.page(root: nil).count == 2, "reopened")

// Sync against a server that isn't there: offline, the edits kept.
outline.startSync(server: "http://127.0.0.1:9", token: "t", observer: Waker())
let deadline = Date().addingTimeInterval(10)
var offline = false
while Date() < deadline && !offline {
    _ = outline.pumpSync()
    if case .offline = outline.syncStatus() { offline = true }
    Thread.sleep(forTimeInterval: 0.1)
}
check(offline, "an unreachable server shows as offline")
outline.stopSync()
print("flowie-ffi Swift smoke test: ok")
