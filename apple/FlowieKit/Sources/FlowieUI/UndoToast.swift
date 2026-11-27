#if canImport(UIKit)
import UIKit

/// The bar at the bottom of the page after a bullet is deleted: "Deleted · Undo" for a few
/// seconds, above the keyboard bar or the tab bar (wiki: iOS TODO). Its Undo is Flowie's undo.
/// VoiceOver announces it, and its button stays reachable for longer.
final class UndoToast: UIView {
    var onUndo: (() -> Void)?
    private let label = UILabel()
    private let button = UIButton(type: .system)
    private var hideWork: DispatchWorkItem?

    /// How long it stays: longer with VoiceOver, to swipe to its button.
    private var duration: TimeInterval { UIAccessibility.isVoiceOverRunning ? 10 : 4 }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let blur = UIVisualEffectView(effect: UIBlurEffect(style: .systemChromeMaterial))
        blur.translatesAutoresizingMaskIntoConstraints = false
        blur.layer.cornerRadius = 22
        blur.layer.cornerCurve = .continuous
        blur.clipsToBounds = true
        addSubview(blur)
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.15
        layer.shadowRadius = 12
        layer.shadowOffset = CGSize(width: 0, height: 4)

        label.text = NSLocalizedString("Deleted", comment: "The bar shown after a bullet is deleted, before its Undo button")
        label.font = .preferredFont(forTextStyle: .subheadline)
        label.adjustsFontForContentSizeCategory = true
        label.textColor = .label
        let dot = UILabel()
        dot.text = "·"
        dot.font = label.font
        dot.textColor = .secondaryLabel
        dot.isAccessibilityElement = false
        var config = UIButton.Configuration.plain()
        config.title = NSLocalizedString("Undo", comment: "Keyboard bar button; hold to redo")
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attributes in
            var attributes = attributes
            attributes.font = UIFontMetrics(forTextStyle: .subheadline).scaledFont(for: .systemFont(ofSize: 15, weight: .semibold))
            return attributes
        }
        config.contentInsets = NSDirectionalEdgeInsets(top: 8, leading: 6, bottom: 8, trailing: 6)
        button.configuration = config
        button.addTarget(self, action: #selector(undo), for: .touchUpInside)
        button.accessibilityHint = NSLocalizedString("Brings back the deleted bullet", comment: "VoiceOver hint of the Undo button after a delete")

        let stack = UIStackView(arrangedSubviews: [label, dot, button])
        stack.axis = .horizontal
        stack.alignment = .center
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        blur.contentView.addSubview(stack)
        NSLayoutConstraint.activate([
            blur.topAnchor.constraint(equalTo: topAnchor),
            blur.bottomAnchor.constraint(equalTo: bottomAnchor),
            blur.leadingAnchor.constraint(equalTo: leadingAnchor),
            blur.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: blur.contentView.topAnchor, constant: 2),
            stack.bottomAnchor.constraint(equalTo: blur.contentView.bottomAnchor, constant: -2),
            stack.leadingAnchor.constraint(equalTo: blur.contentView.leadingAnchor, constant: 18),
            stack.trailingAnchor.constraint(equalTo: blur.contentView.trailingAnchor, constant: -10),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        alpha = 0
        isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    /// Show it (again, if it is up: the timer starts over).
    func show() {
        hideWork?.cancel()
        isHidden = false
        transform = alpha == 0 ? CGAffineTransform(translationX: 0, y: 12) : .identity
        UIView.animate(withDuration: 0.25, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
            self.alpha = 1
            self.transform = .identity
        }
        UIAccessibility.post(notification: .announcement, argument: NSLocalizedString("Deleted", comment: "The bar shown after a bullet is deleted, before its Undo button"))
        let work = DispatchWorkItem { [weak self] in self?.hide() }
        hideWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + duration, execute: work)
    }

    func hide() {
        hideWork?.cancel()
        hideWork = nil
        guard !isHidden else { return }
        UIView.animate(withDuration: 0.2, delay: 0, options: [.beginFromCurrentState]) {
            self.alpha = 0
        } completion: { _ in
            if self.alpha == 0 { self.isHidden = true }
        }
    }

    @objc private func undo() {
        hide()
        onUndo?()
    }
}
#endif
