// Checks FlowieUI's store without drawing anything: pages, edits, undo, the shell's journal,
// stars, search, Move To and Recently Deleted, the launch rule, and sync to a server that isn't
// there. Run by scripts/apple-package-check.sh.
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
    let docs = try OutlineStore.open(databaseAt: dir.appendingPathComponent("outline.sqlite"))
    let home = docs.page(.home)
    home.add("groceries")
    check(home.rows.map(\.text) == ["groceries"], "added at the end of Home")
    let groceries = home.rows[0]
    _ = try docs.outline.create(parent: groceries.id, index: 0, text: "milk")
    home.reload()
    check(home.rows.count == 2 && home.rows[0].hasChildren, "the child shows under it")

    var store = docs.page(.bullet(groceries.id))
    check(store.root == groceries.id && store.rows.map(\.text) == ["milk"], "a bullet opens as its page")
    check(store.info?.title == "groceries" && store.info?.path.isEmpty == true, "the page knows its bullet")
    store.add("bread")
    check(store.rows.map(\.text) == ["milk", "bread"], "adding goes to the page shown")
    store.toggle(store.rows[0])
    check(store.rows[0].completed, "ticked off")
    store.setText(store.rows[1], "rye bread")
    check(store.rows[1].text == "rye bread", "edited")
    // Typing reads back only its row: a bullet added behind the store's back doesn't show yet.
    _ = try docs.outline.create(parent: store.root, index: 2, text: "behind")
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
    // Home took each edit made on the bullet's page.
    check(home.rows.map(\.text) == ["groceries", "milk", "rye bread"], "Home follows another page's edits: \(home.rows.map(\.text))")
    store = home
    check(store.root == nil, "Home")
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

    try checkShell(docs, home: home)

    docs.startSync(server: "http://127.0.0.1:9", token: "t")
    let deadline = Date().addingTimeInterval(10)
    while Date() < deadline {
        if case .offline = docs.syncStatus { break }
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    }
    guard case .offline = docs.syncStatus else { check(false, "an unreachable server shows as offline: \(docs.syncStatus)"); return }
    docs.stopSync()
    check(docs.syncStatus == .off, "sync stops")
    print("FlowieUI check: ok")
}

/// What the shell (step 4) builds on: the journal, folding, stars, search, Move To, Hide
/// Completed, Recently Deleted, and pages off screen catching up.
@MainActor
func checkShell(_ docs: OutlineStore, home: PageModel) throws {
    // A day of the journal shows nothing, and makes nothing, until written in.
    let day = docs.page(.day("2026-11-27"))
    check(day.isUnmadeDay && day.rows.isEmpty && day.root == nil, "a day not made yet is empty")
    let before = home.rows.count
    day.add("", focus: true)
    check(!day.isUnmadeDay && day.rows.count == 1, "writing makes the day, with its blank first line: \(day.rows.map(\.text))")
    check(day.focus?.id == day.rows[0].id, "the caret goes to it")
    check(home.rows.count > before && home.rows.contains { $0.text == "Journal" }, "Home shows the journal")
    day.setText(day.rows[0], "wrote")
    // Opening the same day again (another tab, or the next launch) finds the same page.
    let again = docs.page(.day("2026-11-27"))
    check(again.root == day.root && again.rows.map(\.text) == ["wrote"], "one page per day")
    again.add("more")
    check(again.rows.map(\.text) == ["wrote", "more"], "added to the day")
    check(day.rows.map(\.text) == ["wrote", "more"], "the other view of the day follows")

    // Folding.
    let journalRow = home.rows.first { $0.text == "Journal" }!
    let shown = home.rows.count
    home.toggleFold(journalRow)
    check(home.rows.count < shown && home.rows.first { $0.text == "Journal" }!.collapsed, "folded: \(home.rows.map(\.text))")
    home.toggleFold(home.rows.first { $0.text == "Journal" }!)
    check(home.rows.count == shown, "unfolded")

    // Stars.
    let groceries = home.rows[0]
    docs.setStarred(groceries.id, true)
    check(docs.starred().map(\.id) == [groceries.id], "starred")
    let page = docs.page(.bullet(groceries.id))
    check(page.info?.starred == true, "the page shows it starred")
    docs.setStarred(groceries.id, false)
    check(page.info?.starred == false && docs.starred().isEmpty, "unstarred")

    // Search with filters; the path names where a hit is.
    let hits = docs.search("milk is:done")
    check(hits.map(\.title) == ["日😀milk"] && hits[0].pathLine() == "groceries", "search: \(hits.map(\.title))")
    check(BulletView.line(["a very long first bullet", "b", "c", "the last one"], limit: 30) == "a very long first bullet › … › the last one", "paths cut in the middle")

    // Move To: the picker leaves out the bullet itself; it moves as the last child.
    let rye = home.rows.first { $0.text == "rye bread" }!
    check(!docs.quickFind("", excluding: [groceries.id]).contains { $0.id == rye.id }, "not under itself")
    home.move(rye.id, to: day.root)
    check(day.rows.last?.text == "rye bread", "moved to the end of the day: \(day.rows.map(\.text))")
    check(!home.rows.contains { $0.id == rye.id && $0.depth == 1 }, "gone from under groceries")
    home.undo()
    check(home.rows.contains { $0.id == rye.id }, "undo moves it back")

    // Hide Completed: every page reads again.
    let milkRow = page.rows.first { $0.text == "日😀milk" }!
    check(milkRow.completed && docs.search("milk is:open").isEmpty, "milk was ticked off")
    docs.setHideCompleted(true)
    check(!page.rows.contains { $0.id == milkRow.id } && !home.rows.contains { $0.id == milkRow.id }, "completed rows hidden")
    docs.setHideCompleted(false)
    check(page.rows.contains { $0.id == milkRow.id }, "shown again")

    // Recently Deleted: restore only.
    page.delete(page.rows.first { $0.id == milkRow.id }!)
    let deleted = docs.trashed()
    check(deleted.first?.id == milkRow.id && deleted.first?.count == 1, "in Recently Deleted, newest first: \(deleted.map(\.text))")
    let restored = docs.restore(milkRow.id)
    check(restored?.path.first?.id == groceries.id && page.rows.contains { $0.id == milkRow.id }, "restored where it was")
    check(!docs.trashed().contains { $0.id == milkRow.id }, "gone from Recently Deleted")

    // A page off screen reads what changed when it shows again.
    page.isVisible = false
    home.add("while away")
    page.add("on the page")
    home.isVisible = false
    page.add("unseen by Home")
    check(!home.rows.contains { $0.text == "unseen by Home" }, "Home off screen waits")
    home.isVisible = true
    check(home.rows.contains { $0.text == "unseen by Home" }, "and catches up when shown")
    page.isVisible = true
    check(docs.link(groceries.id) == "[groceries](flowie:\(groceries.id))", "copy link")
    print("Shell check: ok")
}

/// The launch rule (wiki: iOS-Cross-Platform §4.3).
func checkLaunchRule() {
    let defaults = UserDefaults(suiteName: "flowie-launch-check-\(UUID().uuidString)")!
    LaunchMemory.defaults = defaults
    let noon = ISO8601DateFormatter().date(from: "2026-11-27T12:00:00Z")!
    let day = LaunchMemory.day(of: noon)
    check(LaunchMemory.start(at: noon) == .new, "a new user starts fresh")
    check(!LaunchMemory.zoomedOnce, "and sees the hint")
    let place = LaunchMemory.Place(tab: "home", path: ["abc", "def"])
    LaunchMemory.leave(place, at: noon)
    check(LaunchMemory.start(at: noon.addingTimeInterval(1800)) == .place(place), "back within the hour: where it was")
    check(LaunchMemory.start(at: noon.addingTimeInterval(3700)) == .today, "away over an hour: today")
    let nextDay = LaunchMemory.date(of: LaunchMemory.day(day, offset: 1))!.addingTimeInterval(60)
    LaunchMemory.leave(place, at: nextDay.addingTimeInterval(-120))
    check(LaunchMemory.start(at: nextDay) == .today, "the date changed: today")
    check(LaunchMemory.day("2026-12-31", offset: 1) == "2027-01-01" && LaunchMemory.day("2026-03-01", offset: -1) == "2026-02-28", "days around")
    LaunchMemory.rememberScroll("b:abc", row: "r1")
    LaunchMemory.armScroll()
    check(LaunchMemory.takeScroll(for: "home") == nil && LaunchMemory.takeScroll(for: "b:abc") == "r1", "scroll position for its page")
    check(LaunchMemory.takeScroll(for: "b:abc") == nil, "once")
    LaunchMemory.defaults = .standard
    print("Launch rule check: ok")
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
        store.page(.home).add("kept across the move")
        // Not flushed: what is queued is saved when the outline closes.
    }
    try fm.createDirectory(at: old.appendingPathComponent("flowie.sqlite.images/ab"), withIntermediateDirectories: true)
    try Data("img".utf8).write(to: old.appendingPathComponent("flowie.sqlite.images/ab/abcd"))
    let moved = OutlineLocation.adopt(from: old, to: new)
    check(moved == new.appendingPathComponent(OutlineLocation.fileName), "the database is in the container")
    check(!fm.fileExists(atPath: old.path), "the old folder moved")
    check(fm.fileExists(atPath: new.appendingPathComponent("flowie.sqlite.images/ab/abcd").path), "images moved with it")
    let store = try OutlineStore.open(databaseAt: moved).page(.home)
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
    checkLaunchRule()
}
