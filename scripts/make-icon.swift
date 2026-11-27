// Draws the Flowie app icon: three outline rows (bullet + bar, indented like an outline) on a
// blue-to-violet gradient.
// usage: swift scripts/make-icon.swift <out.iconset>            all macOS icon sizes
//        swift scripts/make-icon.swift --ios <AppIcon.appiconset> the iOS app icon: one 1024 px
//            square each for the light, dark and tinted appearances, plus Contents.json
import AppKit
import ImageIO
import UniformTypeIdentifiers

let gradientColors = [NSColor(red: 0.27, green: 0.51, blue: 0.98, alpha: 1).cgColor,
                      NSColor(red: 0.42, green: 0.33, blue: 0.93, alpha: 1).cgColor] as CFArray

/// The rows and the children's guide line inside `rect`, each filled by `fill(path, alpha)`.
func drawRows(in rect: CGRect, fill: (CGPath, CGFloat) -> Void) {
    let w = rect.width
    let rows: [(CGFloat, CGFloat, CGFloat)] = [(0.0, 0.70, 1.0), (0.13, 0.50, 0.85), (0.13, 0.30, 0.85)]
    for (i, (indent, yFrac, alpha)) in rows.enumerated() {
        let y = rect.minY + w * yFrac
        let x = rect.minX + w * (0.2 + indent)
        let r = w * 0.045
        fill(CGPath(ellipseIn: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r), transform: nil), alpha)
        let barW = w * (i == 0 ? 0.5 : 0.38)
        let bar = CGRect(x: x + w * 0.09, y: y - w * 0.03, width: barW, height: w * 0.06)
        fill(CGPath(roundedRect: bar, cornerWidth: w * 0.03, cornerHeight: w * 0.03, transform: nil), alpha)
    }
    // Guide line for the children.
    fill(CGPath(rect: CGRect(x: rect.minX + w * 0.2 - w * 0.008, y: rect.minY + w * 0.27, width: w * 0.016, height: w * 0.36),
                transform: nil), 0.45)
}

func gradient(_ ctx: CGContext, _ rect: CGRect) {
    let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: gradientColors, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: rect.minX, y: rect.maxY), end: CGPoint(x: rect.maxX, y: rect.minY), options: [])
}

func png(_ image: CGImage) -> Data {
    let data = NSMutableData()
    let dest = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, image, nil)
    CGImageDestinationFinalize(dest)
    return data as Data
}

// MARK: macOS

/// A macOS icon: the rounded square with the standard margin and a shadow.
func drawMac(_ px: Int) -> Data {
    let s = CGFloat(px)
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    // Squircle-ish background with the standard macOS margin.
    let inset = s * 0.1
    let rect = CGRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let path = CGPath(roundedRect: rect, cornerWidth: rect.width * 0.225, cornerHeight: rect.width * 0.225, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -s * 0.012), blur: s * 0.03, color: NSColor.black.withAlphaComponent(0.25).cgColor)
    ctx.addPath(path)
    ctx.setFillColor(NSColor.white.cgColor)
    ctx.fillPath()
    ctx.restoreGState()
    ctx.addPath(path)
    ctx.clip()
    gradient(ctx, rect)
    drawRows(in: rect) { path, alpha in
        ctx.setFillColor(NSColor.white.withAlphaComponent(alpha).cgColor)
        ctx.addPath(path)
        ctx.fillPath()
    }
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

// MARK: iOS

enum Appearance: String, CaseIterable { case light, dark, tinted }

/// An iOS icon: a full-bleed square (iOS rounds the corners). Light is opaque, as the App Store
/// requires; dark and tinted are glyphs on a transparent background that iOS puts on its own
/// dark background or tints.
func drawIOS(_ appearance: Appearance, _ px: Int = 1024) -> Data {
    let s = CGFloat(px)
    let rect = CGRect(x: 0, y: 0, width: s, height: s)
    let alpha: CGImageAlphaInfo = appearance == .light ? .noneSkipLast : .premultipliedLast
    let ctx = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: alpha.rawValue)!
    switch appearance {
    case .light:
        gradient(ctx, rect)
        drawRows(in: rect) { path, alpha in
            ctx.setFillColor(NSColor.white.withAlphaComponent(alpha).cgColor)
            ctx.addPath(path)
            ctx.fillPath()
        }
    case .dark:
        // The glyph carries the gradient instead of the background.
        drawRows(in: rect) { path, alpha in
            ctx.saveGState()
            ctx.setAlpha(alpha)
            ctx.addPath(path)
            ctx.clip()
            gradient(ctx, rect)
            ctx.restoreGState()
        }
    case .tinted:
        // Grayscale: iOS tints it, white taking the full tint.
        drawRows(in: rect) { path, alpha in
            ctx.setFillColor(NSColor.white.withAlphaComponent(alpha).cgColor)
            ctx.addPath(path)
            ctx.fillPath()
        }
    }
    return png(ctx.makeImage()!)
}

let args = CommandLine.arguments
if args.count == 3 && args[1] == "--ios" {
    let out = args[2]
    try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
    var images: [[String: Any]] = []
    for appearance in Appearance.allCases {
        let name = appearance == .light ? "AppIcon.png" : "AppIcon-\(appearance.rawValue).png"
        try! drawIOS(appearance).write(to: URL(fileURLWithPath: "\(out)/\(name)"))
        var image: [String: Any] = ["filename": name, "idiom": "universal", "platform": "ios", "size": "1024x1024"]
        if appearance != .light {
            image["appearances"] = [["appearance": "luminosity", "value": appearance.rawValue]]
        }
        images.append(image)
    }
    let contents: [String: Any] = ["images": images, "info": ["author": "xcode", "version": 1]]
    let json = try! JSONSerialization.data(withJSONObject: contents, options: [.prettyPrinted, .sortedKeys])
    try! json.write(to: URL(fileURLWithPath: "\(out)/Contents.json"))
} else if args.count == 2 {
    let out = args[1]
    try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
    for base in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
            try! drawMac(base * scale).write(to: URL(fileURLWithPath: "\(out)/\(name)"))
        }
    }
} else {
    FileHandle.standardError.write("usage: make-icon.swift <out.iconset> | --ios <AppIcon.appiconset>\n".data(using: .utf8)!)
    exit(2)
}
