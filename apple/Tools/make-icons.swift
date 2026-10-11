// Draws Yap's app icons (iOS full-bleed + macOS, App Store + Dev) into
// Yap/Assets.xcassets, in the Windows 98 / AIM look: a gray beveled window
// with a navy title bar and a white sunken well holding a speech bubble.
//   swift apple/Tools/make-icons.swift
import AppKit
import CoreText
import Foundation

let here = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let assets = here.appending(path: "../Yap/Assets.xcassets").standardized

func color(_ hex: UInt32) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
}

func font(_ size: CGFloat) -> CTFont {
    let tahoma = CTFontCreateWithName("Tahoma-Bold" as CFString, size, nil)
    return (CTFontCopyFamilyName(tahoma) as String) == "Tahoma" ? tahoma : CTFontCreateWithName("Helvetica-Bold" as CFString, size, nil)
}

/// Two-ring bevel inside `r`, `t` = ring thickness.
func bevel(_ ctx: CGContext, _ r: CGRect, _ t: CGFloat, raised: Bool) {
    let rings: [(CGColor, CGColor)] = raised
        ? [(color(0xffffff), color(0x000000)), (color(0xdfdfdf), color(0x808080))]
        : [(color(0x808080), color(0xffffff)), (color(0x000000), color(0xdfdfdf))]
    for (i, (tl, br)) in rings.enumerated() {
        let rr = r.insetBy(dx: CGFloat(i) * t, dy: CGFloat(i) * t)
        ctx.setFillColor(br)
        ctx.fill(CGRect(x: rr.minX, y: rr.minY, width: rr.width, height: t))     // bottom (CG is y-up)
        ctx.fill(CGRect(x: rr.maxX - t, y: rr.minY, width: t, height: rr.height)) // right
        ctx.setFillColor(tl)
        ctx.fill(CGRect(x: rr.minX, y: rr.maxY - t, width: rr.width - t, height: t)) // top
        ctx.fill(CGRect(x: rr.minX, y: rr.minY + t, width: t, height: rr.height - t)) // left
    }
}

func text(_ ctx: CGContext, _ string: String, size: CGFloat, color c: CGColor, at point: CGPoint, center: Bool = false) {
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: string, attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): font(size),
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): c,
    ]))
    let b = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
    ctx.textPosition = center ? CGPoint(x: point.x - b.width / 2 - b.minX, y: point.y - b.height / 2 - b.minY) : point
    CTLineDraw(line, ctx)
}

func draw(size: Int, dev: Bool, mac: Bool) -> Data {
    let s = CGFloat(size)
    // iOS icons must be opaque (App Store Connect rejects an alpha channel);
    // the Mac's keep theirs (the window floats on a transparent margin).
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: (mac ? CGImageAlphaInfo.premultipliedLast : CGImageAlphaInfo.noneSkipLast).rawValue)!
    ctx.scaleBy(x: s / 1024, y: s / 1024)
    ctx.setShouldAntialias(false)
    // iOS: the window fills the icon on a teal desktop; Mac: the window alone with a margin.
    let window = mac ? CGRect(x: 92, y: 120, width: 840, height: 784) : CGRect(x: 72, y: 96, width: 880, height: 832)
    if !mac {
        ctx.setFillColor(color(0x008080))
        ctx.fill(CGRect(x: 0, y: 0, width: 1024, height: 1024))
    }
    let t: CGFloat = 12
    ctx.setFillColor(color(0xc0c0c0))
    ctx.fill(window)
    bevel(ctx, window, t, raised: true)

    // Title bar: navy → light blue, "Yap", a close button.
    let bar = CGRect(x: window.minX + 3 * t, y: window.maxY - 3 * t - 150, width: window.width - 6 * t, height: 150)
    let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color(0x000080), color(0x1084d0)] as CFArray, locations: [0, 1])!
    ctx.saveGState()
    ctx.clip(to: bar)
    ctx.drawLinearGradient(gradient, start: CGPoint(x: bar.minX, y: 0), end: CGPoint(x: bar.maxX, y: 0), options: [])
    ctx.restoreGState()
    ctx.setShouldAntialias(true)
    text(ctx, "Yap", size: 104, color: color(0xffffff), at: CGPoint(x: bar.minX + 34, y: bar.minY + 34))
    ctx.setShouldAntialias(false)
    let close = CGRect(x: bar.maxX - 128, y: bar.minY + 22, width: 106, height: 106)
    ctx.setFillColor(color(0xc0c0c0))
    ctx.fill(close)
    bevel(ctx, close, 8, raised: true)
    ctx.setStrokeColor(color(0x000000))
    ctx.setLineWidth(16)
    ctx.move(to: CGPoint(x: close.minX + 30, y: close.minY + 30)); ctx.addLine(to: CGPoint(x: close.maxX - 30, y: close.maxY - 30))
    ctx.move(to: CGPoint(x: close.minX + 30, y: close.maxY - 30)); ctx.addLine(to: CGPoint(x: close.maxX - 30, y: close.minY + 30))
    ctx.strokePath()

    // The white sunken well with a chat bubble.
    let well = CGRect(x: window.minX + 3 * t + 8, y: window.minY + 3 * t + 8, width: window.width - 6 * t - 16, height: bar.minY - window.minY - 3 * t - 24)
    ctx.setFillColor(color(0xffffff))
    ctx.fill(well)
    bevel(ctx, well, t, raised: false)

    ctx.setShouldAntialias(true)
    let bubble = CGRect(x: well.minX + 92, y: well.minY + 150, width: well.width - 184, height: well.height - 260)
    let path = CGMutablePath()
    path.addRoundedRect(in: bubble, cornerWidth: 70, cornerHeight: 70)
    path.move(to: CGPoint(x: bubble.minX + 110, y: bubble.minY + 2))
    path.addLine(to: CGPoint(x: bubble.minX + 70, y: bubble.minY - 110))
    path.addLine(to: CGPoint(x: bubble.minX + 250, y: bubble.minY + 2))
    ctx.addPath(path)
    ctx.setFillColor(color(0xffffe1))
    ctx.fillPath()
    ctx.addPath(path)
    ctx.setStrokeColor(color(0x000000))
    ctx.setLineWidth(18)
    ctx.setLineJoin(.round)
    ctx.strokePath()
    // Cover the seam between bubble and tail.
    ctx.setFillColor(color(0xffffe1))
    ctx.fill(CGRect(x: bubble.minX + 118, y: bubble.minY - 6, width: 120, height: 18))
    // "..." typing dots in navy.
    for i in 0..<3 {
        let d: CGFloat = 70
        let x = bubble.midX - d * 1.5 - 40 + CGFloat(i) * (d + 40)
        ctx.setFillColor(color(0x000080))
        ctx.fill(CGRect(x: x, y: bubble.midY - d / 2, width: d, height: d))
    }

    if dev {
        let band = CGRect(x: 0, y: 0, width: 1024, height: 190)
        ctx.setFillColor(color(0xffffe1))
        ctx.fill(band)
        ctx.setFillColor(color(0x000000))
        ctx.fill(CGRect(x: 0, y: band.maxY, width: 1024, height: 10))
        text(ctx, "DEV", size: 140, color: color(0x000000), at: CGPoint(x: 512, y: band.midY), center: true)
    }

    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

for dev in [false, true] {
    let name = dev ? "AppIcon-Dev" : "AppIcon"
    let dir = assets.appending(path: "\(name).appiconset")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    var images: [[String: String]] = []
    try draw(size: 1024, dev: dev, mac: false).write(to: dir.appending(path: "ios-1024.png"))
    images.append(["filename": "ios-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"])
    for (points, scale) in [(16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let px = points * scale
        let file = "mac-\(points)@\(scale)x.png"
        try draw(size: px, dev: dev, mac: true).write(to: dir.appending(path: file))
        images.append(["filename": file, "idiom": "mac", "scale": "\(scale)x", "size": "\(points)x\(points)"])
    }
    let contents: [String: Any] = ["images": images, "info": ["author": "xcode", "version": 1]]
    try JSONSerialization.data(withJSONObject: contents, options: [.prettyPrinted, .sortedKeys]).write(to: dir.appending(path: "Contents.json"))
    print("wrote \(name)")
}
