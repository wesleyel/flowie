import Foundation

/// Where the iOS app keeps its outline: in the App Group container, so extensions added later
/// (share, widgets) reach it without a move (wiki: iOS-Cross-Platform §5). Without a container
/// (the group isn't in the app's entitlements), in Application Support as before.
public enum OutlineLocation {
    public static let appGroup = "group.flowie.spokey.cn"
    /// The database's file name, in its own folder with everything SQLite and Flowie keep
    /// beside it (`-wal`, `-shm`, `.images`, backups taken before upgrades).
    public static let fileName = "flowie.sqlite"

    /// The database to open. One still in the old place (Application Support) moves into the
    /// container first, if the container has none.
    public static func database(fileManager: FileManager = .default) -> URL {
        let old = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Flowie", isDirectory: true)
        guard let group = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroup) else {
            return old.appendingPathComponent(fileName)
        }
        let new = group.appendingPathComponent("Library/Application Support/Flowie", isDirectory: true)
        return adopt(from: old, to: new, fileManager: fileManager)
    }

    /// The database in folder `new`, after moving folder `old` there when `old` holds a
    /// database and `new` doesn't. The folder moves whole, in one rename, so the database never
    /// gets separated from its `-wal` (which can hold its latest edits) or its images. Nothing
    /// is deleted: a `new` folder without a database is renamed aside first, and when the move
    /// fails the database is opened where it is.
    @discardableResult
    public static func adopt(from old: URL, to new: URL, fileManager: FileManager = .default) -> URL {
        let oldDB = old.appendingPathComponent(fileName)
        let newDB = new.appendingPathComponent(fileName)
        guard fileManager.fileExists(atPath: oldDB.path), !fileManager.fileExists(atPath: newDB.path) else {
            return newDB
        }
        do {
            try fileManager.createDirectory(at: new.deletingLastPathComponent(), withIntermediateDirectories: true)
            if fileManager.fileExists(atPath: new.path) {
                let stamp = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
                let aside = new.deletingLastPathComponent().appendingPathComponent("\(new.lastPathComponent) (before move \(stamp))")
                try fileManager.moveItem(at: new, to: aside)
            }
            try fileManager.moveItem(at: old, to: new)
            return newDB
        } catch {
            return fileManager.fileExists(atPath: oldDB.path) ? oldDB : newDB
        }
    }
}
