#if canImport(UIKit)
import UIKit
import FlowieFFI

/// What points at a page, listed read-only under its rows as the desktop's footer lists it:
/// "Linked from N", the bullets whose text links to the page (`flowie:` links), then
/// "Mirrored in N", its mirrors. Each entry shows its text rendered as rows show it and the path
/// to it; tapping one opens its page (`PageModel.backlinks`, `Outline.backlinks`). Not a row: it
/// never takes the caret, and the page lays it out below the rows, outside the list.
final class BacklinksView: UIView {
    /// Open an entry's page.
    var onOpen: ((BacklinkView) -> Void)?
    private let stack = UIStackView()
    private(set) var entries: [BacklinkView] = []
    private var metrics: RowMetrics?

    override init(frame: CGRect) {
        super.init(frame: frame)
        stack.axis = .vertical
        stack.alignment = .fill
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    /// Show these entries (none: the view is hidden).
    func update(_ entries: [BacklinkView], metrics: RowMetrics) {
        guard entries != self.entries || metrics != self.metrics else { return }
        self.entries = entries
        self.metrics = metrics
        isHidden = entries.isEmpty
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        let links = entries.filter { !$0.mirror }
        let mirrors = entries.filter(\.mirror)
        for (mirror, group) in [(false, links), (true, mirrors)] where !group.isEmpty {
            let header = UILabel()
            header.font = UIFontMetrics(forTextStyle: .footnote).scaledFont(for: .systemFont(ofSize: 13, weight: .semibold))
            header.adjustsFontForContentSizeCategory = true
            header.textColor = .secondaryLabel
            header.accessibilityTraits = .header
            header.text = mirror
                ? String.localizedStringWithFormat(NSLocalizedString("Mirrored in %lld", comment: "Heading under a page's rows over the mirrors of the page (N places)"), group.count)
                : String.localizedStringWithFormat(NSLocalizedString("Linked from %lld", comment: "Heading under a page's rows over the bullets that link to the page (N places)"), group.count)
            let wrap = UIView()
            header.translatesAutoresizingMaskIntoConstraints = false
            wrap.addSubview(header)
            NSLayoutConstraint.activate([
                header.topAnchor.constraint(equalTo: wrap.topAnchor, constant: stack.arrangedSubviews.isEmpty ? 0 : 16),
                header.leadingAnchor.constraint(equalTo: wrap.leadingAnchor),
                header.trailingAnchor.constraint(equalTo: wrap.trailingAnchor),
                header.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -4),
            ])
            stack.addArrangedSubview(wrap)
            for entry in group {
                let row = BacklinkEntry(entry, metrics: metrics)
                row.addAction(UIAction { [weak self] _ in self?.onOpen?(entry) }, for: .touchUpInside)
                stack.addArrangedSubview(row)
            }
        }
    }

    /// How tall it is at `width`.
    func height(for width: CGFloat) -> CGFloat {
        guard !entries.isEmpty else { return 0 }
        return systemLayoutSizeFitting(CGSize(width: width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel).height
    }
}

/// One entry: an icon, the bullet's text rendered (three lines at most), the path to it.
private final class BacklinkEntry: UIControl {
    init(_ entry: BacklinkView, metrics: RowMetrics) {
        super.init(frame: .zero)
        let icon = UIImageView(image: UIImage(systemName: entry.mirror ? "square.on.square" : "link",
                                              withConfiguration: UIImage.SymbolConfiguration(font: metrics.noteFont)))
        icon.tintColor = .tertiaryLabel
        icon.contentMode = .center
        let text = UILabel()
        text.numberOfLines = 3
        text.lineBreakMode = .byTruncatingTail
        let untitled = entry.text.trimmingCharacters(in: .whitespaces).isEmpty
        if untitled {
            text.text = NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text")
            text.font = metrics.font
            text.textColor = .tertiaryLabel
        } else {
            text.attributedText = RenderedText(entry.text).attributed(base: metrics.textAttributes(completed: entry.completed), font: metrics.font, completed: entry.completed)
        }
        text.alpha = entry.completed ? 0.45 : 1
        let path = UILabel()
        path.font = metrics.noteFont
        path.textColor = .secondaryLabel
        path.numberOfLines = 1
        path.lineBreakMode = .byTruncatingMiddle
        path.text = BulletView.line(entry.path.map { $0.text.isEmpty ? NSLocalizedString("Untitled", comment: "Title of a page whose bullet has no text") : $0.text }, limit: 60)
        for view in [icon, text, path] {
            view.translatesAutoresizingMaskIntoConstraints = false
            view.isUserInteractionEnabled = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor),
            icon.widthAnchor.constraint(equalToConstant: metrics.slot),
            icon.centerYAnchor.constraint(equalTo: topAnchor, constant: 6 + metrics.lineHeight / 2),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 4),
            text.trailingAnchor.constraint(equalTo: trailingAnchor),
            path.topAnchor.constraint(equalTo: text.bottomAnchor, constant: 1),
            path.leadingAnchor.constraint(equalTo: text.leadingAnchor),
            path.trailingAnchor.constraint(equalTo: trailingAnchor),
            path.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityLabel = untitled ? text.text : text.attributedText?.string
        accessibilityValue = path.text
        accessibilityHint = NSLocalizedString("Opens its page", comment: "VoiceOver hint of an entry under a page's rows that links to the page")
        addInteraction(UIPointerInteraction())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override var isHighlighted: Bool {
        didSet { backgroundColor = isHighlighted ? .systemFill : .clear }
    }
}
#endif
