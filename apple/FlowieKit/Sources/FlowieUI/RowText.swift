#if canImport(UIKit)
import UIKit
import FlowieFFI

// Rendering rows that aren't being edited (wiki: iOS-Cross-Platform §2.3): markup as attributed
// text, image thumbnails, and the sheets a row opens (its note, its picture).

/// The colors markup shows in, as the desktop's theme has them (`flowie-app/src/ui/theme.rs`),
/// light and dark.
enum MarkupColors {
    static func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> UIColor {
        UIColor(
            red: CGFloat((hex >> 16) & 0xff) / 255,
            green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255,
            alpha: alpha
        )
    }

    static func dynamic(_ light: UIColor, _ dark: UIColor) -> UIColor {
        UIColor { $0.userInterfaceStyle == .dark ? dark : light }
    }

    static func dynamic(_ light: UInt32, _ dark: UInt32) -> UIColor {
        dynamic(rgb(light), rgb(dark))
    }

    /// `{red}…{/}` and the others, in `markup`'s order.
    static let text: [UIColor] = [
        dynamic(0xff3b30, 0xff453a),
        dynamic(0xdc6f00, 0xff9f0a),
        dynamic(0xc28f00, 0xffd60a),
        dynamic(0x248a3d, 0x30d158),
        dynamic(0x007aff, 0x0a84ff),
        dynamic(0xaf52de, 0xbf5af2),
        dynamic(0x8e8e93, 0x98989d),
    ]
    /// `==highlight==`.
    static let mark = dynamic(rgb(0xffdd33, 0.45), rgb(0xf2cc0d, 0.35))
    static let codeBackground = dynamic(0xf2f2f2, 0x2a2a2a)
    static let tag = UIColor.secondaryLabel
    static let tagBackground = dynamic(UIColor(white: 0, alpha: 0.06), UIColor(white: 1, alpha: 0.08))
    static let date = dynamic(0x0f766e, 0x5eead4)
    static let dateBackground = dynamic(0xe6f4f1, 0x16302d)
    static let link = UIColor.link
    static let internalLinkBackground = UIColor.link.withAlphaComponent(0.08)
}

extension RenderedText {
    /// The shown text with its markup's look, over `base` (the row's font, lines and color), as
    /// the desktop draws rows not being edited (`flowie-app/src/text/rich.rs`).
    func attributed(base: [NSAttributedString.Key: Any], font: UIFont, completed: Bool) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for piece in pieces {
            var attributes = base
            let run = piece.run
            var traits: UIFontDescriptor.SymbolicTraits = []
            if run.bold { traits.insert(.traitBold) }
            if run.italic { traits.insert(.traitItalic) }
            if !traits.isEmpty, let descriptor = font.fontDescriptor.withSymbolicTraits(traits) {
                attributes[.font] = UIFont(descriptor: descriptor, size: 0)
            }
            if let color = run.color, !completed {
                attributes[.foregroundColor] = MarkupColors.text[Int(color) % MarkupColors.text.count]
            }
            if run.highlight {
                attributes[.backgroundColor] = MarkupColors.mark
            }
            if run.strike || completed {
                attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            }
            let mono = UIFont.monospacedSystemFont(ofSize: font.pointSize * 0.94, weight: run.bold ? .semibold : .regular)
            switch run.kind {
            case .text, .marker:
                break
            case .code:
                attributes[.font] = mono
                attributes[.backgroundColor] = MarkupColors.codeBackground
            case .math:
                // Not rendered on iOS (§2.2): the source, tinted, in monospace.
                attributes[.font] = mono
                attributes[.foregroundColor] = UIColor.tintColor
                attributes[.backgroundColor] = MarkupColors.codeBackground
            case .link(let url, let bullet):
                attributes[.foregroundColor] = MarkupColors.link
                if bullet != nil {
                    attributes[.backgroundColor] = MarkupColors.internalLinkBackground
                } else if Self.webURL(url) != nil {
                    attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue
                    attributes[.underlineColor] = MarkupColors.link.withAlphaComponent(0.4)
                }
            case .tag, .mention:
                attributes[.foregroundColor] = MarkupColors.tag
                attributes[.backgroundColor] = MarkupColors.tagBackground
            case .date:
                attributes[.foregroundColor] = MarkupColors.date
                attributes[.backgroundColor] = MarkupColors.dateBackground
            }
            out.append(NSAttributedString(string: piece.text, attributes: attributes))
        }
        return out
    }
}

/// Image thumbnails, read and scaled off the main thread, kept while memory allows. A file not
/// there (synced without it) gives nothing, and is looked for again next time.
@MainActor
final class Thumbnails {
    static let shared = Thumbnails()
    private let cache = NSCache<NSString, UIImage>()
    private var waiting: [String: [(UIImage?) -> Void]] = [:]

    /// The thumbnail of the file at `path` for `size` points, if it is ready; otherwise nil, and
    /// `done` hears when it is.
    func image(at path: String, size: CGSize, scale: CGFloat, done: @escaping (UIImage?) -> Void) -> UIImage? {
        let pixels = CGSize(width: ceil(size.width * scale), height: ceil(size.height * scale))
        let key = "\(path)@\(Int(pixels.width))x\(Int(pixels.height))"
        if let hit = cache.object(forKey: key as NSString) { return hit }
        if waiting[key] != nil {
            waiting[key]?.append(done)
            return nil
        }
        waiting[key] = [done]
        DispatchQueue.global(qos: .userInitiated).async {
            let image = UIImage(contentsOfFile: path)?.preparingThumbnail(of: pixels)
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    let shared = Thumbnails.shared
                    if let image { shared.cache.setObject(image, forKey: key as NSString) }
                    let callbacks = shared.waiting.removeValue(forKey: key) ?? []
                    for callback in callbacks { callback(image) }
                }
            }
        }
        return nil
    }
}

/// An image row's picture, full screen: pinch to zoom, tap to close.
final class ImageViewer: UIViewController, UIScrollViewDelegate {
    private let path: String
    private let scroll = UIScrollView()
    private let imageView = UIImageView()

    init(path: String) {
        self.path = path
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .fullScreen
        modalTransitionStyle = .crossDissolve
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        scroll.frame = view.bounds
        scroll.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        scroll.delegate = self
        scroll.minimumZoomScale = 1
        scroll.maximumZoomScale = 4
        scroll.contentInsetAdjustmentBehavior = .never
        view.addSubview(scroll)
        imageView.frame = scroll.bounds
        imageView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        imageView.contentMode = .scaleAspectFit
        imageView.isAccessibilityElement = true
        imageView.accessibilityLabel = NSLocalizedString("Image", comment: "VoiceOver label of an image row's picture")
        scroll.addSubview(imageView)
        let close = UIButton(type: .close)
        close.overrideUserInterfaceStyle = .dark
        close.addAction(UIAction { [weak self] _ in self?.dismiss(animated: true) }, for: .touchUpInside)
        close.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(close)
        NSLayoutConstraint.activate([
            close.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            close.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
        ])
        view.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
        let path = path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let image = UIImage(contentsOfFile: path)?.preparingForDisplay()
            DispatchQueue.main.async { [weak self] in self?.imageView.image = image }
        }
    }

    @objc private func tapped() {
        dismiss(animated: true)
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? {
        imageView
    }
}

/// A bullet's note in a sheet (§2.1): read it, edit it. What changed is saved when the sheet
/// closes, as one undo step.
final class NoteSheet: UIViewController, UIAdaptivePresentationControllerDelegate {
    private let original: String
    private let save: (String) -> Void
    let textView = UITextView()
    private var saved = false

    init(note: String, title: String, save: @escaping (String) -> Void) {
        original = note
        self.save = save
        super.init(nibName: nil, bundle: nil)
        navigationItem.title = NSLocalizedString("Note", comment: "Menu item and title of the sheet with a bullet's note")
        navigationItem.subtitle = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not from a storyboard")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        textView.text = original
        textView.font = .preferredFont(forTextStyle: .body)
        textView.adjustsFontForContentSizeCategory = true
        textView.backgroundColor = .clear
        textView.textContainerInset = UIEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
        // Notes are stored as markup too: quotes and dashes stay as typed.
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        textView.alwaysBounceVertical = true
        textView.keyboardDismissMode = .interactive
        textView.frame = view.bounds
        textView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(textView)
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
            self?.finish()
            self?.dismiss(animated: true)
        })
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if original.isEmpty {
            textView.becomeFirstResponder()
        }
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish()
    }

    private func finish() {
        guard !saved else { return }
        saved = true
        textView.resignFirstResponder()
        if !textView.text.utf8.elementsEqual(original.utf8) {
            save(textView.text)
        }
    }
}
#endif
