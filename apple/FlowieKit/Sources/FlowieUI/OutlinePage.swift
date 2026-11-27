#if canImport(UIKit)
import Combine
import SwiftUI
import UIKit
import FlowieFFI

/// The outline page on iOS (wiki: iOS-Cross-Platform §4.1, §4.2): a plain UICollectionView
/// list with a diffable data source, one UITextView per row. Dense: no cards or separators,
/// 28pt a line at the default text size. Typing reconfigures only its row; rows coming, going
/// or moving apply a new snapshot of ids; the row being edited is never reloaded, and the caret
/// moves before the edit returns, so the next key lands where the edit put it. Tapping a bullet
/// pushes its page (`open`). Rows not being edited show their markup rendered (§2.3): tapping a
/// `flowie:` link opens its bullet, a web link the browser, a tag or a date a search (`search`).
public struct OutlinePage: UIViewControllerRepresentable {
    let page: PageModel
    /// Open a bullet as a page (push it).
    let open: (String) -> Void
    /// Search for this (a tapped tag or date), as the desktop does.
    let search: (String) -> Void

    public init(page: PageModel, open: @escaping (String) -> Void, search: @escaping (String) -> Void = { _ in }) {
        self.page = page
        self.open = open
        self.search = search
    }

    public func makeUIViewController(context: Context) -> OutlinePageController {
        OutlinePageController(page: page, open: open, search: search)
    }

    public func updateUIViewController(_ controller: OutlinePageController, context: Context) {
        controller.open = open
        controller.search = search
    }
}

/// Sizes of a row at the current text size and width (§4.1). At the default (Large) size:
/// 17pt text on 22pt lines, 3pt above and below, a 20pt bullet slot, 16pt a level.
/// How far the page's "…" menu and ⌃-number keys fold the page: the desktop's Show 1 Level
/// (here Collapse All), Show 2 and 3 Levels, and Expand All, with its keys.
enum FoldLevel: CaseIterable {
    case collapseAll, two, three, expandAll

    var levels: UInt32 {
        switch self {
        case .collapseAll: 1
        case .two: 2
        case .three: 3
        case .expandAll: .max
        }
    }

    var title: String {
        switch self {
        case .collapseAll: NSLocalizedString("Collapse All", comment: "Page menu item: fold every top-level bullet")
        case .two: NSLocalizedString("Show 2 Levels", comment: "Page menu item: show top-level bullets and their children")
        case .three: NSLocalizedString("Show 3 Levels", comment: "Page menu item: show three levels of bullets")
        case .expandAll: NSLocalizedString("Expand All", comment: "Page menu item: unfold every bullet on the page")
        }
    }

    var symbol: String {
        switch self {
        case .collapseAll: "rectangle.compress.vertical"
        case .two: "2.circle"
        case .three: "3.circle"
        case .expandAll: "rectangle.expand.vertical"
        }
    }

    /// The desktop's key, with ⌃.
    var key: String {
        switch self {
        case .collapseAll: "1"
        case .two: "2"
        case .three: "3"
        case .expandAll: "0"
        }
    }
}

struct RowMetrics: Equatable {
    var font: UIFont
    var lineHeight: CGFloat
    var noteFont: UIFont
    var noteLineHeight: CGFloat
    var pad: CGFloat = 3
    var leading: CGFloat = 12
    var trailing: CGFloat = 16
    var slot: CGFloat
    var bullet: CGFloat
    var ring: CGFloat
    var indent: CGFloat
    /// Rows deeper than this stop moving right, so the text keeps at least 55% of the width.
    var maxIndent: Int

    init(width: CGFloat, traits: UITraitCollection) {
        let body = UIFontMetrics(forTextStyle: .body)
        let footnote = UIFontMetrics(forTextStyle: .footnote)
        font = body.scaledFont(for: .systemFont(ofSize: 17), compatibleWith: traits)
        lineHeight = ceil(body.scaledValue(for: 22, compatibleWith: traits))
        noteFont = footnote.scaledFont(for: .systemFont(ofSize: 13), compatibleWith: traits)
        noteLineHeight = ceil(footnote.scaledValue(for: 18, compatibleWith: traits))
        slot = body.scaledValue(for: 20, compatibleWith: traits)
        bullet = body.scaledValue(for: 6, compatibleWith: traits)
        ring = body.scaledValue(for: 16, compatibleWith: traits)
        indent = min(body.scaledValue(for: 16, compatibleWith: traits), 22)
        let room = width * 0.45 - leading - trailing - slot
        maxIndent = max(0, Int(floor(room / indent)))
    }

    /// How far right a row at `depth` sits, in levels.
    func shown(_ depth: Int) -> Int {
        min(depth, maxIndent)
    }

    /// The x of the bullet of a row at `depth`.
    func bulletX(_ depth: Int) -> CGFloat {
        leading + CGFloat(shown(depth)) * indent + slot / 2
    }

    func textX(_ depth: Int) -> CGFloat {
        leading + CGFloat(shown(depth)) * indent + slot
    }

    func textAttributes(completed: Bool) -> [NSAttributedString.Key: Any] {
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = lineHeight
        paragraph.maximumLineHeight = lineHeight
        var attributes: [NSAttributedString.Key: Any] = [
            .font: font,
            .paragraphStyle: paragraph,
            .foregroundColor: UIColor.label,
            .baselineOffset: (lineHeight - font.lineHeight) / 4,
        ]
        if completed {
            attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
        }
        return attributes
    }
}

public final class OutlinePageController: UIViewController, UICollectionViewDelegate, UITextViewDelegate, UIGestureRecognizerDelegate, UIContextMenuInteractionDelegate {
    let page: PageModel
    var open: (String) -> Void
    var search: (String) -> Void
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>!
    private var rowsByID: [String: RowView] = [:]
    private var order: [String] = []
    private var indexByID: [String: Int] = [:]
    private var metrics = RowMetrics(width: 390, traits: .current)
    /// The row being typed in.
    private var editingID: String?
    private weak var noteSheet: NoteSheet?
    /// For each row, how many of its guide lines (from the left) belong to the edited row's
    /// ancestors, drawn stronger.
    private var strong: [String: Int] = [:]
    /// While a snapshot is applied or the caret placed, rows leaving the screen keep the caret.
    private var placing = false
    private lazy var bar = KeyboardBar(controller: self)
    /// The undo manager the rows' text views and the system (⌘Z, shake, three-finger swipe,
    /// the shortcuts bar) use: it undoes the outline's steps, not the text view's own.
    private lazy var pageUndo = PageUndoManager(page: page)
    /// A grey line below the rows: how to open a bullet (new users, on Home), or how to start a
    /// day of the journal.
    private let hint = UILabel()
    /// Whether the rows have been shown once (the remembered scroll position applies then).
    private var shownOnce = false
    /// Whether the outline syncs through iCloud, for image rows whose file is missing.
    private var iCloud = false
    private var iCloudWatch: AnyCancellable?
    /// "Deleted · Undo", after a bullet is deleted.
    private let undoToast = UndoToast()
    /// Keeps the toast above the docked keyboard (and its bar).
    private var toastBottom: NSLayoutConstraint!

    init(page: PageModel, open: @escaping (String) -> Void, search: @escaping (String) -> Void) {
        self.page = page
        self.open = open
        self.search = search
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        var config = UICollectionLayoutListConfiguration(appearance: .plain)
        config.showsSeparators = false
        config.backgroundColor = .systemBackground
        // Room below the last row to tap for a new one: part of the content, not an inset, so
        // UIKit's own scrolling to the caret doesn't count it as covered.
        let layout = UICollectionViewCompositionalLayout { _, environment in
            let section = NSCollectionLayoutSection.list(using: config, layoutEnvironment: environment)
            section.contentInsets.bottom = Self.roomBelow
            return section
        }
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: layout)
        collectionView.backgroundColor = .systemBackground
        collectionView.keyboardDismissMode = .interactive
        // A row that gains or loses a line while typed in asks to be measured again.
        collectionView.selfSizingInvalidation = .enabled
        collectionView.alwaysBounceVertical = true
        collectionView.delegate = self
        collectionView.translatesAutoresizingMaskIntoConstraints = false
        view.backgroundColor = .systemBackground
        view.addSubview(collectionView)
        NSLayoutConstraint.activate([
            collectionView.topAnchor.constraint(equalTo: view.topAnchor),
            collectionView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            collectionView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            collectionView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        let registration = UICollectionView.CellRegistration<RowCell, String> { [unowned self] cell, _, id in
            guard let row = rowsByID[id] else { return }
            configure(cell, row)
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { view, path, id in
            view.dequeueConfiguredReusableCell(using: registration, for: path, item: id)
        }

        let tap = UITapGestureRecognizer(target: self, action: #selector(tappedBelowRows))
        tap.delegate = self
        tap.cancelsTouchesInView = false
        collectionView.addGestureRecognizer(tap)

        // The rows stay above the keyboard by an inset as tall as the part of the page it covers.
        // (The keyboard layout guide sat at the top of the page inside the tab view.)
        // (Without an animation, as when the app starts with the caret in a row, only the
        // "did" notifications come.)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardMoved), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardMoved), name: UIResponder.keyboardDidChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardMoved), name: UIResponder.keyboardWillHideNotification, object: nil)
        // A capture asked for while the app was away takes the caret once the app is back.
        NotificationCenter.default.addObserver(self, selector: #selector(takeCapture), name: UIApplication.didBecomeActiveNotification, object: nil)

        hint.font = .preferredFont(forTextStyle: .subheadline)
        hint.adjustsFontForContentSizeCategory = true
        hint.textColor = .tertiaryLabel
        hint.numberOfLines = 0
        collectionView.addSubview(hint)

        undoToast.translatesAutoresizingMaskIntoConstraints = false
        undoToast.onUndo = { [weak self] in self?.undo() }
        view.addSubview(undoToast)
        toastBottom = undoToast.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12)
        NSLayoutConstraint.activate([
            toastBottom,
            undoToast.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            undoToast.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -32),
        ])

        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (self: Self, _) in
            self.remeasure()
        }
        iCloud = page.store.iCloud.isOn
        iCloudWatch = page.store.$iCloud.map(\.isOn).removeDuplicates().dropFirst().sink { [weak self] on in
            self?.iCloudChanged(on)
        }
        page.onChange = { [weak self] change in self?.pageChanged(change) }
        page.finishEditor = { [weak self] in self?.finishEditor() }
        page.onCapture = { [weak self] in self?.takeCapture() }
        read()
        applySnapshot(reconfigure: [], animated: false)
    }

    public override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        page.isVisible = true
    }

    public override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // The list is what the navigation bar's large title and the tab bar follow.
        var ancestor = parent
        while let vc = ancestor, !(vc is UINavigationController), !(vc is UITabBarController) {
            vc.setContentScrollView(collectionView, for: .top)
            vc.setContentScrollView(collectionView, for: .bottom)
            ancestor = vc.parent
        }
        if !shownOnce {
            shownOnce = true
            if let row = LaunchMemory.takeScroll(for: page.kind.key), let index = indexByID[row] {
                collectionView.layoutIfNeeded()
                collectionView.scrollToItem(at: IndexPath(item: index, section: 0), at: .top, animated: false)
            }
        }
        revealIfAsked()
        takeCapture()
    }

    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        page.isVisible = false
    }

    public override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        layoutHint()
    }

    /// Room below the last row, to tap for a new one.
    private static let roomBelow: CGFloat = 120

    /// Where the rows end, in the content (the room below them follows).
    private var rowsBottom: CGFloat {
        order.isEmpty ? 0 : max(0, collectionView.collectionViewLayout.collectionViewContentSize.height - Self.roomBelow)
    }

    /// The keyboard covers the page only when it is docked: on the window's bottom edge and as
    /// wide as the window. A floating, split or undocked keyboard covers nothing (§4.4); the
    /// page scrolls under it. The frame comes in the screen's coordinates, which aren't the
    /// window's under Stage Manager or Split View.
    @objc private func keyboardMoved(_ note: Notification) {
        guard let window = view.window else { return }
        var covered: CGFloat = 0
        if note.name != UIResponder.keyboardWillHideNotification,
           let end = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue,
           !end.isEmpty {
            let space = window.windowScene?.screen.coordinateSpace ?? window.coordinateSpace
            let keyboard = window.convert(end, from: space)
            let bounds = window.bounds
            let docked = keyboard.maxY >= bounds.maxY - 1 && keyboard.minX <= bounds.minX + 1 && keyboard.maxX >= bounds.maxX - 1
            if docked {
                let inView = view.convert(keyboard, from: window)
                covered = max(0, view.bounds.maxY - inView.minY)
                // The safe area at the bottom (tab bar, home indicator) is under the keyboard too.
                covered = max(0, covered - view.safeAreaInsets.bottom)
            }
        }
        collectionView.contentInset.bottom = covered
        collectionView.verticalScrollIndicatorInsets.bottom = covered
        toastBottom.constant = -12 - covered
    }

    public override func viewWillLayoutSubviews() {
        super.viewWillLayoutSubviews()
        if metrics != RowMetrics(width: view.bounds.width, traits: traitCollection) {
            remeasure()
        }
    }

    /// The text size or the width changed: every row is measured again.
    private func remeasure() {
        guard isViewLoaded, dataSource != nil else { return }
        metrics = RowMetrics(width: view.bounds.width, traits: traitCollection)
        var snapshot = dataSource.snapshot()
        snapshot.reconfigureItems(snapshot.itemIdentifiers)
        placing = true
        dataSource.apply(snapshot, animatingDifferences: false)
        placing = false
    }

    // MARK: Rows

    /// Take the store's rows.
    private func read() {
        rowsByID = [:]
        order = []
        indexByID = [:]
        order.reserveCapacity(page.rows.count)
        for row in page.rows where rowsByID[row.id] == nil {
            indexByID[row.id] = order.count
            order.append(row.id)
            rowsByID[row.id] = row
        }
        computeStrongGuides()
        updateHint()
    }

    private func applySnapshot(reconfigure: [String], animated: Bool) {
        var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
        snapshot.appendSections([0])
        snapshot.appendItems(order)
        if !reconfigure.isEmpty {
            snapshot.reconfigureItems(reconfigure)
        }
        placing = true
        dataSource.apply(snapshot, animatingDifferences: animated)
        placing = false
    }

    /// iCloud sync started or stopped: image rows say again whether their file is coming.
    private func iCloudChanged(_ on: Bool) {
        iCloud = on
        let images = order.filter { rowsByID[$0]?.image != nil }
        guard isViewLoaded, !images.isEmpty else { return }
        var snapshot = dataSource.snapshot()
        snapshot.reconfigureItems(images)
        placing = true
        dataSource.apply(snapshot, animatingDifferences: false)
        placing = false
    }

    /// The page changed its rows, inside the edit that changed them.
    private func pageChanged(_ change: PageChange) {
        // Undo would now undo something else than the delete.
        undoToast.hide()
        let focus = page.focus
        page.focus = nil
        // The row taking the caret takes it first, while the one losing it is still there:
        // the keyboard stays up when the edited row goes (Backspace joining rows).
        if let focus, let cell = cell(for: focus.id), !cell.textView.isFirstResponder {
            placing = true
            cell.textView.becomeFirstResponder()
            placing = false
        }
        switch change {
        case .rows:
            let before = rowsByID
            let shown = order
            read()
            let changed = order.filter { id in before[id].map { $0 != rowsByID[id] } ?? false }
            if focus == nil, order == shown, !changed.isEmpty, changed.allSatisfy({ before[$0]?.image != rowsByID[$0]?.image }) {
                // Only pictures changed (a file came through sync, with its size): the rows on
                // screen stay where they are.
                keepingTopRow { applySnapshot(reconfigure: changed, animated: false) }
            } else {
                applySnapshot(reconfigure: changed, animated: true)
            }
            refreshGuides()
        case .content(let ids):
            var shown: [String] = []
            for id in ids {
                if let row = page.rows.first(where: { $0.id == id }), rowsByID[id] != nil {
                    rowsByID[id] = row
                    shown.append(id)
                }
            }
            if !shown.isEmpty {
                var snapshot = dataSource.snapshot()
                snapshot.reconfigureItems(shown)
                placing = true
                dataSource.apply(snapshot, animatingDifferences: false)
                placing = false
            }
        }
        if let focus {
            place(focus)
        }
        // The shortcuts bar's Undo and Redo show whether there is something to undo.
        NotificationCenter.default.post(name: .NSUndoManagerCheckpoint, object: pageUndo)
        revealIfAsked()
        layoutHint()
    }

    /// Do `change` with the top row on screen kept at its place, whatever happened to the
    /// heights of the rows above it (a picture that came in at another size than its
    /// placeholder). Not while the list is being dragged, which moves it anyway.
    private func keepingTopRow(_ change: () -> Void) {
        collectionView.layoutIfNeeded()
        let top = visibleRect.minY
        let anchor = collectionView.indexPathsForVisibleItems
            .compactMap { path in collectionView.layoutAttributesForItem(at: path).map { (path: path, frame: $0.frame) } }
            .filter { $0.frame.maxY > top + 1 }
            .min { $0.path.item < $1.path.item }
        change()
        guard let anchor, !collectionView.isTracking else { return }
        collectionView.layoutIfNeeded()
        guard let now = collectionView.layoutAttributesForItem(at: anchor.path)?.frame.minY, abs(now - anchor.frame.minY) > 0.5 else { return }
        let lowest = -collectionView.adjustedContentInset.top
        let y = max(lowest, collectionView.contentOffset.y + now - anchor.frame.minY)
        placing = true
        collectionView.setContentOffset(CGPoint(x: collectionView.contentOffset.x, y: y), animated: false)
        placing = false
    }

    private func cell(for id: String) -> RowCell? {
        guard let index = indexByID[id] else { return nil }
        return collectionView.cellForItem(at: IndexPath(item: index, section: 0)) as? RowCell
    }

    /// Put the caret where an edit said, scrolling its row into view: not at all when it is in
    /// view already, as little as needed when it is near (a new row under the keyboard),
    /// centered when it was far off screen.
    private func place(_ focus: Focus) {
        guard let index = indexByID[focus.id] else { return }
        let path = IndexPath(item: index, section: 0)
        placing = true
        defer { placing = false }
        collectionView.layoutIfNeeded()
        // A row near the screen (one Return made under the keyboard) comes up just enough; a
        // text view off screen wouldn't take the caret.
        if let frame = collectionView.layoutAttributesForItem(at: path)?.frame {
            let visible = visibleRect
            if frame.intersects(visible.insetBy(dx: 0, dy: -visible.height)) {
                scrollToShow(frame)
                collectionView.layoutIfNeeded()
            }
        }
        var target = collectionView.cellForItem(at: path) as? RowCell
        if target == nil || !(target!.frame.intersects(visibleRect)) {
            collectionView.scrollToItem(at: path, at: .centeredVertically, animated: false)
            collectionView.layoutIfNeeded()
            target = collectionView.cellForItem(at: path) as? RowCell
        }
        guard let target else { return }
        let textView = target.textView
        if !textView.isFirstResponder {
            textView.becomeFirstResponder()
        }
        let length = (textView.text as NSString).length
        let start = min(focus.start, length)
        textView.selectedRange = NSRange(location: start, length: min(max(0, focus.end - focus.start), length - start))
        scrollToShow(collectionView.convert(target.bounds, from: target))
    }

    /// The part of the content not under the bars or the keyboard.
    private var visibleRect: CGRect {
        collectionView.bounds.inset(by: collectionView.adjustedContentInset)
    }

    /// Scroll as little as needed to show `frame` (with a little air), not at all if it shows.
    private func scrollToShow(_ frame: CGRect) {
        let visible = visibleRect
        let wanted = frame.insetBy(dx: 0, dy: -8)
        var dy: CGFloat = 0
        if wanted.minY < visible.minY {
            dy = wanted.minY - visible.minY
        } else if wanted.maxY > visible.maxY {
            // Never so far that the row's top goes under the top bar.
            dy = min(wanted.maxY - visible.maxY, max(0, wanted.minY - visible.minY))
        }
        guard abs(dy) > 0.5 else { return }
        let inset = collectionView.adjustedContentInset
        let lowest = -inset.top
        let highest = max(lowest, collectionView.contentSize.height + inset.bottom - collectionView.bounds.height)
        let y = min(max(collectionView.contentOffset.y + dy, lowest), highest)
        collectionView.setContentOffset(CGPoint(x: collectionView.contentOffset.x, y: y), animated: false)
    }

    private func configure(_ cell: RowCell, _ row: RowView) {
        let textView = cell.textView
        if textView.delegate == nil {
            textView.delegate = self
            textView.pageUndo = pageUndo
            if traitCollection.userInterfaceIdiom == .pad {
                // iPad: the commands sit in the shortcuts bar, above the docked keyboard and at
                // the bottom with a hardware keyboard. An accessory view would be pinned to the
                // screen's bottom under the floating keyboard; there the menus and key commands
                // have them.
                addShortcuts(to: textView.inputAssistantItem)
            } else {
                textView.inputAccessoryView = bar
            }
            textView.onBackspaceAtStart = { [weak self, weak textView] in
                guard let self, let id = textView?.rowID, let row = rowsByID[id] else { return }
                page.backspaceAtStart(row)
            }
            cell.slot.addInteraction(UIContextMenuInteraction(delegate: self))
            cell.accessibilityCustomRotors = rotors
            textView.accessibilityCustomRotors = rotors
            cell.accessibilityStatus = { [weak self, weak cell] in
                guard let self, let id = cell?.textView.rowID else { return nil }
                return accessibilityStatus(of: id)
            }
            cell.accessibilityActions = { [weak self, weak cell] in
                guard let self, let id = cell?.textView.rowID else { return [] }
                return accessibilityActions(for: id)
            }
        }
        cell.slot.onTap = { [weak self, weak cell] in
            guard let self, let id = cell?.textView.rowID, let row = rowsByID[id] else { return }
            zoom(into: row)
        }
        cell.slot.onDoubleTap = { [weak self, weak cell] in
            guard let self, let id = cell?.textView.rowID, let row = rowsByID[id] else { return }
            page.toggleFold(row)
        }
        cell.onTextTap = { [weak self, weak cell] tap in
            guard let self, let id = cell?.textView.rowID else { return }
            tapped(tap, in: id)
        }
        cell.onNoteTap = { [weak self, weak cell] in
            guard let self, let id = cell?.textView.rowID, let row = rowsByID[id] else { return }
            presentNote(row)
        }
        cell.onImageTap = { [weak self] path in
            self?.presentImage(path)
        }
        cell.configure(row: row, metrics: metrics, strong: strong[row.id] ?? 0, iCloud: iCloud)
    }

    // MARK: Guide lines

    /// Which guide lines are drawn stronger: those of the edited row's ancestors, through each
    /// ancestor's rows below it.
    private func computeStrongGuides() {
        strong = [:]
        guard let id = editingID, let index = indexByID[id], let row = rowsByID[id], row.depth > 0 else { return }
        var level = Int(row.depth) - 1
        var i = index - 1
        while i >= 0 && level >= 0 {
            let depth = Int(rowsByID[order[i]]!.depth)
            if depth == level {
                // order[i] is the ancestor at `level`: its rows below show `level + 1` strong lines.
                var j = i + 1
                while j < order.count, Int(rowsByID[order[j]]!.depth) > level {
                    if strong[order[j]] == nil {
                        strong[order[j]] = level + 1
                    }
                    j += 1
                }
                level -= 1
            }
            i -= 1
        }
    }

    private func refreshGuides() {
        computeStrongGuides()
        for case let cell as RowCell in collectionView.visibleCells {
            if let id = cell.textView.rowID {
                cell.slot.setStrong(strong[id] ?? 0)
            }
        }
    }

    // MARK: Actions

    /// Open a bullet as a page (tapping its bullet or depth number): pushed on the navigation
    /// stack.
    func zoom(into row: RowView) {
        view.endEditing(true)
        LaunchMemory.zoomedOnce = true
        updateHint()
        open(row.id)
    }

    /// A tap on a row's rendered text: on a link, a tag or a date it does what the desktop does;
    /// elsewhere the row is edited, the caret where the finger was.
    private func tapped(_ tap: RowCell.TextTap, in id: String) {
        switch tap {
        case .caret(let offset):
            place(Focus(id: id, start: offset, end: offset))
        case .bullet(let target):
            view.endEditing(true)
            if page.store.bullet(target) != nil {
                open(target)
            } else {
                page.store.problem = NSLocalizedString("That bullet was deleted.", comment: "Shown when a tapped link points to a deleted bullet")
            }
        case .url(let url):
            UIApplication.shared.open(url)
        case .search(let query):
            view.endEditing(true)
            search(query)
        }
    }

    // MARK: Note and picture

    /// Put the caret on the row a capture asked for (the quick action, Control Center, the
    /// Action Button: §5), when this page is on screen and shows it.
    @objc private func takeCapture() {
        guard view.window != nil, UIApplication.shared.applicationState != .background,
              let focus = page.store.takeCapture(on: page) else { return }
        place(focus)
    }

    /// End typing (the app is leaving, or a capture moves the caret): text still being
    /// composed is kept, and so is an open note; the keyboard goes.
    private func finishEditor() {
        noteSheet?.saveDraft()
        guard let editingID else { return }
        if let editor = cell(for: editingID)?.textView, editor.isFirstResponder, let row = row(of: editor) {
            editor.unmarkText()
            page.setText(row, (editor.text ?? "").replacingOccurrences(of: "\n", with: " "))
            editor.resignFirstResponder()
        }
        self.editingID = nil
        page.editing(nil)
        refreshGuides()
    }

    /// A bullet's note in a sheet, to read and edit (§2.1); saved as one undo step.
    func presentNote(_ row: RowView) {
        view.endEditing(true)
        let page = self.page
        let title = page.store.bullet(row.id)?.title ?? ""
        let sheet = NoteSheet(note: row.note, title: title) { note in
            // The row as it is now (the sheet may have been open a while).
            let current = page.rows.first { $0.id == row.id } ?? row
            page.setNote(current, note)
        }
        noteSheet = sheet
        let controller = UINavigationController(rootViewController: sheet)
        if let presentation = controller.sheetPresentationController {
            presentation.detents = [.medium(), .large()]
            presentation.prefersGrabberVisible = true
        }
        controller.presentationController?.delegate = sheet
        present(controller, animated: true)
    }

    private func presentImage(_ path: String) {
        guard FileManager.default.fileExists(atPath: path) else { return }
        view.endEditing(true)
        present(ImageViewer(path: path), animated: true)
    }

    @objc private func tappedBelowRows() {
        // A new row at the end of the page; an empty last row takes the caret instead.
        if let last = order.last, let row = rowsByID[last], row.depth == 0, row.text.isEmpty {
            place(Focus(id: last, start: 0, end: 0))
        } else {
            page.add("", focus: true)
        }
    }

    // MARK: Hint and reveal

    private func updateHint() {
        let text: String?
        if case .home = page.kind, !LaunchMemory.zoomedOnce {
            text = NSLocalizedString("Tap a bullet to open it", comment: "Hint under the rows of Home for new users")
        } else if case .day = page.kind, order.isEmpty {
            text = NSLocalizedString("Tap here to write in this day", comment: "Hint on a day of the journal with nothing in it")
        } else {
            text = nil
        }
        hint.text = text
        hint.isHidden = text == nil
        layoutHint()
    }

    /// The hint sits just below the last row.
    private func layoutHint() {
        guard !hint.isHidden, collectionView != nil else { return }
        let x = metrics.textX(0)
        let width = collectionView.bounds.width - x - metrics.trailing
        let size = hint.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        let y = order.isEmpty ? metrics.pad + 4 : rowsBottom + 8
        hint.frame = CGRect(x: x, y: y, width: width, height: size.height)
    }

    /// Scroll to the row the page was opened for (a search result) and point it out.
    private func revealIfAsked() {
        guard view.window != nil, let id = page.reveal, let index = indexByID[id] else { return }
        page.reveal = nil
        let path = IndexPath(item: index, section: 0)
        collectionView.layoutIfNeeded()
        collectionView.scrollToItem(at: path, at: .centeredVertically, animated: false)
        collectionView.layoutIfNeeded()
        guard let cell = collectionView.cellForItem(at: path) else { return }
        cell.contentView.backgroundColor = UIColor.tintColor.withAlphaComponent(0.18)
        UIView.animate(withDuration: 1.2, delay: 0.6, options: [.allowUserInteraction]) {
            cell.contentView.backgroundColor = .clear
        }
    }

    // MARK: Long-press menu (§4.4)

    public func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard let slot = interaction.view as? SlotView,
              let cell = sequence(first: slot as UIView, next: { $0.superview }).first(where: { $0 is RowCell }) as? RowCell,
              let id = cell.textView.rowID, let row = rowsByID[id]
        else { return nil }
        let width = view.bounds.width
        let page = self.page
        return UIContextMenuConfiguration(identifier: row.id as NSString, previewProvider: {
            RowPreviewController(row: row, page: (try? page.store.outline.page(root: row.id)) ?? [], width: width)
        }) { [weak self] _ in
            self?.menu(for: row)
        }
    }

    /// Tapping the preview opens the bullet, as Zoom In.
    public func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willPerformPreviewActionForMenuWith configuration: UIContextMenuConfiguration, animator: any UIContextMenuInteractionCommitAnimating) {
        guard let id = configuration.identifier as? String else { return }
        animator.preferredCommitStyle = .dismiss
        animator.addCompletion { [weak self] in
            guard let self, let row = rowsByID[id] else { return }
            zoom(into: row)
        }
    }

    /// The whole row lifts with the menu, not just its bullet.
    public func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configuration: UIContextMenuConfiguration, highlightPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
        guard let cell = interaction.view.flatMap({ view in sequence(first: view, next: { $0.superview }).first { $0 is RowCell } }) as? RowCell else { return nil }
        let parameters = UIPreviewParameters()
        parameters.backgroundColor = .systemBackground
        return UITargetedPreview(view: cell.contentView, parameters: parameters)
    }

    private func menu(for row: RowView) -> UIMenu {
        let store = page.store
        let starred = store.bullet(row.id)?.starred ?? false
        var items: [UIMenuElement] = [
            UIAction(
                title: starred ? NSLocalizedString("Unstar", comment: "Menu item") : NSLocalizedString("Star", comment: "Menu item"),
                image: UIImage(systemName: starred ? "star.slash" : "star")
            ) { _ in store.setStarred(row.id, !starred) },
            UIAction(title: NSLocalizedString("Move To…", comment: "Menu item"), image: UIImage(systemName: "arrow.turn.down.right")) { [weak self] _ in
                self?.presentMoveTo(row)
            },
            UIAction(title: NSLocalizedString("Note", comment: "Menu item and title of the sheet with a bullet's note"), image: UIImage(systemName: "note.text")) { [weak self] _ in
                self?.presentNote(row)
            },
            UIAction(title: NSLocalizedString("Copy Link", comment: "Menu item"), image: UIImage(systemName: "link")) { _ in
                if let link = store.link(row.id) { UIPasteboard.general.string = link }
            },
            UIAction(title: NSLocalizedString("Zoom In", comment: "Opens a bullet as the page, for VoiceOver"), image: UIImage(systemName: "arrow.up.left.and.arrow.down.right")) { [weak self] _ in
                self?.zoom(into: row)
            },
        ]
        items.append(UIMenu(options: .displayInline, children: [
            UIAction(title: NSLocalizedString("Delete", comment: "Menu item: move a bullet to Recently Deleted"), image: UIImage(systemName: "trash"), attributes: .destructive) { [weak self] _ in
                self?.delete(row)
            },
        ]))
        if row.hasChildren {
            items.insert(
                UIAction(
                    title: row.collapsed ? NSLocalizedString("Expand", comment: "Menu item: show a bullet's children") : NSLocalizedString("Collapse", comment: "Menu item: hide a bullet's children"),
                    image: UIImage(systemName: row.collapsed ? "chevron.down" : "chevron.right")
                ) { [weak self] _ in self?.page.toggleFold(row) },
                at: 0
            )
        }
        return UIMenu(children: items)
    }

    // MARK: VoiceOver (§4.4)

    /// A row's level, children, folding and completion, read after its text.
    private func accessibilityStatus(of id: String) -> String? {
        guard let row = rowsByID[id] else { return nil }
        var parts = [String.localizedStringWithFormat(NSLocalizedString("Level %lld", comment: "VoiceOver: how deep a bullet sits on the page, 1 at the top"), Int(row.depth) + 1)]
        if row.hasChildren {
            if let count = page.childCount(row), count > 0 {
                parts.append(String.localizedStringWithFormat(NSLocalizedString("%lld children", comment: "VoiceOver: how many children a bullet has"), count))
            }
            parts.append(row.collapsed
                ? NSLocalizedString("Collapsed", comment: "VoiceOver: a bullet whose children are hidden")
                : NSLocalizedString("Expanded", comment: "VoiceOver: a bullet whose children show"))
        }
        if row.completed {
            parts.append(NSLocalizedString("Completed", comment: "A completed bullet, for VoiceOver"))
        }
        return parts.joined(separator: ", ")
    }

    /// What VoiceOver's actions rotor offers on a row: the keyboard bar's and the long-press
    /// menu's commands, on that row whether or not it is edited.
    private func accessibilityActions(for id: String) -> [UIAccessibilityCustomAction] {
        guard let row = rowsByID[id] else { return [] }
        let store = page.store
        /// `stays`: the row stays on the page, and VoiceOver reads it again where it went.
        func action(_ name: String, _ symbol: String, stays: Bool = false, _ body: @escaping (OutlinePageController, RowView) -> Void) -> UIAccessibilityCustomAction {
            UIAccessibilityCustomAction(name: name, image: UIImage(systemName: symbol)) { [weak self] _ in
                guard let self, let row = rowsByID[id] else { return false }
                body(self, row)
                if stays {
                    DispatchQueue.main.async { [weak self] in self?.focusVoiceOver(on: id) }
                }
                return true
            }
        }
        let starred = store.bullet(id)?.starred ?? false
        var actions = [
            action(NSLocalizedString("Indent", comment: "Keyboard bar button"), "increase.indent", stays: true) { $0.page.indent($1) },
            action(NSLocalizedString("Outdent", comment: "Keyboard bar button"), "decrease.indent", stays: true) { $0.page.outdent($1) },
            action(NSLocalizedString("Move Up", comment: "Keyboard bar button"), "arrow.up", stays: true) { $0.page.moveUp($1) },
            action(NSLocalizedString("Move Down", comment: "Keyboard bar button"), "arrow.down", stays: true) { $0.page.moveDown($1) },
            action(row.completed
                ? NSLocalizedString("Mark Not Completed", comment: "VoiceOver action: mark a completed bullet as not completed")
                : NSLocalizedString("Complete", comment: "Keyboard bar button"),
                row.completed ? "circle" : "checkmark.circle", stays: true) { $0.page.toggle($1) },
        ]
        if row.hasChildren {
            actions.append(action(row.collapsed
                ? NSLocalizedString("Expand", comment: "Menu item: show a bullet's children")
                : NSLocalizedString("Collapse", comment: "Menu item: hide a bullet's children"),
                row.collapsed ? "chevron.down" : "chevron.right", stays: true) { $0.page.toggleFold($1) })
        }
        actions += [
            action(NSLocalizedString("Zoom In", comment: "Opens a bullet as the page, for VoiceOver"), "arrow.up.left.and.arrow.down.right") { $0.zoom(into: $1) },
            action(NSLocalizedString("Move To…", comment: "Menu item"), "arrow.turn.down.right") { $0.presentMoveTo($1) },
            action(starred ? NSLocalizedString("Unstar", comment: "Menu item") : NSLocalizedString("Star", comment: "Menu item"), starred ? "star.slash" : "star") { _, row in
                store.setStarred(row.id, !starred)
            },
            action(NSLocalizedString("Note", comment: "Menu item and title of the sheet with a bullet's note"), "note.text") { $0.presentNote($1) },
        ]
        if let path = row.image?.path, FileManager.default.fileExists(atPath: path) {
            actions.append(action(NSLocalizedString("View Image", comment: "VoiceOver action: show an image row's picture full screen"), "photo") { controller, _ in
                controller.presentImage(path)
            })
        }
        actions.append(action(NSLocalizedString("Delete", comment: "Menu item: move a bullet to Recently Deleted"), "trash") { $0.delete($1) })
        return actions
    }

    /// The 「同级」 and 「父级」 rotors: the next or previous row at the same depth under the same
    /// parent, and the row above this one.
    private lazy var rotors: [UIAccessibilityCustomRotor] = [
        UIAccessibilityCustomRotor(name: NSLocalizedString("Siblings", comment: "VoiceOver rotor: the next or previous bullet under the same parent")) { [weak self] predicate in
            self?.rotorTarget(predicate) { index, depths in
                RowNavigation.sibling(of: index, in: depths, forward: predicate.searchDirection == .next)
            }
        },
        UIAccessibilityCustomRotor(name: NSLocalizedString("Parent", comment: "VoiceOver rotor: the bullet this one sits under")) { [weak self] predicate in
            self?.rotorTarget(predicate) { index, depths in RowNavigation.parent(of: index, in: depths) }
        },
    ]

    private func rotorTarget(_ predicate: UIAccessibilityCustomRotorSearchPredicate, _ find: (Int, [Int]) -> Int?) -> UIAccessibilityCustomRotorItemResult? {
        let current = predicate.currentItem.targetElement ?? UIAccessibility.focusedElement(using: .notificationVoiceOver) as? NSObjectProtocol
        guard let id = rowID(of: current), let index = indexByID[id],
              let target = find(index, order.map { Int(rowsByID[$0]?.depth ?? 0) }),
              let element = voiceOverElement(for: order[target])
        else { return nil }
        return UIAccessibilityCustomRotorItemResult(targetElement: element, targetRange: nil)
    }

    /// The row an accessibility element belongs to: a row, or its text view while edited.
    private func rowID(of element: Any?) -> String? {
        guard let view = element as? UIView else { return nil }
        let cell = sequence(first: view, next: { $0.superview }).first { $0 is RowCell } as? RowCell
        return cell?.textView.rowID
    }

    /// What VoiceOver focuses for a row, scrolled on screen: the row, or its text view while it
    /// is edited.
    private func voiceOverElement(for id: String) -> NSObject? {
        guard let index = indexByID[id] else { return nil }
        let path = IndexPath(item: index, section: 0)
        collectionView.layoutIfNeeded()
        if let frame = collectionView.layoutAttributesForItem(at: path)?.frame, !frame.intersects(visibleRect) {
            placing = true
            collectionView.scrollToItem(at: path, at: .centeredVertically, animated: false)
            collectionView.layoutIfNeeded()
            placing = false
        }
        guard let cell = collectionView.cellForItem(at: path) as? RowCell else { return nil }
        return cell.textView.isFirstResponder ? cell.textView : cell
    }

    private func focusVoiceOver(on id: String) {
        guard UIAccessibility.isVoiceOverRunning, let element = voiceOverElement(for: id) else { return }
        UIAccessibility.post(notification: .layoutChanged, argument: element)
    }

    // MARK: Move To (§2.1)

    /// Pick where a bullet goes (built on search), then move it there as the last child.
    func presentMoveTo(_ row: RowView) {
        view.endEditing(true)
        let page = self.page
        var host: UIViewController?
        let picker = MoveToPicker(store: page.store, moving: row) { target in
            if let target {
                page.move(row.id, to: target.id)
            }
            host?.dismiss(animated: true)
        }
        let controller = UIHostingController(rootView: picker)
        host = controller
        if let sheet = controller.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        present(controller, animated: true)
    }

    func moveTo() {
        if let row = editingRow { presentMoveTo(row) }
    }

    public func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        let point = touch.location(in: collectionView)
        return point.y > rowsBottom
    }

    var editingRow: RowView? {
        editingID.flatMap { rowsByID[$0] }
    }

    @objc func outdent() { if let row = editingRow { page.outdent(row) } }
    @objc func indent() { if let row = editingRow { page.indent(row) } }
    @objc func moveUp() { if let row = editingRow { page.moveUp(row) } }
    @objc func moveDown() { if let row = editingRow { page.moveDown(row) } }
    @objc func complete() { if let row = editingRow { page.toggle(row) } }
    @objc func collapse() { if let row = editingRow, row.hasChildren, !row.collapsed { page.toggleFold(row) } }
    @objc func expand() { if let row = editingRow, row.collapsed { page.toggleFold(row) } }
    @objc func zoomIntoEdited() { if let row = editingRow { zoom(into: row) } }
    @objc func deleteEdited() { if let row = editingRow { delete(row) } }

    /// Move a bullet to Recently Deleted, offering to undo it for a few seconds.
    func delete(_ row: RowView) {
        page.delete(row)
        if page.store.problem == nil {
            undoToast.show()
        }
    }
    @objc func moveToFromKeyboard() { moveTo() }
    func undo() { pageUndo.undo() }
    func redo() { pageUndo.redo() }
    @objc func dismissKeyboard() { view.endEditing(true) }

    public override var undoManager: UndoManager? { pageUndo }

    // MARK: Hardware keyboard (§4.4)

    /// The desktop's keys for the row being edited (docs/usage.md), listed in the ⌘ overlay and
    /// the iPad menu bar. ⌘Z and ⇧⌘Z come from the system's Edit menu, through `pageUndo`.
    private lazy var rowKeyCommands: [UIKeyCommand] = {
        func key(_ title: String, _ input: String, _ flags: UIKeyModifierFlags, _ action: Selector) -> UIKeyCommand {
            let command = UIKeyCommand(title: title, action: action, input: input, modifierFlags: flags)
            command.discoverabilityTitle = title
            // Tab, Return and the arrows are the text view's otherwise.
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
        return [
            key(NSLocalizedString("Indent", comment: "Keyboard bar button"), "\t", [], #selector(indent)),
            key(NSLocalizedString("Outdent", comment: "Keyboard bar button"), "\t", .shift, #selector(outdent)),
            key(NSLocalizedString("Complete", comment: "Keyboard bar button"), "\r", .command, #selector(complete)),
            key(NSLocalizedString("Move Up", comment: "Keyboard bar button"), UIKeyCommand.inputUpArrow, [.command, .shift], #selector(moveUp)),
            key(NSLocalizedString("Move Down", comment: "Keyboard bar button"), UIKeyCommand.inputDownArrow, [.command, .shift], #selector(moveDown)),
            key(NSLocalizedString("Collapse", comment: "Menu item: hide a bullet's children"), UIKeyCommand.inputUpArrow, .command, #selector(collapse)),
            key(NSLocalizedString("Expand", comment: "Menu item: show a bullet's children"), UIKeyCommand.inputDownArrow, .command, #selector(expand)),
            key(NSLocalizedString("Move To…", comment: "Keyboard bar button"), "m", [.command, .shift], #selector(moveToFromKeyboard)),
            key(NSLocalizedString("Zoom In", comment: "Opens a bullet as the page, for VoiceOver"), "]", .command, #selector(zoomIntoEdited)),
            key(NSLocalizedString("Delete", comment: "Menu item: move a bullet to Recently Deleted"), UIKeyCommand.inputDelete, [.command, .shift], #selector(deleteEdited)),
        ]
    }()

    /// The desktop's keys for folding the whole page to a level (⌃1 ⌃2 ⌃3 ⌃0), edited or not.
    private lazy var pageKeyCommands: [UIKeyCommand] = FoldLevel.allCases.map { level in
        let command = UIKeyCommand(title: level.title, action: #selector(foldPage(_:)), input: level.key, modifierFlags: .control, propertyList: level.levels)
        command.discoverabilityTitle = level.title
        command.wantsPriorityOverSystemBehavior = true
        return command
    }

    @objc private func foldPage(_ command: UIKeyCommand) {
        guard let levels = command.propertyList as? UInt32 else { return }
        page.expandToLevel(levels)
    }

    public override var keyCommands: [UIKeyCommand]? {
        (editingID == nil ? super.keyCommands ?? [] : rowKeyCommands) + pageKeyCommands
    }

    /// iPad: the keyboard bar's commands in the shortcuts bar, after the system's own items.
    /// When the bar is narrow, each group folds into one button with a menu.
    private func addShortcuts(to item: UITextInputAssistantItem) {
        func button(_ symbol: String, _ title: String, _ action: Selector) -> UIBarButtonItem {
            let button = UIBarButtonItem(image: UIImage(systemName: symbol), style: .plain, target: self, action: action)
            button.title = title
            button.accessibilityLabel = title
            return button
        }
        func group(_ symbol: String, _ items: [UIBarButtonItem]) -> UIBarButtonItemGroup {
            let menu = UIMenu(children: items.map { item in
                UIAction(title: item.title ?? "", image: item.image) { [weak self] _ in
                    if let action = item.action { self?.perform(action) }
                }
            })
            return UIBarButtonItemGroup(barButtonItems: items, representativeItem: UIBarButtonItem(image: UIImage(systemName: symbol), menu: menu))
        }
        let levels = group("increase.indent", [
            button("decrease.indent", NSLocalizedString("Outdent", comment: "Keyboard bar button"), #selector(outdent)),
            button("increase.indent", NSLocalizedString("Indent", comment: "Keyboard bar button"), #selector(indent)),
        ])
        let rows = group("ellipsis.circle", [
            button("arrow.up", NSLocalizedString("Move Up", comment: "Keyboard bar button"), #selector(moveUp)),
            button("arrow.down", NSLocalizedString("Move Down", comment: "Keyboard bar button"), #selector(moveDown)),
            button("checkmark.circle", NSLocalizedString("Complete", comment: "Keyboard bar button"), #selector(complete)),
            button("arrow.turn.down.right", NSLocalizedString("Move To…", comment: "Keyboard bar button"), #selector(moveToFromKeyboard)),
        ])
        item.leadingBarButtonGroups += [levels]
        item.trailingBarButtonGroups = [rows]
    }

    // MARK: Scrolling

    public func scrollViewDidScroll(_ scrollView: UIScrollView) {
        // The edited row scrolled off screen: editing ends rather than holding the keyboard.
        // (The collection view keeps the cell holding the caret alive off screen, so it
        // never says the cell ended displaying.)
        guard !placing, scrollView.isTracking || scrollView.isDecelerating,
              let id = editingID, let index = indexByID[id],
              let frame = collectionView.layoutAttributesForItem(at: IndexPath(item: index, section: 0))?.frame
        else { return }
        let visible = collectionView.bounds.inset(by: collectionView.adjustedContentInset)
        if !frame.intersects(visible) {
            view.endEditing(true)
        }
    }

    public func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
        if !decelerate { rememberScroll() }
    }

    public func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) {
        rememberScroll()
    }

    /// Remember the top row on screen, to come back to it at launch (§4.3).
    private func rememberScroll() {
        let top = collectionView.contentOffset.y + collectionView.adjustedContentInset.top
        let first = collectionView.indexPathsForVisibleItems
            .compactMap { path in collectionView.layoutAttributesForItem(at: path).map { (path, $0.frame) } }
            .filter { $0.1.maxY > top + 1 }
            .min { $0.0.item < $1.0.item }
        LaunchMemory.rememberScroll(page.kind.key, row: first.flatMap { order.indices.contains($0.0.item) ? order[$0.0.item] : nil })
    }

    // MARK: Typing

    private func row(of textView: UITextView) -> RowView? {
        (textView as? BulletTextView)?.rowID.flatMap { rowsByID[$0] }
    }

    public func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
        // While composing, the keyboard owns Return (it confirms the composition).
        guard (text.contains("\n") || text.contains("\t")), textView.markedTextRange == nil, let row = row(of: textView) else { return true }
        if text == "\n" {
            page.enter(row, from: range.location, to: range.location + range.length)
        } else if text == "\t" {
            // A Tab the key commands didn't take indents, as on the desktop; it is never text.
            page.indent(row)
        } else {
            // A bullet's text is one line: pasted newlines and tabs become spaces, as on the desktop.
            let line = text.replacingOccurrences(of: "\n", with: " ").replacingOccurrences(of: "\t", with: " ")
            textView.textStorage.replaceCharacters(in: range, with: line)
            textView.selectedRange = NSRange(location: range.location + (line as NSString).length, length: 0)
            textViewDidChange(textView)
        }
        return false
    }

    public func textViewDidChange(_ textView: UITextView) {
        // Typing is saved as it goes (one undo step per bullet), once a composition ends.
        guard textView.markedTextRange == nil, let row = row(of: textView) else { return }
        page.setText(row, (textView.text ?? "").replacingOccurrences(of: "\n", with: " "))
        // A line more or less: the row grows or shrinks.
        if let cell = cell(for: row.id), cell.needsResize() {
            cell.invalidateIntrinsicContentSize()
        }
    }

    public func textViewDidBeginEditing(_ textView: UITextView) {
        guard let row = row(of: textView) else { return }
        editingID = row.id
        page.editing(row)
        refreshGuides()
        // The row stops being one element while edited: VoiceOver goes to its text.
        if UIAccessibility.isVoiceOverRunning {
            UIAccessibility.post(notification: .layoutChanged, argument: textView)
        }
    }

    public func textViewDidEndEditing(_ textView: UITextView) {
        // The row shows its markup rendered again.
        if let cell = sequence(first: textView as UIView, next: { $0.superview }).first(where: { $0 is RowCell }) as? RowCell {
            cell.showRendered()
        }
        guard let id = (textView as? BulletTextView)?.rowID, id == editingID else { return }
        // When the caret moved to another row, that row has begun editing by now.
        DispatchQueue.main.async { [weak self] in
            guard let self, editingID == id else { return }
            editingID = nil
            page.editing(nil)
            refreshGuides()
            // VoiceOver was on the text: it goes back to the row, one element again.
            if UIAccessibility.isVoiceOverRunning, let cell = cell(for: id),
               (UIAccessibility.focusedElement(using: .notificationVoiceOver) as? UIView).map({ $0 === textView || $0 === cell }) ?? true {
                UIAccessibility.post(notification: .layoutChanged, argument: cell)
            }
        }
    }
}

/// One row: its bullet slot (with the guide lines), an image row's picture, its text and a line
/// of its note. While the row is edited its text is the stored markup; otherwise it shows
/// rendered (§2.3), read-only, and a tap either follows what is under the finger (a link, a tag,
/// a date) or starts editing there.
final class RowCell: UICollectionViewCell {
    /// What a tap on the rendered text does.
    enum TextTap: Equatable {
        /// Edit the row, the caret at this offset of the stored text (UTF-16).
        case caret(Int)
        case bullet(String)
        case url(URL)
        case search(String)
    }

    let slot = SlotView()
    let textView = BulletTextView()
    private let note = UILabel()
    private let picture = UIImageView()
    private let placeholder = UIImageView(image: UIImage(systemName: "photo"))
    /// Under the placeholder, when the file isn't on this device and the outline syncs through
    /// iCloud: a file comes as a record of its own, maybe after its row (wiki: Per-Node-Sync
    /// §3.5), or not at all from a Mac too old to send it.
    private let notSynced = UILabel()
    var onTextTap: ((TextTap) -> Void)?
    var onNoteTap: (() -> Void)?
    /// A tap on the picture, with its file.
    var onImageTap: ((String) -> Void)?
    /// For VoiceOver: the row's level, children, folding and completion, and its actions.
    var accessibilityStatus: (() -> String?)? {
        didSet { textView.accessibilityStatus = accessibilityStatus }
    }
    var accessibilityActions: (() -> [UIAccessibilityCustomAction])? {
        didSet { textView.accessibilityActions = accessibilityActions }
    }
    private let textTap = UITapGestureRecognizer()
    private var metrics: RowMetrics?
    private var row: RowView?
    private var depth = 0
    private var hasNote = false
    /// What the stored text's attributes were made for (while edited).
    private var styled: (completed: Bool, metrics: RowMetrics)?
    /// The text shown while the row isn't edited, and what it was made from.
    private var rendered: RenderedText?
    private var renderedFor: (text: String, kind: String, completed: Bool, metrics: RowMetrics)?
    /// Whether the text view holds the stored text (the row is or was just edited).
    private var showsStored = false
    /// The picture's file shown (or loading).
    private var picturePath: String?
    /// Whether the outline syncs through iCloud (a missing file then says why).
    private var iCloud = false
    /// The picture's file isn't on this device.
    private var pictureMissing = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        textView.isScrollEnabled = false
        // Tapping the status bar scrolls the page, not a row's text view.
        textView.scrollsToTop = false
        textView.backgroundColor = .clear
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        // Typed text is stored as markup: quotes and dashes stay as typed.
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        // Read-only until it is edited: taps come here.
        textView.isEditable = false
        textView.isSelectable = false
        textTap.addTarget(self, action: #selector(tappedText))
        textView.addGestureRecognizer(textTap)
        textView.willEdit = { [weak self] in self?.showStored() }
        textView.onActivate = { [weak self] in
            guard let self else { return }
            onTextTap?(.caret(rendered?.storedLength ?? 0))
        }
        note.numberOfLines = 1
        note.lineBreakMode = .byTruncatingTail
        note.textColor = .secondaryLabel
        note.isUserInteractionEnabled = true
        note.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tappedNote)))
        note.accessibilityTraits = .button
        note.accessibilityHint = NSLocalizedString("Opens the note", comment: "VoiceOver hint of a bullet's note preview")
        picture.contentMode = .scaleAspectFill
        picture.clipsToBounds = true
        picture.layer.cornerRadius = 6
        picture.layer.cornerCurve = .continuous
        picture.backgroundColor = .secondarySystemFill
        picture.isUserInteractionEnabled = true
        picture.isAccessibilityElement = true
        picture.accessibilityLabel = NSLocalizedString("Image", comment: "VoiceOver label of an image row's picture")
        picture.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tappedPicture)))
        placeholder.tintColor = .tertiaryLabel
        placeholder.contentMode = .center
        picture.addSubview(placeholder)
        notSynced.text = NSLocalizedString("Image not downloaded from iCloud yet", comment: "Caption of an image row whose file hasn't come to this device through iCloud (yet); the picture replaces it when it does")
        notSynced.font = .preferredFont(forTextStyle: .caption1)
        notSynced.adjustsFontForContentSizeCategory = true
        notSynced.textColor = .secondaryLabel
        notSynced.textAlignment = .center
        notSynced.numberOfLines = 2
        notSynced.adjustsFontSizeToFitWidth = true
        notSynced.minimumScaleFactor = 0.7
        notSynced.isHidden = true
        picture.addSubview(notSynced)
        contentView.addSubview(slot)
        contentView.addSubview(picture)
        contentView.addSubview(textView)
        contentView.addSubview(note)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        if textView.isFirstResponder {
            textView.resignFirstResponder()
        }
    }

    func configure(row: RowView, metrics: RowMetrics, strong: Int, iCloud: Bool) {
        self.iCloud = iCloud
        let rowChanged = textView.rowID != row.id
        textView.rowID = row.id
        self.row = row
        self.metrics = metrics
        depth = Int(row.depth)
        let attributes = metrics.textAttributes(completed: row.completed)
        if textView.isFirstResponder {
            // Never under an unfinished composition (pinyin).
            if textView.markedTextRange == nil {
                if rowChanged || !textView.text.utf8.elementsEqual(row.text.utf8) {
                    let selection = textView.selectedRange
                    textView.attributedText = NSAttributedString(string: row.text, attributes: attributes)
                    if !rowChanged {
                        let length = (row.text as NSString).length
                        textView.selectedRange = NSRange(location: min(selection.location, length), length: 0)
                    }
                } else if styled.map({ $0.completed != row.completed || $0.metrics != metrics }) ?? true {
                    let selection = textView.selectedRange
                    textView.textStorage.setAttributes(attributes, range: NSRange(location: 0, length: textView.textStorage.length))
                    textView.selectedRange = selection
                }
                styled = (row.completed, metrics)
            }
        } else {
            showRendered(resize: false)
        }
        textView.typingAttributes = attributes
        textView.alpha = row.completed ? 0.45 : 1

        let preview = row.note.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? ""
        hasNote = !preview.isEmpty
        note.attributedText = hasNote
            ? RenderedText(preview).attributed(base: [.font: metrics.noteFont, .foregroundColor: UIColor.secondaryLabel], font: metrics.noteFont, completed: false)
            : nil
        note.alpha = row.completed ? 0.45 : 1
        note.isHidden = !hasNote

        configurePicture(row.image)

        let deep = depth > metrics.maxIndent
        slot.update(
            guides: (0..<metrics.shown(depth)).map { metrics.bulletX($0) },
            strong: strong,
            bulletX: metrics.bulletX(depth),
            bulletY: metrics.pad + metrics.lineHeight / 2,
            metrics: metrics,
            ring: row.collapsed && row.hasChildren,
            number: deep ? String(depth + 1) : nil,
            completed: row.completed,
            foldable: row.hasChildren
        )
        setNeedsLayout()
    }

    // MARK: VoiceOver (§4.4)

    // The row is one element: its text (and a line of its note), then its level, children,
    // folding and completion, with the row's actions. While it is edited its parts are elements
    // instead, so VoiceOver reads and edits the text view's text.

    override var isAccessibilityElement: Bool {
        get { !textView.isFirstResponder }
        set {}
    }

    override var accessibilityLabel: String? {
        get {
            var parts: [String] = []
            if row?.image != nil {
                parts.append(picture.accessibilityLabel ?? "")
            }
            let text = (rendered?.string ?? row?.text ?? "").trimmingCharacters(in: .whitespaces)
            if !text.isEmpty { parts.append(text) }
            if hasNote, let note = note.attributedText?.string, !note.isEmpty { parts.append(note) }
            return parts.joined(separator: ", ")
        }
        set {}
    }

    override var accessibilityValue: String? {
        get {
            let status = accessibilityStatus?()
            guard let waiting = picture.accessibilityValue else { return status }
            return [status, waiting].compactMap { $0 }.joined(separator: ", ")
        }
        set {}
    }

    override var accessibilityCustomActions: [UIAccessibilityCustomAction]? {
        get { accessibilityActions?() }
        set {}
    }

    /// Double-tap edits the row, the caret at the end of its text.
    override func accessibilityActivate() -> Bool {
        onTextTap?(.caret(rendered?.storedLength ?? (row?.text as NSString?)?.length ?? 0))
        return true
    }

    // MARK: Stored and rendered text

    /// The row is about to be edited: its text view takes the stored text, editable.
    private func showStored() {
        guard let row, let metrics else { return }
        textView.isEditable = true
        textView.isSelectable = true
        textTap.isEnabled = false
        guard !showsStored else { return }
        showsStored = true
        textView.attributedText = NSAttributedString(string: row.text, attributes: metrics.textAttributes(completed: row.completed))
        styled = (row.completed, metrics)
        resizeIfNeeded()
    }

    /// The row isn't edited (any more): its text shows rendered, read-only.
    func showRendered(resize: Bool = true) {
        guard let row, let metrics, !textView.isFirstResponder else { return }
        textView.isEditable = false
        textView.isSelectable = false
        textTap.isEnabled = true
        let current = renderedFor.map { $0.kind == row.kind && $0.completed == row.completed && $0.metrics == metrics && $0.text.utf8.elementsEqual(row.text.utf8) }
        if showsStored || current != true {
            showsStored = false
            let text = RenderedText(row.text, code: row.kind == "code")
            rendered = text
            renderedFor = (row.text, row.kind, row.completed, metrics)
            var attributes = metrics.textAttributes(completed: row.completed)
            if row.kind == "code" {
                attributes[.font] = UIFont.monospacedSystemFont(ofSize: metrics.font.pointSize * 0.94, weight: .regular)
            }
            textView.attributedText = text.attributed(base: attributes, font: metrics.font, completed: row.completed)
            if resize { resizeIfNeeded() }
        }
    }

    /// Stored and rendered text can wrap differently: the row is measured again.
    private func resizeIfNeeded() {
        if needsResize() {
            invalidateIntrinsicContentSize()
        }
    }

    @objc private func tappedText(_ recognizer: UITapGestureRecognizer) {
        guard let rendered, !textView.isFirstResponder else { return }
        let point = recognizer.location(in: textView)
        let start = textView.beginningOfDocument
        let shown = textView.closestPosition(to: point).map { textView.offset(from: start, to: $0) } ?? rendered.string.utf16.count
        for offset in [shown, shown - 1] {
            guard let piece = rendered.piece(at: offset) else { continue }
            let tap: TextTap?
            switch piece.run.kind {
            case .link(let address, let bullet): tap = bullet.map(TextTap.bullet) ?? RenderedText.webURL(address).map(TextTap.url)
            case .tag, .mention: tap = .search(piece.text)
            case .date: tap = .search("date:\(piece.text)")
            default: tap = nil
            }
            guard let tap, let from = textView.position(from: start, offset: piece.shown.lowerBound),
                  let to = textView.position(from: start, offset: piece.shown.upperBound),
                  let range = textView.textRange(from: from, to: to) else { continue }
            if textView.selectionRects(for: range).contains(where: { !$0.rect.isEmpty && $0.rect.insetBy(dx: -1, dy: -2).contains(point) }) {
                onTextTap?(tap)
                return
            }
        }
        onTextTap?(.caret(rendered.storedOffset(shown)))
    }

    @objc private func tappedNote() {
        onNoteTap?()
    }

    @objc private func tappedPicture() {
        if let picturePath { onImageTap?(picturePath) }
    }

    // MARK: Picture

    private func pictureSize(width: CGFloat) -> CGSize? {
        guard let image = row?.image, let metrics else { return nil }
        let textWidth = width - metrics.textX(depth) - metrics.trailing
        return Thumbnail.size(width: image.width, height: image.height, maxWidth: Double(max(16, textWidth)))
    }

    /// The picture's thumbnail, read off the main thread; until it is there (or when the file
    /// isn't on this device) a placeholder of the same size.
    private func configurePicture(_ image: ImageView?) {
        picture.isHidden = image == nil
        guard let image, let path = image.path, let size = pictureSize(width: bounds.width > 0 ? bounds.width : window?.bounds.width ?? 390) else {
            picturePath = nil
            picture.image = nil
            pictureMissing = false
            showPlaceholder()
            return
        }
        if picturePath != path {
            picture.image = nil
            pictureMissing = false
        }
        picturePath = path
        let scale = traitCollection.displayScale > 0 ? traitCollection.displayScale : 2
        let ready = Thumbnails.shared.image(at: path, size: size, scale: scale) { [weak self] loaded in
            guard let self, picturePath == path else { return }
            picture.image = loaded
            pictureMissing = loaded == nil && !FileManager.default.fileExists(atPath: path)
            showPlaceholder()
        }
        if let ready { picture.image = ready }
        showPlaceholder()
    }

    /// The placeholder while there is no picture, and why when the file is missing for iCloud.
    private func showPlaceholder() {
        placeholder.isHidden = picture.image != nil
        let waiting = picture.image == nil && pictureMissing && iCloud
        notSynced.isHidden = !waiting
        picture.accessibilityValue = waiting ? notSynced.text : nil
        setNeedsLayout()
    }

    // MARK: Layout

    private func textHeight(width: CGFloat) -> CGFloat {
        guard let metrics else { return 22 }
        let fitted = textView.sizeThatFits(CGSize(width: max(1, width), height: .greatestFiniteMagnitude)).height
        return max(metrics.lineHeight, ceil(fitted))
    }

    /// The picture's height and the gap below it (none without one).
    private func pictureBlock(width: CGFloat) -> CGFloat {
        pictureSize(width: width).map { $0.height + 4 } ?? 0
    }

    private func height(width: CGFloat) -> CGFloat {
        guard let metrics else { return 28 }
        let textWidth = width - metrics.textX(depth) - metrics.trailing
        return metrics.pad + pictureBlock(width: width) + textHeight(width: textWidth) + (hasNote ? metrics.noteLineHeight : 0) + metrics.pad
    }

    /// Whether the text no longer fits its height (typing wrapped a line or unwrapped one).
    func needsResize() -> Bool {
        abs(height(width: bounds.width) - bounds.height) > 0.5
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let metrics else { return }
        let x = metrics.textX(depth)
        let width = bounds.width - x - metrics.trailing
        var y = metrics.pad
        if let size = pictureSize(width: bounds.width) {
            picture.frame = CGRect(x: x, y: y, width: size.width, height: size.height)
            placeholder.frame = picture.bounds
            if !notSynced.isHidden {
                // The icon a little above the middle, the caption below it.
                let inset = picture.bounds.insetBy(dx: 8, dy: 6)
                let caption = notSynced.sizeThatFits(CGSize(width: inset.width, height: .greatestFiniteMagnitude))
                let captionHeight = min(caption.height, max(0, inset.height / 2))
                placeholder.frame = CGRect(x: 0, y: 0, width: picture.bounds.width, height: picture.bounds.height - captionHeight - 6)
                notSynced.frame = CGRect(x: inset.minX, y: placeholder.frame.maxY, width: inset.width, height: captionHeight)
            }
            y += size.height + 4
        }
        let height = textHeight(width: width)
        slot.frame = CGRect(x: 0, y: 0, width: x, height: bounds.height)
        textView.frame = CGRect(x: x, y: y, width: width, height: height)
        note.frame = CGRect(x: x, y: y + height, width: width, height: hasNote ? metrics.noteLineHeight : 0)
    }

    override func preferredLayoutAttributesFitting(_ attributes: UICollectionViewLayoutAttributes) -> UICollectionViewLayoutAttributes {
        let fitted = super.preferredLayoutAttributesFitting(attributes)
        fitted.frame.size.height = height(width: attributes.size.width)
        return fitted
    }
}

/// A row's left part: the guide lines of its ancestors, and its bullet (a ring around it when
/// it is collapsed with children), or its depth number when the row is deeper than rows move
/// right. Tapping the bullet slot opens the row as a page; double-tapping it folds or unfolds a
/// row with children; holding it opens the row's menu.
final class SlotView: UIView, UIPointerInteractionDelegate {
    var onTap: (() -> Void)?
    /// Double-tapping the bullet of a row with children folds or unfolds it.
    var onDoubleTap: (() -> Void)?
    private let doubleTap = UITapGestureRecognizer()
    private var guides: [CGFloat] = []
    private var strong = 0
    private var bulletX: CGFloat = 0
    private var bulletY: CGFloat = 0
    private var metrics: RowMetrics?
    private var ring = false
    private var number: String?
    private var completed = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        contentMode = .redraw
        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
        doubleTap.numberOfTapsRequired = 2
        doubleTap.addTarget(self, action: #selector(doubleTapped))
        // Rows without children have nothing to fold: their single tap doesn't wait.
        doubleTap.isEnabled = false
        tap.require(toFail: doubleTap)
        addGestureRecognizer(doubleTap)
        addGestureRecognizer(tap)
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityLabel = NSLocalizedString("Zoom In", comment: "Opens a bullet as the page, for VoiceOver")
        // iPad pointer: the bullet highlights under it, as a button does (§4.4). A secondary
        // click opens the row's menu (the context menu interaction), as on the desktop; one on
        // the text gets the text's own menu.
        addInteraction(UIPointerInteraction(delegate: self))
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: Self, _) in
            self.setNeedsDisplay()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    func update(guides: [CGFloat], strong: Int, bulletX: CGFloat, bulletY: CGFloat, metrics: RowMetrics, ring: Bool, number: String?, completed: Bool, foldable: Bool) {
        doubleTap.isEnabled = foldable
        self.guides = guides
        self.strong = strong
        self.bulletX = bulletX
        self.bulletY = bulletY
        self.metrics = metrics
        self.ring = ring
        self.number = number
        self.completed = completed
        setNeedsDisplay()
    }

    func setStrong(_ strong: Int) {
        if strong != self.strong {
            self.strong = strong
            setNeedsDisplay()
        }
    }

    /// The bullet slot, the whole row high (a little wider to the left, for fingers).
    private var tapArea: CGRect {
        guard let metrics else { return .zero }
        return CGRect(x: bulletX - metrics.slot / 2 - 8, y: 0, width: metrics.slot + 8, height: bounds.height)
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        tapArea.contains(point)
    }

    @objc private func tapped() {
        onTap?()
    }

    func pointerInteraction(_ interaction: UIPointerInteraction, regionFor request: UIPointerRegionRequest, defaultRegion: UIPointerRegion) -> UIPointerRegion? {
        tapArea.contains(request.location) ? UIPointerRegion(rect: tapArea) : nil
    }

    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        guard let metrics else { return nil }
        // Round, around the bullet (wider for a number).
        let height = metrics.ring + 8
        var width = height
        if let number {
            let font = UIFont.monospacedDigitSystemFont(ofSize: metrics.noteFont.pointSize - 2, weight: .medium)
            width = max(height, (number as NSString).size(withAttributes: [.font: font]).width + 14)
        }
        let shape = UIBezierPath(roundedRect: CGRect(x: bulletX - width / 2, y: bulletY - height / 2, width: width, height: height), cornerRadius: height / 2)
        let parameters = UIPreviewParameters()
        parameters.visiblePath = shape
        return UIPointerStyle(effect: .highlight(UITargetedPreview(view: self, parameters: parameters)), shape: .path(shape))
    }

    @objc private func doubleTapped() {
        onDoubleTap?()
    }

    override func draw(_ rect: CGRect) {
        guard let metrics else { return }
        let hairline: CGFloat = 0.33
        for (i, x) in guides.enumerated() where abs(x - bulletX) > 0.5 {
            UIColor.label.withAlphaComponent(i < strong ? 0.3 : 0.1).setFill()
            UIRectFill(CGRect(x: x - hairline / 2, y: 0, width: hairline, height: bounds.height))
        }
        let alpha: CGFloat = completed ? 0.45 : 1
        if let number {
            let font = UIFont.monospacedDigitSystemFont(ofSize: metrics.noteFont.pointSize - 2, weight: .medium)
            let text = NSAttributedString(string: number, attributes: [.font: font, .foregroundColor: UIColor.secondaryLabel.withAlphaComponent(alpha)])
            let size = text.size()
            if ring {
                UIColor.label.withAlphaComponent(0.12 * alpha).setFill()
                let d = max(metrics.ring, size.width + 6)
                UIBezierPath(ovalIn: CGRect(x: bulletX - d / 2, y: bulletY - metrics.ring / 2, width: d, height: metrics.ring)).fill()
            }
            text.draw(at: CGPoint(x: bulletX - size.width / 2, y: bulletY - size.height / 2))
            return
        }
        if ring {
            UIColor.label.withAlphaComponent(0.12 * alpha).setFill()
            let r = metrics.ring / 2
            UIBezierPath(ovalIn: CGRect(x: bulletX - r, y: bulletY - r, width: 2 * r, height: 2 * r)).fill()
        }
        UIColor.label.withAlphaComponent(0.6 * alpha).setFill()
        let r = metrics.bullet / 2
        UIBezierPath(ovalIn: CGRect(x: bulletX - r, y: bulletY - r, width: 2 * r, height: 2 * r)).fill()
    }
}

/// A row's UITextView: it reports Backspace with the caret at the start (UIKit doesn't ask
/// `shouldChangeTextIn` there), and says when it is about to be edited, so its row swaps the
/// rendered text for the stored one first.
final class BulletTextView: UITextView {
    var rowID: String?
    var onBackspaceAtStart: (() -> Void)?
    var willEdit: (() -> Void)?
    /// VoiceOver activated the row while it wasn't edited.
    var onActivate: (() -> Void)?
    /// The page's undo manager: ⌘Z, shake and the shortcuts bar undo outline steps.
    weak var pageUndo: UndoManager?
    /// While edited, VoiceOver reads the text as the value, the row's status as the hint, and
    /// offers the row's actions (`RowCell`).
    var accessibilityStatus: (() -> String?)?
    var accessibilityActions: (() -> [UIAccessibilityCustomAction])?

    override var accessibilityHint: String? {
        get { accessibilityStatus?() }
        set {}
    }

    override var accessibilityCustomActions: [UIAccessibilityCustomAction]? {
        get { accessibilityActions?() }
        set {}
    }

    override var undoManager: UndoManager? { pageUndo ?? super.undoManager }

    @discardableResult override func becomeFirstResponder() -> Bool {
        if !isFirstResponder {
            willEdit?()
        }
        return super.becomeFirstResponder()
    }

    override func accessibilityActivate() -> Bool {
        guard !isFirstResponder, let onActivate else { return super.accessibilityActivate() }
        onActivate()
        return true
    }

    override func deleteBackward() {
        if selectedRange == NSRange(location: 0, length: 0) && markedTextRange == nil {
            onBackspaceAtStart?()
        } else {
            super.deleteBackward()
        }
    }
}

/// The bar above the keyboard, one for all rows (§4.4): 44pt high, 44pt cells: outdent,
/// indent, move up, move down, complete, move to…, undo (hold: redo), and dismiss the keyboard
/// at the right. With a hardware keyboard the bar alone sits at the bottom of the screen: it
/// grows by the safe area there, its buttons above the Home indicator.
final class KeyboardBar: UIInputView {
    private weak var controller: OutlinePageController?

    init(controller: OutlinePageController) {
        self.controller = controller
        super.init(frame: CGRect(x: 0, y: 0, width: 390, height: 44), inputViewStyle: .keyboard)
        // A flexible height: the keyboard sizes the bar by its intrinsic size, not its frame.
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        allowsSelfSizing = true
        let items: [(String, String, Selector)] = [
            ("decrease.indent", NSLocalizedString("Outdent", comment: "Keyboard bar button"), #selector(outdent)),
            ("increase.indent", NSLocalizedString("Indent", comment: "Keyboard bar button"), #selector(indent)),
            ("arrow.up", NSLocalizedString("Move Up", comment: "Keyboard bar button"), #selector(moveUp)),
            ("arrow.down", NSLocalizedString("Move Down", comment: "Keyboard bar button"), #selector(moveDown)),
            ("checkmark.circle", NSLocalizedString("Complete", comment: "Keyboard bar button"), #selector(complete)),
            ("arrow.turn.down.right", NSLocalizedString("Move To…", comment: "Keyboard bar button"), #selector(moveTo)),
            ("arrow.uturn.backward", NSLocalizedString("Undo", comment: "Keyboard bar button; hold to redo"), #selector(undo)),
        ]
        var x: CGFloat = 4
        for (symbol, label, action) in items {
            let button = makeButton(symbol, label, action)
            button.frame = CGRect(x: x, y: 0, width: 44, height: 44)
            addSubview(button)
            x += 44
            if action == #selector(undo) {
                button.addGestureRecognizer(UILongPressGestureRecognizer(target: self, action: #selector(heldUndo)))
                button.accessibilityHint = NSLocalizedString("Hold to redo", comment: "VoiceOver hint of the Undo button")
            }
        }
        let dismiss = makeButton("keyboard.chevron.compact.down", NSLocalizedString("Hide Keyboard", comment: "Keyboard bar button"), #selector(dismissKeyboard))
        dismiss.frame = CGRect(x: bounds.width - 48, y: 0, width: 44, height: 44)
        dismiss.autoresizingMask = .flexibleLeftMargin
        addSubview(dismiss)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: 44 + bottomPad)
    }

    /// Room below the buttons: the bottom safe area when the bar sits at the bottom of the
    /// screen (a hardware keyboard), none above an on-screen keyboard.
    private var bottomPad: CGFloat = 0 {
        didSet {
            if bottomPad != oldValue {
                invalidateIntrinsicContentSize()
            }
        }
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        bottomPad = safeAreaInsets.bottom
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        bottomPad = safeAreaInsets.bottom
    }

    private func makeButton(_ symbol: String, _ label: String, _ action: Selector) -> UIButton {
        let button = UIButton(type: .system)
        button.setImage(UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17)), for: .normal)
        button.tintColor = .label
        button.accessibilityLabel = label
        button.addTarget(self, action: action, for: .touchUpInside)
        return button
    }

    @objc private func outdent() { controller?.outdent() }
    @objc private func indent() { controller?.indent() }
    @objc private func moveUp() { controller?.moveUp() }
    @objc private func moveDown() { controller?.moveDown() }
    @objc private func complete() { controller?.complete() }
    @objc private func moveTo() { controller?.moveTo() }
    @objc private func undo() { controller?.undo() }
    @objc private func dismissKeyboard() { controller?.dismissKeyboard() }

    @objc private func heldUndo(_ recognizer: UILongPressGestureRecognizer) {
        if recognizer.state == .began {
            controller?.redo()
        }
    }
}

/// Undo for the rows' text views and the system's undo gestures: steps are the outline's
/// (`PageModel.undo`, coalesced in Rust: typing in a bullet is one step), so the text view's
/// own undo stack is never used. What UIKit registers here stays unused (one group at most).
final class PageUndoManager: UndoManager {
    private weak var page: PageModel?

    init(page: PageModel) {
        self.page = page
        super.init()
        levelsOfUndo = 1
    }

    override var canUndo: Bool { MainActor.assumeIsolated { page?.store.outline.canUndo() ?? false } }
    override var canRedo: Bool { MainActor.assumeIsolated { page?.store.outline.canRedo() ?? false } }
    override var undoActionName: String { "" }
    override var redoActionName: String { "" }
    override var undoMenuItemTitle: String { NSLocalizedString("Undo", comment: "Keyboard bar button; hold to redo") }

    override func undo() {
        MainActor.assumeIsolated { page?.undo() }
        NotificationCenter.default.post(name: .NSUndoManagerDidUndoChange, object: self)
    }

    override func redo() {
        MainActor.assumeIsolated { page?.redo() }
        NotificationCenter.default.post(name: .NSUndoManagerDidRedoChange, object: self)
    }

    override func undoNestedGroup() { undo() }
}
#endif
