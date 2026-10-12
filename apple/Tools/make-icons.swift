// Draws every raster Yap icon from one picture, the pixel speech bubble with
// sound bars (web/icon.svg) on the teal desktop:
//   - the iPhone + Mac app icons (App Store + Dev) and the in-app YapIcon in Yap/Assets.xcassets;
//   - the web app's icon-192.png, icon-512.png and apple-touch-icon.png.
// The desktop app draws the same pixels itself (desktop/src/ui/mod.rs › app_icon).
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

func text(_ ctx: CGContext, _ string: String, size: CGFloat, color c: CGColor, at point: CGPoint, center: Bool = false) {
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: string, attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): font(size),
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): c,
    ]))
    let b = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
    ctx.textPosition = center ? CGPoint(x: point.x - b.width / 2 - b.minX, y: point.y - b.height / 2 - b.minY) : point
    CTLineDraw(line, ctx)
}

/// Yap's icon: the pixel speech bubble with sound bars from `web/icon.svg`, on the teal desktop.
/// Drawn on that file's 32-unit grid (y down, like the SVG), so every copy is the same picture.
let bubbleRects: [(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat, color: UInt32, alpha: CGFloat)] = [
    (4, 6, 28, 19, 0x000000, 0.35),  // drop shadow
    (2, 4, 28, 19, 0x000000, 1),     // outline
    (4, 6, 24, 15, 0xffffff, 1),     // inside
    (4, 6, 24, 2, 0xdfdfdf, 1),      // top shade
    (8, 23, 8, 2, 0x000000, 1),      // tail
    (8, 25, 5, 2, 0x000000, 1),
    (8, 27, 2, 2, 0x000000, 1),
    (10, 21, 4, 2, 0xffffff, 1),
    (10, 23, 2, 2, 0xffffff, 1),
    (7, 12, 2, 3, 0x000080, 1),      // sound bars
    (11, 10, 2, 7, 0x000080, 1),
    (15, 8, 2, 11, 0x000080, 1),
    (19, 10, 2, 7, 0x000080, 1),
    (23, 12, 2, 3, 0x000080, 1),
]

/// Fills `rect` (in 1024 space, y down) with the bubble, its 30x25-unit drawing centered.
func drawBubble(_ ctx: CGContext, in rect: CGRect) {
    let unit = min(rect.width / 30, rect.height / 25)
    let ox = rect.midX - 17 * unit, oy = rect.midY - 16.5 * unit
    for r in bubbleRects {
        ctx.setFillColor(color(r.color).copy(alpha: r.alpha)!)
        // CG is y-up: flip the SVG's y.
        ctx.fill(CGRect(x: ox + r.x * unit, y: 1024 - (oy + (r.y + r.h) * unit), width: r.w * unit, height: r.h * unit))
    }
}

/// `web`: the bubble a little smaller, so a circular "maskable" crop never cuts it.
func draw(size: Int, dev: Bool, mac: Bool, web: Bool = false) -> Data {
    let s = CGFloat(size)
    // iOS and web icons must be opaque (App Store Connect rejects an alpha channel);
    // the Mac's keep theirs (the tile floats on a transparent margin).
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: (mac ? CGImageAlphaInfo.premultipliedLast : CGImageAlphaInfo.noneSkipLast).rawValue)!
    ctx.scaleBy(x: s / 1024, y: s / 1024)
    ctx.setShouldAntialias(false)
    if mac {
        // macOS: a rounded teal tile with the standard margin.
        let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
        ctx.setShouldAntialias(true)
        ctx.addPath(CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil))
        ctx.setFillColor(color(0x008080))
        ctx.fillPath()
        // Hard pixel edges blur into a blob at Finder sizes (16-64 px); smooth those.
        ctx.setShouldAntialias(size < 128)
        drawBubble(ctx, in: CGRect(x: 222, y: 252, width: 580, height: 520))
    } else {
        ctx.setFillColor(color(0x008080))
        ctx.fill(CGRect(x: 0, y: 0, width: 1024, height: 1024))
        drawBubble(ctx, in: web ? CGRect(x: 212, y: 262, width: 600, height: 500) : CGRect(x: 152, y: 212, width: 720, height: 600))
    }

    if dev {
        let band = CGRect(x: 0, y: 0, width: 1024, height: 190)
        ctx.setFillColor(color(0xffffe1))
        ctx.fill(band)
        ctx.setFillColor(color(0x000000))
        ctx.fill(CGRect(x: 0, y: band.maxY, width: 1024, height: 10))
        ctx.setShouldAntialias(true)
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

// The in-app icon (DebugLaunch) is the iOS icon.
try draw(size: 1024, dev: false, mac: false).write(to: assets.appending(path: "YapIcon.imageset/yap-icon.png"))
print("wrote YapIcon")

// The web app (repo root is three levels up from Yap/Assets.xcassets).
let web = assets.appending(path: "../../../web").standardized
try draw(size: 192, dev: false, mac: false, web: true).write(to: web.appending(path: "icon-192.png"))
try draw(size: 512, dev: false, mac: false, web: true).write(to: web.appending(path: "icon-512.png"))
try draw(size: 180, dev: false, mac: false).write(to: web.appending(path: "apple-touch-icon.png"))
print("wrote web icons")
