import FlowieFFI
import Foundation

extension Outline {
    /// The rows of the page at `root` (nil: Home), as `page(root:)` has them, read from
    /// `pagePacked(root:)`'s bytes: UniFFI reads a list of rows field by field, which took
    /// 45–75 ms for 20 000 rows on iOS, paid again by every structural edit (wiki: TODO-ios
    /// ios:24). The format is flowie-ffi's `Outline::page_packed`.
    public func pageRows(root: String?) throws -> [RowView] {
        try PackedRows.read(pagePacked(root: root))
    }
}

/// Reads `Outline::page_packed`'s format: little-endian, the row count (u32), then per row its
/// depth (u16), flags (u8: 1 completed, 2 collapsed, 4 has children, 8 image, 16 image file),
/// id, text, note and kind as strings (u32 byte length, UTF-8), and for an image its width and
/// height (u32) and file path (a string, with flag 16).
enum PackedRows {
    struct Truncated: Error {}

    static func read(_ data: Data) throws -> [RowView] {
        try data.withUnsafeBytes { bytes in
            var at = 0
            func take(_ n: Int) throws -> UnsafeRawBufferPointer {
                guard n >= 0, at + n <= bytes.count else { throw Truncated() }
                defer { at += n }
                return UnsafeRawBufferPointer(rebasing: bytes[at..<at + n])
            }
            func u32() throws -> UInt32 { UInt32(littleEndian: try take(4).loadUnaligned(as: UInt32.self)) }
            func string() throws -> String { String(decoding: try take(Int(try u32())), as: UTF8.self) }
            let count = Int(try u32())
            var rows: [RowView] = []
            rows.reserveCapacity(count)
            for _ in 0..<count {
                let depth = UInt16(littleEndian: try take(2).loadUnaligned(as: UInt16.self))
                let flags = try take(1)[0]
                let id = try string(), text = try string(), note = try string(), kind = try string()
                var image: ImageView?
                if flags & 8 != 0 {
                    let width = try u32(), height = try u32()
                    image = ImageView(path: flags & 16 != 0 ? try string() : nil, width: width, height: height)
                }
                rows.append(RowView(
                    id: id, depth: depth, text: text, note: note, kind: kind,
                    completed: flags & 1 != 0, collapsed: flags & 2 != 0, hasChildren: flags & 4 != 0,
                    image: image
                ))
            }
            guard at == bytes.count else { throw Truncated() }
            return rows
        }
    }
}
