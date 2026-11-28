// Flowie's share extension: a small sheet with what was shared, editable, that leaves it in the
// inbox for the app to add to today's journal (wiki: iOS-Cross-Platform §5). It never opens the
// outline and links no Rust: only the app writes the database. Text and web pages only; iOS v1
// can't insert images (§2.2), so the activation rule (Project.swift) offers no images.
import FlowieInbox
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The extension's principal class (`NSExtensionPrincipalClass`), hosting the SwiftUI sheet.
final class ShareViewController: UIViewController {
    private let model = ShareModel()

    override func viewDidLoad() {
        super.viewDidLoad()
        model.finish = { [weak self] in self?.extensionContext?.completeRequest(returningItems: nil) }
        model.cancel = { [weak self] in
            self?.extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
        }
        let host = UIHostingController(rootView: ShareSheet(model: model))
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        Task { await model.load(items) }
    }
}

/// What the sheet shows and does.
@MainActor
final class ShareModel: ObservableObject {
    @Published var text = ""
    @Published var url: URL?
    @Published var loading = true
    @Published var failed = false
    var finish: () -> Void = {}
    var cancel: () -> Void = {}

    var canAdd: Bool {
        !loading && (url != nil || !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    /// The text field starts with the page's title, or the text shared; a web page shared is
    /// linked from the first line.
    func load(_ items: [NSExtensionItem]) async {
        var title: String?
        var texts: [String] = []
        for item in items {
            if title == nil, let name = (item.attributedTitle ?? item.attributedContentText)?.string, !name.isEmpty {
                title = name
            }
            for provider in item.attachments ?? [] {
                if url == nil, provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
                   let shared = try? await provider.loadItem(forTypeIdentifier: UTType.url.identifier) as? URL,
                   !shared.isFileURL {
                    url = shared
                } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier),
                          let shared = try? await provider.loadItem(forTypeIdentifier: UTType.plainText.identifier) {
                    if let string = shared as? String {
                        texts.append(string)
                    } else if let data = shared as? Data, let string = String(data: data, encoding: .utf8) {
                        texts.append(string)
                    }
                }
            }
        }
        // The title is often the text shared too.
        var lines = texts.filter { $0 != title && $0 != url?.absoluteString }
        if let title { lines.insert(title, at: 0) }
        text = lines.joined(separator: "\n")
        loading = false
    }

    func add() {
        let item = SharedItem(day: SharedItem.day(of: Date()), text: text, url: url?.absoluteString)
        do {
            guard let inbox = ShareInbox.appGroup() else { throw CocoaError(.fileNoSuchFile) }
            try inbox.add(item)
        } catch {
            failed = true
            return
        }
        // An app on screen beside the sharing app (iPad) takes the share in at once.
        CFNotificationCenterPostNotification(CFNotificationCenterGetDarwinNotifyCenter(),
                                             CFNotificationName(ShareInbox.notification as CFString), nil, nil, true)
        finish()
    }
}

struct ShareSheet: View {
    @ObservedObject var model: ShareModel
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            Form {
                if model.loading {
                    ProgressView()
                        .frame(maxWidth: .infinity)
                } else {
                    TextField(NSLocalizedString("Text", comment: "Placeholder of the share sheet's text field"),
                              text: $model.text, axis: .vertical)
                        .lineLimit(3...12)
                        .focused($focused)
                        .onAppear { focused = true }
                    if let url = model.url {
                        Label {
                            Text(verbatim: url.absoluteString)
                                .lineLimit(2)
                                .truncationMode(.middle)
                        } icon: {
                            Image(systemName: "link")
                        }
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle(Text(verbatim: "Flowie"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(NSLocalizedString("Cancel", comment: "Closes the share sheet without adding anything"), action: model.cancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(NSLocalizedString("Add to Today", comment: "Share sheet: adds what was shared to today's journal"), action: model.add)
                        .disabled(!model.canAdd)
                }
            }
            .alert(NSLocalizedString("Flowie couldn’t save this. Try again.", comment: "The share couldn't be written for the app"),
                   isPresented: $model.failed) {
                Button(NSLocalizedString("OK", comment: "Dismisses an alert")) {}
            }
        }
    }
}
