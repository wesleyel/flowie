import CloudKit
import Foundation
import os

/// iCloud sync's log: a line for each image file (`Blob`) fetched or sent, and for each fetch
/// that failed, so a file that never shows up can be traced (Console, subsystem
/// `flowie.spokey.cn`, category `icloud`). Debug builds also keep the lines in a small file
/// beside CKSyncEngine's state (`sync-debug.log`, then `sync-debug.old.log`), which can be
/// copied off a device without Console (`xcrun devicectl device copy from`).
public enum CloudLog {
    static let logger = Logger(subsystem: "flowie.spokey.cn", category: "icloud")
    /// The file grows to this many bytes before it starts again (the old one kept).
    static let fileLimit: UInt64 = 256 << 10
    private static let lock = NSLock()

    /// Log `message` (public: it carries record names and sizes, never contents).
    public static func note(_ message: String, beside stateURL: URL? = nil) {
        logger.notice("\(message, privacy: .public)")
        #if DEBUG
        if let stateURL {
            append(message, to: stateURL.deletingLastPathComponent().appendingPathComponent("sync-debug.log"))
        }
        #endif
    }

    /// Log `message` at debug level, which Console shows only when asked (each fetch batch).
    public static func debug(_ message: String, beside stateURL: URL? = nil) {
        logger.debug("\(message, privacy: .public)")
        #if DEBUG
        if let stateURL {
            append(message, to: stateURL.deletingLastPathComponent().appendingPathComponent("sync-debug.log"))
        }
        #endif
    }

    /// What a fetched `Blob` record carries, for its log line: whether its asset has a file
    /// and how large it is. Read while CKSyncEngine hands the record over.
    static func describe(blob record: CKRecord) -> String {
        guard let asset = record[CloudRecordCodec.blobFile] as? CKAsset else { return "no asset" }
        guard let url = asset.fileURL else { return "asset without a file URL" }
        guard let size = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? NSNumber else {
            return "asset file missing (\(url.lastPathComponent))"
        }
        return "asset file \(size.int64Value) bytes"
    }

    #if DEBUG
    private static func append(_ message: String, to url: URL) {
        let line = "\(ISO8601DateFormatter().string(from: Date())) \(message)\n"
        lock.withLock {
            let manager = FileManager.default
            if let size = (try? manager.attributesOfItem(atPath: url.path))?[.size] as? NSNumber, size.uint64Value > fileLimit {
                let old = url.deletingLastPathComponent().appendingPathComponent("sync-debug.old.log")
                try? manager.removeItem(at: old)
                try? manager.moveItem(at: url, to: old)
            }
            if let handle = try? FileHandle(forWritingTo: url) {
                defer { try? handle.close() }
                _ = try? handle.seekToEnd()
                try? handle.write(contentsOf: Data(line.utf8))
            } else {
                try? Data(line.utf8).write(to: url)
            }
        }
    }
    #endif
}
