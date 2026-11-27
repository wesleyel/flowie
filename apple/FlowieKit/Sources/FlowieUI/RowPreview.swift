#if canImport(UIKit)
import UIKit
import FlowieFFI

/// The long-press menu's preview of a bullet (§4.4): its text and the first rows under it,
/// indented as on its page, read-only. Tapping it opens the bullet (the menu's Zoom In).
final class RowPreviewController: UIViewController {
    /// How many rows under the bullet it shows.
    static let limit = 12

    private let row: RowView
    private let below: [RowView]
    /// Whether more rows than `limit` are under it.
    private let more: Bool
    private let width: CGFloat

    /// `page`: the bullet's own page; `width`: the outline's.
    init(row: RowView, page: [RowView], width: CGFloat) {
        self.row = row
        self.width = min(width - 48, 380)
        below = Array(page.prefix(Self.limit))
        more = page.count > Self.limit
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let stack = UIStackView()
        stack.axis = .vertical
        stack.alignment = .fill
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -16),
        ])

        let title = UILabel()
        title.numberOfLines = 3
        title.attributedText = text(row, font: .preferredFont(forTextStyle: .headline))
        stack.addArrangedSubview(title)
        if !below.isEmpty {
            stack.setCustomSpacing(10, after: title)
        }
        let body = UIFont.preferredFont(forTextStyle: .subheadline)
        for child in below {
            let label = UILabel()
            label.numberOfLines = 2
            let line = NSMutableAttributedString(string: "•  ", attributes: [.font: body, .foregroundColor: UIColor.tertiaryLabel])
            line.append(text(child, font: body))
            let paragraph = NSMutableParagraphStyle()
            paragraph.firstLineHeadIndent = CGFloat(min(Int(child.depth), 6)) * 16
            paragraph.headIndent = paragraph.firstLineHeadIndent + ("•  " as NSString).size(withAttributes: [.font: body]).width
            line.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: line.length))
            label.attributedText = line
            stack.addArrangedSubview(label)
        }
        if more {
            let label = UILabel()
            label.text = "…"
            label.font = body
            label.textColor = .tertiaryLabel
            stack.addArrangedSubview(label)
        }
        // Read as a whole: VoiceOver users get the rows on the page itself.
        view.isAccessibilityElement = true
        view.accessibilityLabel = ([title.attributedText?.string ?? ""] + below.map { RenderedText($0.text).string }).joined(separator: ", ")

        let fitted = view.systemLayoutSizeFitting(
            CGSize(width: width, height: UIView.layoutFittingCompressedSize.height),
            withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel
        )
        preferredContentSize = CGSize(width: width, height: ceil(fitted.height))
    }

    private func text(_ row: RowView, font: UIFont) -> NSAttributedString {
        let color: UIColor = row.completed ? .secondaryLabel : .label
        let code = row.kind == "code"
        return RenderedText(row.text, code: code).attributed(base: [.font: font, .foregroundColor: color], font: font, completed: row.completed)
    }
}
#endif
