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
    check(home.childCount(home.rows[0]) == 2 && home.childCount(home.rows[1]) == 0, "children counted")
    home.toggleFold(home.rows[0])
    check(home.childCount(home.rows[0]) == 2, "a collapsed row's children counted")
    home.toggleFold(home.rows[0])

    // VoiceOver's rotors: siblings under the same parent, and the parent.
    let depths = [0, 1, 2, 1, 0, 1]
    check(RowNavigation.sibling(of: 1, in: depths, forward: true) == 3, "next sibling skips deeper rows")
    check(RowNavigation.sibling(of: 3, in: depths, forward: true) == nil, "no sibling past the parent's children")
    check(RowNavigation.sibling(of: 3, in: depths, forward: false) == 1, "previous sibling")
    check(RowNavigation.sibling(of: 0, in: depths, forward: true) == 4, "top-level sibling")
    check(RowNavigation.parent(of: 2, in: depths) == 1 && RowNavigation.parent(of: 5, in: depths) == 4, "parent")
    check(RowNavigation.parent(of: 4, in: depths) == nil, "no parent at the top")
    check(RowNavigation.childCount(of: 0, in: depths) == 2 && RowNavigation.childCount(of: 2, in: depths) == 0, "child count")

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
    try checkCapture(docs)

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

/// Quick capture (step 6): the capture entries put the caret on an empty last row of today's
/// journal, made once; "Add to Today's Journal" appends text, which pages showing the day read.
@MainActor
func checkCapture(_ docs: OutlineStore) throws {
    let today = LaunchMemory.today
    let day = docs.page(.day(today))
    let before = day.rows.map(\.text)
    try docs.captureToday()
    guard let capture = docs.capture else { check(false, "a capture is asked for"); return }
    check(capture.day == today && day.rows.last?.id == capture.row && day.rows.last?.text == "", "capture goes to an empty last row: \(day.rows.map(\.text))")
    check(day.rows.count == before.count + (before.last == "" ? 0 : 1), "at most one row is made")
    try docs.captureToday()
    check(docs.capture?.row == capture.row && docs.capture != capture, "capturing again reuses the row, as a new capture")
    check(day.rows.count == before.count + (before.last == "" ? 0 : 1), "no second empty row")
    // Visiting the day neither adds nor focuses anything.
    let visit = docs.page(.day(today))
    check(visit.rows == day.rows && visit.focus == nil, "visiting Today changes nothing")

    // Typing in the capture row, then Add to Today's Journal: the row being typed in stays.
    day.editing(day.rows.last)
    let typed = day.rows.map(\.text)
    check(try docs.appendToToday("  from **Siri** 日😀  \n\nsecond line"), "text added")
    check(Array(day.rows.suffix(3).map(\.text)) == ["", "from **Siri** 日😀", "second line"], "one bullet a line, after the row being typed in: \(day.rows.map(\.text))")
    check(visit.rows.map(\.text) == day.rows.map(\.text), "other views of the day read it")
    check(try !docs.appendToToday(" \n "), "blank text adds nothing")
    docs.finishEditing()
    day.undo()
    check(day.rows.map(\.text) == typed, "Add to Today's Journal is one undo step: \(day.rows.map(\.text))")
    day.redo()
    try docs.flush()
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

/// Rows not being edited show their markup rendered (§2.3): syntax hidden, long addresses cut
/// in the middle, and a tap's place mapped back to the stored text; notes save as one step.
@MainActor
func checkRendering() throws {
    // 日 is one UTF-16 unit, 😀 two.
    let id = String(repeating: "ab", count: 16)
    let stored = "日😀 **粗体** [链接](flowie:\(id)) #标签 $x^2$"
    let shown = RenderedText(stored)
    check(shown.string == "日😀 粗体 链接 #标签 $x^2$", "syntax hidden, math kept as source: \(shown.string)")
    let bold = shown.pieces.first { $0.run.bold }
    check(bold?.text == "粗体" && bold?.shown == 4..<6 && bold?.stored == 6..<8, "bold maps back past the emoji and **")
    check(shown.storedOffset(0) == 0 && shown.storedOffset(3) == 3, "before the bold, as stored")
    check(shown.storedOffset(5) == 7, "inside the bold")
    check(shown.storedOffset(6) == 8, "at its end: before the closing **")
    let link = shown.pieces.first { if case .link(_, let bullet) = $0.run.kind { return bullet == id }; return false }
    check(link?.text == "链接", "a flowie: link shows its label")
    check(shown.storedOffset(shown.string.utf16.count) == stored.utf16.count, "the end is the end")
    check(shown.piece(at: 4)?.text == "粗体" && shown.piece(at: 99) == nil, "the piece under a place")

    // Long web addresses: about 28 characters, cut in the middle; a caret in the "…" goes
    // past the middle.
    let url = "https://example.com/a/very/long/path/to/some/page.html"
    let cut = RenderedText("see \(url) ok")
    let address = cut.pieces.filter { if case .link = $0.run.kind { return true }; return false }.map(\.text).joined()
    check(address.count == RenderedText.linkLimit && address.hasPrefix("https://example.co") && address.hasSuffix("page.html"), "cut: \(address)")
    check(address.contains("…"), "with an ellipsis")
    let ellipsis = cut.pieces.first { $0.text == "…" }!
    check(cut.storedOffset(ellipsis.shown.upperBound) == ellipsis.stored.upperBound, "after the ellipsis: past the middle")
    check(cut.storedOffset(cut.string.utf16.count) == ("see \(url) ok" as String).utf16.count, "the end after a cut address")
    check(RenderedText("https://short.example/x").string == "https://short.example/x", "short addresses stay whole")
    let longLabel = String(repeating: "中😀", count: 30)
    let longLink = RenderedText("[\(longLabel)](flowie:\(id))")
    check(longLink.string.count == 28 && longLink.storedOffset(longLink.string.utf16.count) == 1 + longLabel.utf16.count, "long label maps to its original end")
    check(RenderedText.webURL("https://example.org/页")?.host == "example.org" && RenderedText.webURL("HTTP://example.org") != nil, "web links")
    for unsupported in ["flowie:bad", "other:app", "file:///tmp/note", "//example.org", "https:"] {
        check(RenderedText.webURL(unsupported) == nil, "unsupported link: \(unsupported)")
    }
    let many = RenderedText(String(repeating: "**中😀** ", count: 1024))
    check(many.pieces.last?.shown.upperBound == 4 * 1024 && many.storedOffset(4 * 1024) == 8 * 1024, "many runs retain UTF-16 alignment")
    check(RenderedText("**not** markup", code: true).string == "**not** markup", "code blocks show as stored")
    check(RenderedText("").pieces.isEmpty && RenderedText("").storedOffset(0) == 0, "empty")

    // Thumbnails: at most 64pt high, the pixel size at 2×, and a placeholder of the same size
    // when the size isn't known.
    check(Thumbnail.size(width: 400, height: 100, maxWidth: 300) == CGSize(width: 200, height: 50), "small enough: half its pixels")
    check(Thumbnail.size(width: 1200, height: 900, maxWidth: 300) == CGSize(width: 85, height: 64), "at most 64pt high")
    check(Thumbnail.size(width: 4000, height: 200, maxWidth: 300) == CGSize(width: 300, height: 16), "at most the width, at least 16pt")
    check(Thumbnail.size(width: 0, height: 0, maxWidth: 300) == CGSize(width: 96, height: 64), "unknown: a placeholder")

    // The notes sheet saves through set_note: one undo step, the preview follows.
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("flowie-render-check-\(UUID().uuidString)")
    let docs = try OutlineStore.open(databaseAt: dir.appendingPathComponent("outline.sqlite"))
    let home = docs.page(.home)
    home.add("a bullet")
    home.setText(home.rows[0], "a bullet, typed")
    home.setNote(home.rows[0], "first line\nsecond \"line\"")
    check(home.rows[0].note == "first line\nsecond \"line\"", "the note is saved")
    home.setNote(home.rows[0], "又一条 -- \"备注\"")
    home.undo()
    check(home.rows[0].note == "first line\nsecond \"line\"", "a second sheet save is independent")
    home.undo()
    check(home.rows[0].note.isEmpty && home.rows[0].text == "a bullet, typed", "one undo step takes the note away only")
    home.undo()
    check(home.rows[0].text == "a bullet", "the previous undo step is the body edit")
    home.setText(home.rows[0], "**é**")
    home.setText(home.rows[0], "**e\u{301}**")
    check(home.rows[0].text.utf16.count == 6, "keep the stored spelling even for canonically equivalent input")
    try docs.outline.flush()
    try? FileManager.default.removeItem(at: dir)
    print("Rendering check: ok")
}

try MainActor.assumeIsolated {
    try run()
    try checkLocation()
    checkLaunchRule()
    try checkRendering()
}
