// The desktop's way into FlowieCloudKit (crates/flowie-cloudkit): a few C functions over
// CloudController, with the outline on the Rust side. Each question CloudKit sync asks the
// outline goes to Rust as a JSON message with a reply token; Rust answers on the thread that
// owns the outline (`flowie_ck_reply`), which resumes the waiting task. Messages without a
// token are news (account, progress). Compiled only by flowie-cloudkit's build.rs, together
// with apple/FlowieKit/Sources/FlowieCloudKit.
import AppKit
import CloudKit
import Foundation

/// `(context, message JSON, reply token or nil)`.
public typealias FlowieCKCallback = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void

/// Resumes the task waiting for an answer from Rust.
final class Reply: @unchecked Sendable {
    let resume: (String) -> Void

    init(_ resume: @escaping (String) -> Void) {
        self.resume = resume
    }
}

/// The outline on the Rust side, as CloudKit sync's host.
final class RustHost: CloudSyncHost, @unchecked Sendable {
    let context: UnsafeMutableRawPointer?
    let callback: FlowieCKCallback

    init(context: UnsafeMutableRawPointer?, callback: FlowieCKCallback) {
        self.context = context
        self.callback = callback
    }

    private func encode(_ message: [String: Any]) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: message)) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }

    func post(_ message: [String: Any]) {
        encode(message).withCString { callback(context, $0, nil) }
    }

    func ask(_ message: [String: Any]) async -> Any? {
        let text = encode(message)
        let answer = await withCheckedContinuation { (continuation: CheckedContinuation<String, Never>) in
            let reply = Unmanaged.passRetained(Reply { continuation.resume(returning: $0) }).toOpaque()
            text.withCString { callback(context, $0, reply) }
        }
        return try? JSONSerialization.jsonObject(with: Data(answer.utf8), options: [.fragmentsAllowed])
    }

    static func wire(_ wire: CloudWire) -> [String: Any] {
        var object: [String: Any] = ["id": wire.id, "envelope": wire.envelope]
        if let etag = wire.etag {
            object["etag"] = etag
        }
        if let file = wire.file {
            object["file"] = ["path": file.path, "width": file.width, "height": file.height, "mime": file.mime]
        }
        return object
    }

    static func wire(from object: [String: Any]) -> CloudWire? {
        guard let id = object["id"] as? String else { return nil }
        let file = (object["file"] as? [String: Any]).flatMap { file -> CloudFile? in
            guard let path = file["path"] as? String else { return nil }
            let size = { (key: String) in (file[key] as? NSNumber)?.uint32Value ?? 0 }
            return CloudFile(path: path, width: size("width"), height: size("height"), mime: file["mime"] as? String ?? "")
        }
        guard let envelope = object["envelope"] as? String ?? (file == nil ? nil : "") else { return nil }
        return CloudWire(id: id, etag: object["etag"] as? String, envelope: envelope, file: file)
    }

    func unsentIDs() async -> [String] {
        (await ask(["type": "unsent"]) as? [String]) ?? []
    }

    func nextSaves(limit: Int) async -> [CloudWire] {
        let answer = await ask(["type": "next_saves", "limit": limit]) as? [[String: Any]] ?? []
        return answer.compactMap(RustHost.wire(from:))
    }

    func onSaved(_ outcomes: [CloudSaved]) async {
        let list: [[String: Any]] = outcomes.map { outcome in
            switch outcome {
            case .saved(let id, let etag): return ["result": "saved", "id": id, "etag": etag]
            case .conflict(let server): return ["result": "conflict", "server": RustHost.wire(server)]
            case .failed(let id, let error): return ["result": "failed", "id": id, "error": error]
            }
        }
        _ = await ask(["type": "saved", "outcomes": list])
    }

    func onFetched(_ records: [CloudWire]) async {
        _ = await ask(["type": "fetched", "records": records.map(RustHost.wire)])
    }

    func resendAll() async {
        _ = await ask(["type": "resend_all"])
    }

    func cloudEvent(_ event: CloudEvent) async {
        switch event {
        case .access(let access):
            var message: [String: Any] = ["type": "access"]
            switch access {
            case .available: message["access"] = "available"
            case .noAccount: message["access"] = "no_account"
            case .restricted: message["access"] = "restricted"
            case .temporarilyUnavailable: message["access"] = "temporarily_unavailable"
            case .notEntitled: message["access"] = "not_entitled"
            case .unsupported: message["access"] = "unsupported"
            case .failed(let error):
                message["access"] = "failed"
                message["error"] = error
            }
            post(message)
        case .fetched: post(["type": "fetched_all"])
        case .sent: post(["type": "sent"])
        case .failed(let error): post(["type": "failed", "error": error])
        }
    }
}

/// What `flowie_ck_start` hands Rust: the controller (kept untyped for macOS 13).
final class Handle {
    let controller: AnyObject

    init(_ controller: AnyObject) {
        self.controller = controller
    }

    @available(macOS 14.0, *)
    var cloud: CloudController? {
        controller as? CloudController
    }
}

private func handle(_ pointer: UnsafeMutableRawPointer?) -> Handle? {
    pointer.map { Unmanaged<Handle>.fromOpaque($0).takeUnretainedValue() }
}

/// Whether this build is signed with the iCloud entitlements for Flowie's container. Without
/// them, nothing may touch CloudKit (it ends the process).
@_cdecl("flowie_ck_entitled")
public func flowie_ck_entitled() -> Bool {
    CloudAccess.entitled()
}

/// Start iCloud sync: checks the account, runs CKSyncEngine while one is signed in, with its
/// state in the file at `statePath`. Returns the handle for the calls below, or nil where
/// CKSyncEngine isn't there (macOS 13; the callback hears why).
@_cdecl("flowie_ck_start")
public func flowie_ck_start(_ statePath: UnsafePointer<CChar>, _ fresh: Bool, _ hold: Bool, _ context: UnsafeMutableRawPointer?,
                            _ callback: FlowieCKCallback) -> UnsafeMutableRawPointer? {
    let host = RustHost(context: context, callback: callback)
    guard #available(macOS 14.0, *) else {
        host.post(["type": "access", "access": "unsupported"])
        return nil
    }
    let url = URL(fileURLWithPath: String(cString: statePath))
    let controller = CloudController(host: host, stateURL: url, fresh: fresh, holdSends: hold)
    controller.start()
    // CKSyncEngine's subscription sends silent pushes; the app has the aps-environment
    // entitlement for them in the Mac App Store build.
    DispatchQueue.main.async { NSApplication.shared.registerForRemoteNotifications() }
    return Unmanaged.passRetained(Handle(controller)).toOpaque()
}

/// These ids have something to send (JSON array of strings).
@_cdecl("flowie_ck_queue")
public func flowie_ck_queue(_ pointer: UnsafeMutableRawPointer?, _ json: UnsafePointer<CChar>) {
    guard #available(macOS 14.0, *), let cloud = handle(pointer)?.cloud else { return }
    let ids = (try? JSONSerialization.jsonObject(with: Data(String(cString: json).utf8))) as? [String] ?? []
    cloud.queue(ids)
}

/// Send from now on (after a first sync's fetch).
@_cdecl("flowie_ck_release")
public func flowie_ck_release(_ pointer: UnsafeMutableRawPointer?) {
    guard #available(macOS 14.0, *), let cloud = handle(pointer)?.cloud else { return }
    cloud.releaseSends()
}

/// Fetch what changed in iCloud now.
@_cdecl("flowie_ck_fetch")
public func flowie_ck_fetch(_ pointer: UnsafeMutableRawPointer?) {
    guard #available(macOS 14.0, *), let cloud = handle(pointer)?.cloud else { return }
    Task { await cloud.fetchNow() }
}

/// Stop syncing and let go of the handle.
@_cdecl("flowie_ck_stop")
public func flowie_ck_stop(_ pointer: UnsafeMutableRawPointer?) {
    guard let pointer else { return }
    let handle = Unmanaged<Handle>.fromOpaque(pointer)
    if #available(macOS 14.0, *) {
        handle.takeUnretainedValue().cloud?.stop()
    }
    handle.release()
}

/// Rust's answer (JSON) to a message that came with `token`.
@_cdecl("flowie_ck_reply")
public func flowie_ck_reply(_ token: UnsafeMutableRawPointer?, _ json: UnsafePointer<CChar>) {
    guard let token else { return }
    let reply = Unmanaged<Reply>.fromOpaque(token).takeRetainedValue()
    reply.resume(String(cString: json))
}
