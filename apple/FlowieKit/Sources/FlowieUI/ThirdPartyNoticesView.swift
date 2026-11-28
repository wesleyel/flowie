import SwiftUI

/// Settings → About → Open-Source Notices: the licenses of the open-source components in the
/// app, which their licenses (MIT, Apache-2.0, BSD, MPL-2.0…) ask to ship with the binary.
/// `scripts/package-ios.sh` generates `ThirdPartyNotices.txt` with cargo-about before archiving;
/// builds made without it (Xcode runs, screenshots) show a note instead.
struct ThirdPartyNoticesView: View {
    /// The file split at its `====` rules, one block per license, so a long list stays lazy.
    private let sections: [String]? = {
        guard let url = Bundle.main.url(forResource: "ThirdPartyNotices", withExtension: "txt"),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
        let rule = "\n" + String(repeating: "=", count: 80) + "\n"
        return text.components(separatedBy: rule)
            .map { $0.trimmingCharacters(in: .newlines) }
            .filter { !$0.isEmpty }
    }()

    var body: some View {
        Group {
            if let sections {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 24) {
                        ForEach(sections.indices, id: \.self) { i in
                            Text(verbatim: sections[i])
                                .font(.caption.monospaced())
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .padding()
                }
            } else {
                ContentUnavailableView(
                    NSLocalizedString("Open-Source Notices", comment: "Settings row"),
                    systemImage: "doc.text",
                    description: Text("The licenses of Flowie’s open-source components are included in App Store builds.")
                )
            }
        }
        .navigationTitle(NSLocalizedString("Open-Source Notices", comment: "Settings row"))
        .navigationBarTitleDisplayMode(.inline)
    }
}
