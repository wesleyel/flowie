#if canImport(UIKit)
import SwiftUI
import UIKit

/// The horizontal swipe that goes back and forward through a window's pages (wiki: Sliding-Panes
/// §2): one finger anywhere on the pages, or two on a trackpad (its scroll, as Safari's). It
/// starts only for a mostly horizontal drag (twice as far across as down), and leaves the text
/// alone: not on a row's selection or its handles, nor while text is being composed (pinyin).
/// Lists and the page scroll vertically only once it has failed.
struct PageSwipe: UIGestureRecognizerRepresentable {
    /// How far it has gone across, as it goes.
    let changed: (CGFloat) -> Void
    /// Let go: how far across, and how fast.
    let ended: (CGFloat, CGFloat) -> Void

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let recognizer = UIPanGestureRecognizer()
        recognizer.allowedScrollTypesMask = .continuous
        recognizer.maximumNumberOfTouches = 1
        recognizer.delegate = context.coordinator
        return recognizer
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIPanGestureRecognizer, context: Context) {
        let dx = recognizer.translation(in: recognizer.view).x
        switch recognizer.state {
        case .changed:
            changed(dx)
        case .ended:
            ended(dx, recognizer.velocity(in: recognizer.view).x)
        case .cancelled, .failed:
            ended(0, 0)
        default:
            break
        }
    }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
            let velocity = pan.velocity(in: pan.view)
            let moved = pan.translation(in: pan.view)
            let across = abs(velocity.x) > 0 ? velocity : moved
            guard abs(across.x) > 2 * abs(across.y) else { return false }
            // Not while a composition is open anywhere (its candidates are chosen with swipes).
            if let editor = UIResponder.first as? UITextView, editor.markedTextRange != nil {
                return false
            }
            return true
        }

        /// A touch on a row's text being selected (its selection or handles) is the text's.
        func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            var view = touch.view
            while let current = view {
                if let text = current as? UITextView, text.isFirstResponder {
                    return text.markedTextRange == nil && text.selectedRange.length == 0
                }
                view = current.superview
            }
            return true
        }

        /// The page's and the lists' vertical scrolling wait for the swipe to fail, so a sideways
        /// drag doesn't scroll them.
        func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
            other.view is UIScrollView && other is UIPanGestureRecognizer
        }

        func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            false
        }
    }
}

/// Two-finger spread on a pane in two-pane mode widens that pane to fill the window; a
/// two-finger pinch in single-pane mode brings the two panes back. Works on a trackpad too.
struct PagePinch: UIGestureRecognizerRepresentable {
    let canExpand: () -> Bool
    let canCollapse: () -> Bool
    /// `(expand, leftHalf)`: true to fill the window with the pane on that half, false to show
    /// two panes again.
    let trigger: (Bool, Bool) -> Void

    func makeUIGestureRecognizer(context: Context) -> UIPinchGestureRecognizer {
        let recognizer = UIPinchGestureRecognizer()
        recognizer.delegate = context.coordinator
        return recognizer
    }

    func updateUIGestureRecognizer(_ recognizer: UIPinchGestureRecognizer, context: Context) {
        context.coordinator.canExpand = canExpand
        context.coordinator.canCollapse = canCollapse
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIPinchGestureRecognizer, context: Context) {
        let coordinator = context.coordinator
        switch recognizer.state {
        case .began:
            coordinator.fired = false
            if let view = recognizer.view, view.bounds.width > 0 {
                coordinator.leftHalf = recognizer.location(in: view).x < view.bounds.width / 2
            } else {
                coordinator.leftHalf = false
            }
        case .changed:
            guard !coordinator.fired else { return }
            if recognizer.scale >= 1.25, canExpand() {
                coordinator.fired = true
                trigger(true, coordinator.leftHalf)
            } else if recognizer.scale <= 0.78, canCollapse() {
                coordinator.fired = true
                trigger(false, coordinator.leftHalf)
            }
        case .ended:
            guard !coordinator.fired else { return }
            let v = recognizer.velocity
            if (recognizer.scale >= 1.12 || (recognizer.scale > 1.04 && v > 0.6)), canExpand() {
                coordinator.fired = true
                trigger(true, coordinator.leftHalf)
            } else if (recognizer.scale <= 0.88 || (recognizer.scale < 0.96 && v < -0.6)), canCollapse() {
                coordinator.fired = true
                trigger(false, coordinator.leftHalf)
            }
        default:
            break
        }
    }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator {
        Coordinator(canExpand: canExpand, canCollapse: canCollapse)
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var canExpand: () -> Bool
        var canCollapse: () -> Bool
        var fired = false
        var leftHalf = false

        init(canExpand: @escaping () -> Bool, canCollapse: @escaping () -> Bool) {
            self.canExpand = canExpand
            self.canCollapse = canCollapse
        }

        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard canExpand() || canCollapse() else { return false }
            if let editor = UIResponder.first as? UITextView, editor.markedTextRange != nil {
                return false
            }
            return true
        }

        func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
            other.view is UIScrollView && other is UIPanGestureRecognizer
        }
    }
}

extension UIResponder {
    @MainActor private static weak var found: UIResponder?

    /// The first responder now (the text view being typed in), if any.
    @MainActor static var first: UIResponder? {
        found = nil
        UIApplication.shared.sendAction(#selector(markFirst), to: nil, from: nil, for: nil)
        return found
    }

    @objc private func markFirst() {
        UIResponder.found = self
    }
}

/// How a page dragged past the end of the history follows the finger less and less, as a scroll
/// view's rubber band.
func rubberBand(_ offset: CGFloat, width: CGFloat) -> CGFloat {
    guard width > 0 else { return 0 }
    let pulled = (1 - 1 / (abs(offset) * 0.55 / width + 1)) * width
    return offset < 0 ? -pulled : pulled
}
#endif
