#if canImport(UIKit)
import SwiftUI
import UIKit
import FlowieFFI

/// The outline page on iOS (wiki: iOS-Cross-Platform §4.1, §4.2): a plain UICollectionView
/// list with a diffable data source, one UITextView per row. Dense: no cards or separators,
/// 28pt a line at the default text size. Typing reconfigures only its row; rows coming, going
/// or moving apply a new snapshot of ids; the row being edited is never reloaded, and the caret
/// moves before the edit returns, so the next key lands where the edit put it. Tapping a bullet
/// pushes its page (`open`).
public struct OutlinePage: UIViewControllerRepresentable {
    let page: PageModel
    /// Open a bullet as a page (push it).
    let open: (String) -> Void

    public init(page: PageModel, open: @escaping (String) -> Void) {
        self.page = page
        self.open = open
    }

    public func makeUIViewController(context: Context) -> OutlinePageController {
        OutlinePageController(page: page, open: open)
    }

    public func updateUIViewController(_ controller: OutlinePageController, context: Context) {
        controller.open = open
    }
}

/// Sizes of a row at the current text size and width (§4.1). At the default (Large) size:
/// 17pt text on 22pt lines, 3pt above and below, a 20pt bullet slot, 16pt a level.
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
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>!
    private var rowsByID: [String: RowView] = [:]
    private var order: [String] = []
    private var indexByID: [String: Int] = [:]
    private var metrics = RowMetrics(width: 390, traits: .current)
    /// The row being typed in.
    private var editingID: String?
    /// For each row, how many of its guide lines (from the left) belong to the edited row's
    /// ancestors, drawn stronger.
    private var strong: [String: Int] = [:]
    /// While a snapshot is applied or the caret placed, rows leaving the screen keep the caret.
    private var placing = false
    private lazy var bar = KeyboardBar(controller: self)
    /// A grey line below the rows: how to open a bullet (new users, on Home), or how to start a
    /// day of the journal.
    private let hint = UILabel()
    /// Whether the rows have been shown once (the remembered scroll position applies then).
    private var shownOnce = false

    init(page: PageModel, open: @escaping (String) -> Void) {
        self.page = page
        self.open = open
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
        let layout = UICollectionViewCompositionalLayout.list(using: config)
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: layout)
        collectionView.backgroundColor = .systemBackground
        collectionView.keyboardDismissMode = .interactive
        // A row that gains or loses a line while typed in asks to be measured again.
        collectionView.selfSizingInvalidation = .enabled
        collectionView.alwaysBounceVertical = true
        collectionView.delegate = self
        collectionView.translatesAutoresizingMaskIntoConstraints = false
        // Room below the last row to tap for a new one.
        collectionView.contentInset.bottom = Self.roomBelow
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
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardMoved), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardMoved), name: UIResponder.keyboardWillHideNotification, object: nil)

        hint.font = .preferredFont(forTextStyle: .subheadline)
        hint.adjustsFontForContentSizeCategory = true
        hint.textColor = .tertiaryLabel
        hint.numberOfLines = 0
        collectionView.addSubview(hint)

        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (self: Self, _) in
            self.remeasure()
        }
        page.onChange = { [weak self] change in self?.pageChanged(change) }
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
    }

    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        page.isVisible = false
    }

    public override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        layoutHint()
    }

    /// Room below the last row: to tap for a new one, and for the keyboard over the page.
    private static let roomBelow: CGFloat = 120

    @objc private func keyboardMoved(_ note: Notification) {
        guard let window = view.window else { return }
        var covered: CGFloat = 0
        if note.name != UIResponder.keyboardWillHideNotification,
           let end = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue {
            let keyboard = view.convert(window.convert(end, from: nil), from: window)
            covered = max(0, view.bounds.maxY - keyboard.minY)
            // The safe area at the bottom (tab bar, home indicator) is under the keyboard too.
            covered = max(0, covered - view.safeAreaInsets.bottom)
        }
        collectionView.contentInset.bottom = Self.roomBelow + covered
        collectionView.verticalScrollIndicatorInsets.bottom = covered
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

    /// The page changed its rows, inside the edit that changed them.
    private func pageChanged(_ change: PageChange) {
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
            read()
            let changed = order.filter { id in before[id].map { $0 != rowsByID[id] } ?? false }
            applySnapshot(reconfigure: changed, animated: true)
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
        revealIfAsked()
        layoutHint()
    }

    private func cell(for id: String) -> RowCell? {
        guard let index = indexByID[id] else { return nil }
        return collectionView.cellForItem(at: IndexPath(item: index, section: 0)) as? RowCell
    }

    /// Put the caret where an edit said, scrolling its row into view.
    private func place(_ focus: Focus) {
        guard let index = indexByID[focus.id] else { return }
        let path = IndexPath(item: index, section: 0)
        placing = true
        defer { placing = false }
        collectionView.layoutIfNeeded()
        var target = collectionView.cellForItem(at: path) as? RowCell
        if target == nil {
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
        let frame = collectionView.convert(target.bounds, from: target)
        collectionView.scrollRectToVisible(frame.insetBy(dx: 0, dy: -8), animated: false)
    }

    private func configure(_ cell: RowCell, _ row: RowView) {
        let textView = cell.textView
        if textView.delegate == nil {
            textView.delegate = self
            textView.inputAccessoryView = bar
            textView.onBackspaceAtStart = { [weak self, weak textView] in
                guard let self, let id = textView?.rowID, let row = rowsByID[id] else { return }
                page.backspaceAtStart(row)
            }
            cell.slot.addInteraction(UIContextMenuInteraction(delegate: self))
        }
        cell.slot.onTap = { [weak self, weak cell] in
            guard let self, let id = cell?.textView.rowID, let row = rowsByID[id] else { return }
            zoom(into: row)
        }
        cell.slot.onDoubleTap = { [weak self, weak cell] in
            guard let self, let id = cell?.textView.rowID, let row = rowsByID[id] else { return }
            page.toggleFold(row)
        }
        cell.configure(row: row, metrics: metrics, strong: strong[row.id] ?? 0)
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
        let y = order.isEmpty ? metrics.pad + 4 : collectionView.collectionViewLayout.collectionViewContentSize.height + 8
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
        return UIContextMenuConfiguration(identifier: nil, previewProvider: nil) { [weak self] _ in
            self?.menu(for: row)
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
            UIAction(title: NSLocalizedString("Copy Link", comment: "Menu item"), image: UIImage(systemName: "link")) { _ in
                if let link = store.link(row.id) { UIPasteboard.general.string = link }
            },
            UIAction(title: NSLocalizedString("Zoom In", comment: "Opens a bullet as the page, for VoiceOver"), image: UIImage(systemName: "arrow.up.left.and.arrow.down.right")) { [weak self] _ in
                self?.zoom(into: row)
            },
        ]
        items.append(UIMenu(options: .displayInline, children: [
            UIAction(title: NSLocalizedString("Delete", comment: "Menu item: move a bullet to Recently Deleted"), image: UIImage(systemName: "trash"), attributes: .destructive) { [weak self] _ in
                self?.page.delete(row)
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
        return point.y > collectionView.collectionViewLayout.collectionViewContentSize.height
    }

    var editingRow: RowView? {
        editingID.flatMap { rowsByID[$0] }
    }

    func outdent() { if let row = editingRow { page.outdent(row) } }
    func indent() { if let row = editingRow { page.indent(row) } }
    func moveUp() { if let row = editingRow { page.moveUp(row) } }
    func moveDown() { if let row = editingRow { page.moveDown(row) } }
    func complete() { if let row = editingRow { page.toggle(row) } }
    func undo() { page.undo() }
    func redo() { page.redo() }
    func dismissKeyboard() { view.endEditing(true) }

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
        guard text.contains("\n"), textView.markedTextRange == nil, let row = row(of: textView) else { return true }
        if text == "\n" {
            page.enter(row, from: range.location, to: range.location + range.length)
        } else {
            // A bullet's text is one line: pasted newlines become spaces, as on the desktop.
            textView.textStorage.replaceCharacters(in: range, with: text.replacingOccurrences(of: "\n", with: " "))
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
    }

    public func textViewDidEndEditing(_ textView: UITextView) {
        guard let id = (textView as? BulletTextView)?.rowID, id == editingID else { return }
        // When the caret moved to another row, that row has begun editing by now.
        DispatchQueue.main.async { [weak self] in
            guard let self, editingID == id else { return }
            editingID = nil
            page.editing(nil)
            refreshGuides()
        }
    }
}

/// One row: its bullet slot (with the guide lines), its text and a line of its note.
final class RowCell: UICollectionViewCell {
    let slot = SlotView()
    let textView = BulletTextView()
    private let note = UILabel()
    private var metrics: RowMetrics?
    private var depth = 0
    private var hasNote = false
    /// What the text's attributes were made for.
    private var styled: (completed: Bool, metrics: RowMetrics)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        textView.isScrollEnabled = false
        textView.backgroundColor = .clear
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        // Typed text is stored as markup: quotes and dashes stay as typed.
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        note.numberOfLines = 1
        note.lineBreakMode = .byTruncatingTail
        note.textColor = .secondaryLabel
        contentView.addSubview(slot)
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

    func configure(row: RowView, metrics: RowMetrics, strong: Int) {
        let rowChanged = textView.rowID != row.id
        textView.rowID = row.id
        self.metrics = metrics
        depth = Int(row.depth)
        let attributes = metrics.textAttributes(completed: row.completed)
        // Never under an unfinished composition (pinyin).
        if textView.markedTextRange == nil {
            if rowChanged || textView.text != row.text {
                let selection = textView.selectedRange
                textView.attributedText = NSAttributedString(string: row.text, attributes: attributes)
                if textView.isFirstResponder && !rowChanged {
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
        textView.typingAttributes = attributes
        textView.alpha = row.completed ? 0.45 : 1
        textView.accessibilityValue = row.completed ? NSLocalizedString("Completed", comment: "A completed bullet, for VoiceOver") : nil

        let preview = row.note.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? ""
        hasNote = !preview.isEmpty
        note.text = preview
        note.font = metrics.noteFont
        note.alpha = row.completed ? 0.45 : 1
        note.isHidden = !hasNote

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

    private func textHeight(width: CGFloat) -> CGFloat {
        guard let metrics else { return 22 }
        let fitted = textView.sizeThatFits(CGSize(width: max(1, width), height: .greatestFiniteMagnitude)).height
        return max(metrics.lineHeight, ceil(fitted))
    }

    private func height(width: CGFloat) -> CGFloat {
        guard let metrics else { return 28 }
        let textWidth = width - metrics.textX(depth) - metrics.trailing
        return metrics.pad + textHeight(width: textWidth) + (hasNote ? metrics.noteLineHeight : 0) + metrics.pad
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
        let height = textHeight(width: width)
        slot.frame = CGRect(x: 0, y: 0, width: x, height: bounds.height)
        textView.frame = CGRect(x: x, y: metrics.pad, width: width, height: height)
        note.frame = CGRect(x: x, y: metrics.pad + height, width: width, height: hasNote ? metrics.noteLineHeight : 0)
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
final class SlotView: UIView {
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
/// `shouldChangeTextIn` there).
final class BulletTextView: UITextView {
    var rowID: String?
    var onBackspaceAtStart: (() -> Void)?

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
/// at the right.
final class KeyboardBar: UIInputView {
    private weak var controller: OutlinePageController?

    init(controller: OutlinePageController) {
        self.controller = controller
        super.init(frame: CGRect(x: 0, y: 0, width: 390, height: 44), inputViewStyle: .keyboard)
        autoresizingMask = .flexibleWidth
        allowsSelfSizing = false
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
        CGSize(width: UIView.noIntrinsicMetric, height: 44)
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
#endif
