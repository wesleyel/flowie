// The share extension's inbox (wiki: iOS-Cross-Platform §5): the extension never opens the
// outline, it leaves each share as a small JSON file in the App Group container, and the app
// adds it to the journal (`OutlineStore.importShares`). Foundation only, so the extension links
// no Rust.
import Foundation

/// One share: what the extension's sheet held when "Add to Today" was tapped.
public struct SharedItem: Codable, Equatable, Sendable {
    /// The share's own id, also its file's name: the outline records it (flowie-ffi
    /// `journal_share`), so the share goes in once however often its file is read.
    public var id: String
    /// The day it was shared on (YYYY-MM-DD, local time): the journal page it goes to.
    public var day: String
    /// The text, one bullet a line.
    public var text: String
    /// A web page shared with it, linked from the first line.
    public var url: String?
    public var created: Date

    public init(id: String = UUID().uuidString, day: String, text: String, url: String? = nil, created: Date = Date()) {
        self.id = id
        self.day = day
        self.text = text
        self.url = url
        self.created = created
    }

    /// `date` as the journal names its days (YYYY-MM-DD, in the device's time zone).
    public static func day(of date: Date) -> String {
        dayFormatter().string(from: date)
    }

    /// The journal page to add it to: `day`, or the day of `created` if `day` isn't a date.
    public var journalDay: String {
        let formatter = Self.dayFormatter()
        if let date = formatter.date(from: day), formatter.string(from: date) == day {
            return day
        }
        return formatter.string(from: created)
    }

    private static func dayFormatter() -> DateFormatter {
        let formatter = DateFormatter()
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .current
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter
    }
}

/// The folder of shares waiting for the app, one `<id>.json` file each.
public struct ShareInbox: Sendable {
    public static let appGroup = "group.flowie.spokey.cn"
    /// Posted (Darwin notification) after a share is written, so an app on screen beside the
    /// sharing app (iPad) takes it in at once.
    public static let notification = "flowie.spokey.cn.share"

    public let directory: URL

    public init(directory: URL) {
        self.directory = directory
    }

    /// The inbox in the App Group container, where the app and the extension both reach it.
    public static func appGroup(fileManager: FileManager = .default) -> ShareInbox? {
        fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
            .map { ShareInbox(directory: $0.appendingPathComponent("inbox", isDirectory: true)) }
    }

    /// Leave `item` for the app. Written whole to a temporary name, then renamed into place in
    /// one step: the app never reads half a share.
    public func add(_ item: SharedItem) throws {
        let fileManager = FileManager.default
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(item)
        let temporary = directory.appendingPathComponent(".\(item.id).tmp")
        try data.write(to: temporary)
        guard rename(temporary.path, file(item.id).path) == 0 else {
            let error = CocoaError(.fileWriteUnknown, userInfo: [NSUnderlyingErrorKey: POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)])
            try? fileManager.removeItem(at: temporary)
            throw error
        }
    }

    /// The shares waiting, oldest first. A temporary file (a share still being written, or one
    /// whose writer was ended) is left alone until it's a day old; a file that can't be read
    /// is set aside as `<name>.unreadable`, kept but not read again.
    public func pending(now: Date = Date()) -> [SharedItem] {
        let fileManager = FileManager.default
        guard let names = try? fileManager.contentsOfDirectory(atPath: directory.path) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        var items: [SharedItem] = []
        for name in names {
            let url = directory.appendingPathComponent(name)
            if name.hasPrefix("."), name.hasSuffix(".tmp") {
                let modified = (try? fileManager.attributesOfItem(atPath: url.path)[.modificationDate]) as? Date
                if let modified, now.timeIntervalSince(modified) > 24 * 60 * 60 {
                    try? fileManager.removeItem(at: url)
                }
                continue
            }
            guard name.hasSuffix(".json") else { continue }
            if let data = try? Data(contentsOf: url), let item = try? decoder.decode(SharedItem.self, from: data),
               file(item.id) == url {
                items.append(item)
            } else {
                try? fileManager.moveItem(at: url, to: url.appendingPathExtension("unreadable"))
            }
        }
        return items.sorted { ($0.created, $0.id) < ($1.created, $1.id) }
    }

    /// Delete the share's file, once the outline has it saved.
    public func remove(_ item: SharedItem) throws {
        do {
            try FileManager.default.removeItem(at: file(item.id))
        } catch CocoaError.fileNoSuchFile {
        }
    }

    private func file(_ id: String) -> URL {
        directory.appendingPathComponent("\(id).json")
    }
}
