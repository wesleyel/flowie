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

    // Return and Backspace, as on the desktop (byte offsets: "rye " is 4 bytes).
    store.enter(store.rows[2], from: 4, to: 4)
    check(store.rows.map(\.text) == ["groceries", "milk", "rye ", "bread"], "Return splits at the caret: \(store.rows.map(\.text))")
    check(store.focus == Focus(id: store.rows[3].id, offset: 0), "the caret goes to the new bullet")
    store.backspaceAtStart(store.rows[3])
    check(store.rows.map(\.text) == ["groceries", "milk", "rye bread"], "Backspace at the start joins it back")
    check(store.focus == Focus(id: store.rows[2].id, offset: 4), "the caret goes where they meet")
    store.undo()
    check(store.rows.map(\.text) == ["groceries", "milk", "rye ", "bread"], "undo of Backspace")
    store.undo()
    check(store.rows.map(\.text) == ["groceries", "milk", "rye bread"], "undo of Return")

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

try MainActor.assumeIsolated { try run() }
