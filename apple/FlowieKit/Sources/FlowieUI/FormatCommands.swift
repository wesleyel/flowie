#if canImport(UIKit)
import UIKit
import FlowieFFI

/// The "Aa" format menu of the row being typed in (wiki: iOS-Cross-Platform §4.4): the keyboard
/// bar's Aa button on iPhone, the shortcuts bar's on iPad, and the desktop's keys. Each command
/// writes the desktop's markup (`toggleFormat`, from flowie-ffi: `markup::toggle_wrap` and
/// `markup::insert_link`), so a selection is wrapped or unwrapped, and a caret gets the pair.
extension TextFormat: @retroactive CaseIterable {
    public static var allCases: [TextFormat] { [.bold, .italic, .strikethrough, .code, .highlight, .link] }

    var title: String {
        switch self {
        case .bold: NSLocalizedString("Bold", comment: "Format menu item: **bold** text")
        case .italic: NSLocalizedString("Italic", comment: "Format menu item: *italic* text")
        case .strikethrough: NSLocalizedString("Strikethrough", comment: "Format menu item: ~~struck through~~ text")
        case .code: NSLocalizedString("Inline Code", comment: "Format menu item: `code` text")
        case .highlight: NSLocalizedString("Highlight", comment: "Format menu item: ==highlighted== text")
        case .link: NSLocalizedString("Link", comment: "Format menu item: makes the selection a [link](https://)")
        }
    }

    var symbol: String {
        switch self {
        case .bold: "bold"
        case .italic: "italic"
        case .strikethrough: "strikethrough"
        case .code: "chevron.left.forwardslash.chevron.right"
        case .highlight: "highlighter"
        case .link: "link"
        }
    }

    /// The desktop's key (crates/flowie-app keymap), if it has one. ⌘B and ⌘I are the system's
    /// Bold and Italic, which the row's text view takes (`BulletTextView`).
    var key: (input: String, flags: UIKeyModifierFlags)? {
        switch self {
        case .bold: ("b", .command)
        case .italic: ("i", .command)
        case .strikethrough: nil
        case .code: ("e", .command)
        case .highlight: ("y", [.command, .shift])
        case .link: ("k", .command)
        }
    }

    /// `text` with the command done to the selection `range` (UTF-16), and the new selection.
    func apply(to text: String, range: NSRange) -> (text: String, selection: NSRange) {
        let out = toggleFormat(text: text, start: UInt32(range.location), end: UInt32(range.location + range.length), format: self)
        return (out.text, NSRange(location: Int(out.start), length: Int(out.end) - Int(out.start)))
    }
}
#endif
