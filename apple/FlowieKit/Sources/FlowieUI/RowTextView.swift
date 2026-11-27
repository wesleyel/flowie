#if canImport(UIKit)
import SwiftUI
import UIKit
import FlowieFFI

/// A bullet's text on iOS: a UITextView, since SwiftUI's text fields don't report Backspace at
/// the start of the text. Return splits the bullet and Backspace at the start joins it to the
/// one above, as on the desktop. A stopgap until the page becomes a UICollectionView
/// (wiki: iOS-Cross-Platform §4), which takes over the text view and the caret.
struct RowTextView: UIViewRepresentable {
    @ObservedObject var store: OutlineStore
    let row: RowView

    func makeCoordinator() -> Coordinator {
        Coordinator(store: store, row: row)
    }

    func makeUIView(context: Context) -> BulletTextView {
        let view = BulletTextView()
        view.delegate = context.coordinator
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.font = .preferredFont(forTextStyle: .body)
        view.adjustsFontForContentSizeCategory = true
        // Typed text is stored as markup: quotes and dashes stay as typed.
        view.smartQuotesType = .no
        view.smartDashesType = .no
        view.text = row.text
        let coordinator = context.coordinator
        view.onBackspaceAtStart = { [weak coordinator] in coordinator?.backspaceAtStart() }
        return view
    }

    func updateUIView(_ view: BulletTextView, context: Context) {
        // The store can publish new rows before the list hands this view its new row (typing
        // would then be undone): read the row afresh.
        let row = store.rows.first { $0.id == self.row.id } ?? self.row
        context.coordinator.store = store
        context.coordinator.row = row
        // The text follows the outline (a split, a join or an undo changes it), except while
        // a composition (pinyin) is under way.
        if view.markedTextRange == nil && view.text != row.text {
            let caret = view.selectedRange.location
            view.text = row.text
            view.selectedRange = NSRange(location: min(caret, (row.text as NSString).length), length: 0)
        }
        view.textColor = row.completed ? .secondaryLabel : .label
        // A key moved the caret to this row.
        if let focus = store.focus, focus.id == row.id {
            store.focus = nil
            view.pendingCaret = row.text.utf16Offset(utf8: focus.offset)
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: BulletTextView, context: Context) -> CGSize? {
        let width = proposal.width ?? 320
        let fitted = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        return CGSize(width: width, height: ceil(fitted.height))
    }

    @MainActor
    final class Coordinator: NSObject, UITextViewDelegate {
        var store: OutlineStore
        var row: RowView

        init(store: OutlineStore, row: RowView) {
            self.store = store
            self.row = row
        }

        func textView(_ view: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
            // While composing, the keyboard owns Return (it confirms the composition).
            guard text.contains("\n"), view.markedTextRange == nil else { return true }
            if text == "\n" {
                let current = view.text ?? ""
                let start = current.utf8Offset(utf16: range.location)
                let end = current.utf8Offset(utf16: range.location + range.length)
                store.enter(row, from: start, to: end)
            } else {
                // A bullet's text is one line: pasted newlines become spaces, as on the desktop.
                view.textStorage.replaceCharacters(in: range, with: text.replacingOccurrences(of: "\n", with: " "))
                textViewDidChange(view)
            }
            return false
        }

        func textViewDidChange(_ view: UITextView) {
            // Typing is saved as it goes (one undo step per bullet), once a composition ends.
            if view.markedTextRange == nil {
                store.setText(row, (view.text ?? "").replacingOccurrences(of: "\n", with: " "))
            }
        }

        func textViewDidBeginEditing(_ view: UITextView) {
            store.editing(row)
        }

        func textViewDidEndEditing(_ view: UITextView) {
            store.editing(nil)
        }

        func backspaceAtStart() {
            store.backspaceAtStart(row)
        }
    }
}

/// A UITextView that reports Backspace with the caret at the start, and takes the caret when a
/// key put it here (once it is on screen).
final class BulletTextView: UITextView {
    var onBackspaceAtStart: (() -> Void)?
    /// Where the caret goes (UTF-16) once this view is in a window.
    var pendingCaret: Int? {
        // After the view update that set it: taking the caret ends editing in another row.
        didSet { DispatchQueue.main.async { [weak self] in self?.takeCaret() } }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        takeCaret()
    }

    private func takeCaret() {
        guard let caret = pendingCaret, window != nil else { return }
        pendingCaret = nil
        becomeFirstResponder()
        selectedRange = NSRange(location: min(caret, (text as NSString).length), length: 0)
    }

    override func deleteBackward() {
        if selectedRange == NSRange(location: 0, length: 0) && markedTextRange == nil {
            onBackspaceAtStart?()
        } else {
            super.deleteBackward()
        }
    }
}
#endif
