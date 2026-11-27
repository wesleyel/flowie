// Checks FlowieUI's store without drawing anything: pages, edits, undo, and sync to a server
// that isn't there. Run by scripts/apple-package-check.sh.
import FlowieFFI
import FlowieUI
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok {
        FileHandle.standardError.write("FAILED: \(what)\n".data(using: .utf8)!)
        exit(1)
    }
}

@MainActor
func run() throws {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("flowie-ui-check-\(UUID().uuidString)")
    let store = try OutlineStore.open(databaseAt: dir.appendingPathComponent("outline.sqlite"))
    store.add("groceries")
    check(store.rows.map(\.text) == ["groceries"], "added at the end of Home")
    let groceries = store.rows[0]
    _ = try store.outline.create(parent: groceries.id, index: 0, text: "milk")
    store.reload()
    check(store.rows.count == 2 && store.rows[0].hasChildren, "the child shows under it")

    store.open(store.rows[0])
    check(store.root == groceries.id && store.rows.map(\.text) == ["milk"], "a bullet opens as its page")
    store.add("bread")
    check(store.rows.map(\.text) == ["milk", "bread"], "adding goes to the page shown")
    store.toggle(store.rows[0])
    check(store.rows[0].completed, "ticked off")
    store.setText(store.rows[1], "rye bread")
    check(store.rows[1].text == "rye bread", "edited")
    // Typing reads back only its row: a bullet added behind the store's back doesn't show yet.
    _ = try store.outline.create(parent: store.root, index: 2, text: "behind")
    store.setText(store.rows[1], "rye bread 中文 ✓")
    check(store.rows.map(\.text) == ["milk", "rye bread 中文 ✓"], "typing reads only its row: \(store.rows.map(\.text))")
    store.reload()
    check(store.rows.count == 3, "the page read again shows it")
    store.delete(store.rows[2])
    store.setText(store.rows[1], "rye bread")
    store.delete(store.rows[1])
    check(store.rows.count == 1, "deleted")
    store.undo()
    check(store.rows.count == 2, "undo brings it back")
    store.back()
    check(store.root == nil && store.rows.map(\.text) == ["groceries", "milk", "rye bread"], "back to Home: \(store.rows.map(\.text))")
    store.indent(store.rows[2])
    check(store.rows.map(\.depth) == [0, 1, 2], "indented under milk")
    store.outdent(store.rows[2])
    store.outdent(store.rows[2])
    check(store.rows.map(\.depth) == [0, 1, 0], "outdented to Home")
    store.undo()
    check(store.rows.map(\.depth) == [0, 1, 1], "undo outdents one step back")

    // Return and Backspace, as on the desktop (UTF-16 offsets).
    store.enter(store.rows[2], from: 4, to: 4)
    check(store.rows.map(\.text) == ["groceries", "milk", "rye ", "bread"], "Return splits at the caret: \(store.rows.map(\.text))")
    check(store.focus == Focus(id: store.rows[3].id, start: 0, end: 0), "the caret goes to the new bullet")
    store.backspaceAtStart(store.rows[3])
    check(store.rows.map(\.text) == ["groceries", "milk", "rye bread"], "Backspace at the start joins it back")
    check(store.focus == Focus(id: store.rows[2].id, start: 4, end: 4), "the caret goes where they meet")
    store.undo()
    check(store.rows.map(\.text) == ["groceries", "milk", "rye ", "bread"], "undo of Backspace")
    store.undo()
    check(store.rows.map(\.text) == ["groceries", "milk", "rye bread"], "undo of Return")
    check(store.focus == Focus(id: store.rows[2].id, start: 4, end: 4), "undo puts the caret back")
    // Offsets count UTF-16, past CJK and emoji: 日 is one unit, 😀 two.
    store.setText(store.rows[1], "日😀milk")
    store.enter(store.rows[1], from: 3, to: 3)
    check(store.rows.map(\.text) == ["groceries", "日😀", "milk", "rye bread"], "Return after an emoji: \(store.rows.map(\.text))")
    store.backspaceAtStart(store.rows[2])
    check(store.focus == Focus(id: store.rows[1].id, start: 3, end: 3), "the caret after the emoji")

    // Move up and down, as the desktop's Move Up / Move Down.
    check(store.rows.map(\.text) == ["groceries", "日😀milk", "rye bread"], "before moving: \(store.rows.map(\.text))")
    store.moveUp(store.rows[2])
    check(store.rows.map(\.text) == ["groceries", "rye bread", "日😀milk"], "moved up: \(store.rows.map(\.text))")
    store.moveDown(store.rows[1])
    check(store.rows.map(\.text) == ["groceries", "日😀milk", "rye bread"], "moved down: \(store.rows.map(\.text))")

    // A page view hears each change inside the edit, with the caret's new place already
    // known: so a second Return right after the first goes to the new row.
    var heard: [(PageChange, Focus?)] = []
    store.onChange = { change in heard.append((change, store.focus)) }
    store.enter(store.rows[2], from: 9, to: 9)
    let first = store.rows[3]
    check(heard.count == 1 && heard[0].0 == .rows && heard[0].1 == Focus(id: first.id, start: 0, end: 0), "Return tells the page where the caret goes: \(heard)")
    store.enter(first, from: 0, to: 0)
    check(store.rows.count == 5 && heard.last?.1?.id == store.rows[4].id, "a second Return splits the new row: \(store.rows.map(\.text))")
    heard = []
    store.setText(store.rows[4], "jam")
    check(heard.count == 1 && heard[0].0 == .content([store.rows[4].id]), "typing tells only its row: \(heard)")
    store.onChange = nil
    store.add("", focus: true)
    check(store.focus?.id == store.rows.last?.id && store.rows.last?.text == "", "a row added at the end takes the caret")

    store.startSync(server: "http://127.0.0.1:9", token: "t")
    let deadline = Date().addingTimeInterval(10)
    while Date() < deadline {
        if case .offline = store.syncStatus { break }
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    }
    guard case .offline = store.syncStatus else { check(false, "an unreachable server shows as offline: \(store.syncStatus)"); return }
    store.stopSync()
    check(store.syncStatus == .off, "sync stops")
    print("FlowieUI check: ok")
}

/// The iOS app's outline moves into the App Group container whole, and only when the
/// container has none.
@MainActor
func checkLocation() throws {
    let fm = FileManager.default
    let base = fm.temporaryDirectory.appendingPathComponent("flowie-location-check-\(UUID().uuidString)")
    let old = base.appendingPathComponent("support/Flowie")
    let new = base.appendingPathComponent("group/Library/Application Support/Flowie")
    do {
        let store = try OutlineStore.open(databaseAt: old.appendingPathComponent(OutlineLocation.fileName))
        store.add("kept across the move")
        // Not flushed: what is queued is saved when the outline closes.
    }
    try fm.createDirectory(at: old.appendingPathComponent("flowie.sqlite.images/ab"), withIntermediateDirectories: true)
    try Data("img".utf8).write(to: old.appendingPathComponent("flowie.sqlite.images/ab/abcd"))
    let moved = OutlineLocation.adopt(from: old, to: new)
    check(moved == new.appendingPathComponent(OutlineLocation.fileName), "the database is in the container")
    check(!fm.fileExists(atPath: old.path), "the old folder moved")
    check(fm.fileExists(atPath: new.appendingPathComponent("flowie.sqlite.images/ab/abcd").path), "images moved with it")
    let store = try OutlineStore.open(databaseAt: moved)
    check(store.rows.map(\.text) == ["kept across the move"], "the outline reads the same: \(store.rows.map(\.text))")

    // Both places have one: the container's is used, the old one is left alone.
    try fm.createDirectory(at: old, withIntermediateDirectories: true)
    try Data("old".utf8).write(to: old.appendingPathComponent(OutlineLocation.fileName))
    check(OutlineLocation.adopt(from: old, to: new) == moved, "the container's database wins")
    check(fm.fileExists(atPath: old.appendingPathComponent(OutlineLocation.fileName).path), "and the old one stays")

    // A container folder without a database is kept aside, not replaced.
    let other = base.appendingPathComponent("group2/Flowie")
    try fm.createDirectory(at: other, withIntermediateDirectories: true)
    try Data("x".utf8).write(to: other.appendingPathComponent("stray.txt"))
    OutlineLocation.adopt(from: old, to: other)
    check(fm.fileExists(atPath: other.appendingPathComponent(OutlineLocation.fileName).path), "moved into the container")
    let aside = try fm.contentsOfDirectory(atPath: other.deletingLastPathComponent().path).filter { $0.hasPrefix("Flowie (before move") }
    check(aside.count == 1, "the stray folder is kept aside: \(aside)")
    try? fm.removeItem(at: base)
    print("Outline location check: ok")
}

try MainActor.assumeIsolated {
    try run()
    try checkLocation()
}
