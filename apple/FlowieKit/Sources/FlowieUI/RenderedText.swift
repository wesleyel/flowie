import Foundation
import FlowieFFI

/// A bullet's text as a row shows it when it isn't being edited (wiki: iOS-Cross-Platform §2.3,
/// §4.1): the desktop's markup runs (`markup`, from flowie-ffi) with the syntax hidden, and long
/// web addresses cut in the middle. The row being edited shows the stored text instead. Pieces
/// keep where they came from, so a tap on the shown text puts the caret at the same place in the
/// stored text. Offsets are UTF-16, as UIKit and flowie-ffi count.
public struct RenderedText: Equatable {
    /// A stretch of the shown text, and the stretch of the stored text it shows.
    public struct Piece: Equatable {
        public var text: String
        /// How it looks: its run's kind and style.
        public var run: MarkupRun
        /// Where it is in `string`.
        public var shown: Range<Int>
        /// Where it is in the stored text. As long as `shown`, except for the "…" standing for
        /// the middle of a long address.
        public var stored: Range<Int>
    }

    public private(set) var pieces: [Piece] = []
    /// The text shown.
    public private(set) var string = ""
    /// The stored text's length.
    public let storedLength: Int

    /// Link labels longer than this many characters are cut in the middle (§4.1).
    public static let linkLimit = 28

    /// Addresses a row can hand to the system browser. Internal links navigate inside Flowie.
    public static func webURL(_ address: String) -> URL? {
        guard let url = URL(string: address), let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http", let host = url.host, !host.isEmpty else { return nil }
        return url
    }

    /// `code`: a code block, shown as stored (no markup).
    public init(_ stored: String, code: Bool = false) {
        let units = Array(stored.utf16)
        storedLength = units.count
        let runs = code
            ? [MarkupRun(start: 0, end: UInt32(units.count), kind: .text, bold: false, italic: false, strike: false, highlight: false, color: nil)]
            : markup(text: stored)
        for run in runs where run.kind != .marker && run.end > run.start {
            let range = Int(run.start)..<Int(run.end)
            let text = String(decoding: units[range], as: UTF16.self)
            if case .link = run.kind, text.count > Self.linkLimit {
                let head = String(text.prefix(Self.linkLimit - 10))
                let tail = String(text.suffix(9))
                let headEnd = range.lowerBound + head.utf16.count
                let tailStart = range.upperBound - tail.utf16.count
                append(head, run, stored: range.lowerBound..<headEnd)
                append("…", run, stored: headEnd..<tailStart)
                append(tail, run, stored: tailStart..<range.upperBound)
            } else {
                append(text, run, stored: range)
            }
        }
    }

    private mutating func append(_ text: String, _ run: MarkupRun, stored: Range<Int>) {
        let start = pieces.last?.shown.upperBound ?? 0
        string += text
        pieces.append(Piece(text: text, run: run, shown: start..<(start + text.utf16.count), stored: stored))
    }

    /// Where offset `shown` of the shown text is in the stored text: a caret at the edge of a
    /// hidden piece of syntax stays on the visible side.
    public func storedOffset(_ shown: Int) -> Int {
        for piece in pieces where shown <= piece.shown.upperBound {
            if shown <= piece.shown.lowerBound { return piece.stored.lowerBound }
            if piece.shown.count == piece.stored.count {
                return piece.stored.lowerBound + (shown - piece.shown.lowerBound)
            }
            return piece.stored.upperBound
        }
        return pieces.last?.stored.upperBound ?? storedLength
    }

    /// The piece at offset `shown` (the character after it).
    public func piece(at shown: Int) -> Piece? {
        pieces.first { $0.shown.contains(shown) }
    }
}

/// How big an image row's picture shows (§2.3): its pixel size at 2×, scaled down to at most
/// 64pt high and the text's width, so the row's height is known before the file is read. An
/// image of unknown size (synced without its file) gets a placeholder of the size it will have.
public enum Thumbnail {
    public static let maxHeight: Double = 64

    public static func size(width: UInt32, height: UInt32, maxWidth: Double) -> CGSize {
        guard width > 0, height > 0 else {
            return CGSize(width: min(maxWidth, maxHeight * 1.5), height: maxHeight)
        }
        let (w, h) = (Double(width) / 2, Double(height) / 2)
        let scale = min(maxWidth / w, maxHeight / h, 1)
        return CGSize(width: max(16, (w * scale).rounded()), height: max(16, (h * scale).rounded()))
    }
}
