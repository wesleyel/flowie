// Draws the Flowie app icon at all macOS icon sizes into an .iconset.
// usage: swift scripts/make-icon.swift <out.iconset>
import AppKit

let out = CommandLine.arguments[1]
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

func draw(_ px: Int) -> Data {
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
    let colors = [NSColor(red: 0.27, green: 0.51, blue: 0.98, alpha: 1).cgColor,
                  NSColor(red: 0.42, green: 0.33, blue: 0.93, alpha: 1).cgColor] as CFArray
    let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: rect.minX, y: rect.maxY), end: CGPoint(x: rect.maxX, y: rect.minY), options: [])
    // Three outline rows: bullet + bar, indented like an outline.
    let w = rect.width
    let rows: [(CGFloat, CGFloat, CGFloat)] = [(0.0, 0.70, 1.0), (0.13, 0.50, 0.85), (0.13, 0.30, 0.85)]
    for (i, (indent, yFrac, alpha)) in rows.enumerated() {
        let y = rect.minY + w * yFrac
        let x = rect.minX + w * (0.2 + indent)
        let r = w * 0.045
        ctx.setFillColor(NSColor.white.withAlphaComponent(alpha).cgColor)
        ctx.fillEllipse(in: CGRect(x: x - r, y: y - r, width: 2 * r, height: 2 * r))
        let barW = w * (i == 0 ? 0.5 : 0.38)
        let bar = CGRect(x: x + w * 0.09, y: y - w * 0.03, width: barW, height: w * 0.06)
        ctx.addPath(CGPath(roundedRect: bar, cornerWidth: w * 0.03, cornerHeight: w * 0.03, transform: nil))
        ctx.fillPath()
    }
    // Guide line for the children.
    ctx.setFillColor(NSColor.white.withAlphaComponent(0.45).cgColor)
    ctx.fill(CGRect(x: rect.minX + w * 0.2 - w * 0.008, y: rect.minY + w * 0.27, width: w * 0.016, height: w * 0.36))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        try! draw(base * scale).write(to: URL(fileURLWithPath: "\(out)/\(name)"))
    }
}
