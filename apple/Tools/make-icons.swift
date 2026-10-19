// Draws every raster Yap icon from one picture, the pixel speech bubble
// (Pixelarticons' "message-text", MIT; README › Credits) in white on the teal desktop:
//   - the iPhone + Mac app icons (App Store + Dev; the iPhone's dark and tinted looks too)
//     and the in-app YapIcon in Yap/Assets.xcassets;
//   - the web app's icon-192.png, icon-512.png and apple-touch-icon.png.
// web/icon.svg is the same picture as a vector; the desktop app draws it itself
// (desktop/src/ui/mod.rs › app_icon).
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

/// The bubble on the glyph's 24-unit grid (y down, like the SVG); the drawing spans 2...22.
let bubbleRects: [(x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat)] = [
    (2, 2, 20, 2),   // top
    (2, 4, 2, 18),   // left, down into the tail
    (20, 4, 2, 14),  // right
    (6, 16, 14, 2),  // bottom
    (4, 18, 2, 2),   // the tail's step
    (6, 7, 12, 2),   // text lines
    (6, 11, 8, 2),
]

/// The three iPhone looks: the background's top and bottom, and the bubble.
enum Look {
    case light, dark, tinted
    var colors: (top: UInt32, bottom: UInt32, bubble: UInt32) {
        switch self {
        case .light: (0x0b8f8f, 0x006b6b, 0xffffff)
        case .dark: (0x2e3130, 0x0e0f0f, 0x0b8f8f)
        case .tinted: (0x3a3d3c, 0x151716, 0xd9dcdb)
        }
    }
}

/// Fills the current clip with the look's top-to-bottom gradient over `rect` (pixels, y up).
func fillBackground(_ ctx: CGContext, _ look: Look, _ rect: CGRect) {
    let g = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color(look.colors.top), color(look.colors.bottom)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(g, start: CGPoint(x: rect.midX, y: rect.maxY), end: CGPoint(x: rect.midX, y: rect.minY), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
}

/// Draws the bubble at 62% of `tile`'s width, centered on `center` (pixels, y down).
/// Wherever a grid cell gets 2.5 px or more it's rounded to whole pixels, so the edges
/// stay hard; smaller (the Mac's Finder sizes) it's smoothed instead of turning to mush.
/// 62% keeps the bubble inside a maskable web icon's safe circle (80% across).
func drawBubble(_ ctx: CGContext, tile: CGFloat, center: CGPoint, canvas: CGFloat, color c: CGColor) {
    let ideal = 0.62 * tile / 24
    let crisp = ideal >= 2.5
    let unit = crisp ? ideal.rounded() : ideal
    var ox = center.x - 12 * unit, oy = center.y - 12 * unit
    if crisp { ox.round(); oy.round() }
    ctx.setShouldAntialias(!crisp)
    ctx.setFillColor(c)
    for r in bubbleRects {
        // CG is y-up: flip the grid's y.
        ctx.fill(CGRect(x: ox + r.x * unit, y: canvas - (oy + (r.y + r.h) * unit), width: r.w * unit, height: r.h * unit))
    }
}

func draw(size: Int, dev: Bool, mac: Bool, look: Look = .light) -> Data {
    let s = CGFloat(size)
    // iOS and web icons must be opaque (App Store Connect rejects an alpha channel);
    // the Mac's keep theirs (the tile floats on a transparent margin).
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: (mac ? CGImageAlphaInfo.premultipliedLast : CGImageAlphaInfo.noneSkipLast).rawValue)!
    // The Dev band covers the bottom 200/1024; the bubble moves up to stay clear of it.
    let band = dev ? 200 / 1024 * s : 0
    if mac {
        // macOS: a rounded teal tile with the standard margin.
        let tile = CGRect(x: 100 / 1024 * s, y: 100 / 1024 * s, width: 824 / 1024 * s, height: 824 / 1024 * s)
        ctx.saveGState()
        ctx.setShouldAntialias(true)
        ctx.addPath(CGPath(roundedRect: tile, cornerWidth: 185 / 1024 * s, cornerHeight: 185 / 1024 * s, transform: nil))
        ctx.clip()
        fillBackground(ctx, look, tile)
        ctx.restoreGState()
        drawBubble(ctx, tile: tile.width, center: CGPoint(x: s / 2, y: (tile.minY + min(tile.maxY, s - band)) / 2), canvas: s, color: color(look.colors.bubble))
    } else {
        fillBackground(ctx, look, CGRect(x: 0, y: 0, width: s, height: s))
        drawBubble(ctx, tile: s, center: CGPoint(x: s / 2, y: (s - band) / 2), canvas: s, color: color(look.colors.bubble))
    }

    if dev {
        ctx.setShouldAntialias(false)
        let rect = CGRect(x: 0, y: 0, width: s, height: 190 / 1024 * s)
        ctx.setFillColor(color(0xffffe1))
        ctx.fill(rect)
        ctx.setFillColor(color(0x000000))
        ctx.fill(CGRect(x: 0, y: rect.maxY, width: s, height: 10 / 1024 * s))
        ctx.setShouldAntialias(true)
        text(ctx, "DEV", size: 140 / 1024 * s, color: color(0x000000), at: CGPoint(x: s / 2, y: rect.midY), center: true)
    }

    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

for dev in [false, true] {
    let name = dev ? "AppIcon-Dev" : "AppIcon"
    let dir = assets.appending(path: "\(name).appiconset")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    var images: [[String: Any]] = []
    try draw(size: 1024, dev: dev, mac: false).write(to: dir.appending(path: "ios-1024.png"))
    images.append(["filename": "ios-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"])
    // The iPhone's dark and tinted Home Screens (the App Store build only).
    if !dev {
        for look in [Look.dark, .tinted] {
            let file = "ios-1024-\(look).png"
            try draw(size: 1024, dev: false, mac: false, look: look).write(to: dir.appending(path: file))
            images.append(["appearances": [["appearance": "luminosity", "value": "\(look)"]], "filename": file, "idiom": "universal", "platform": "ios", "size": "1024x1024"])
        }
    }
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
try draw(size: 192, dev: false, mac: false).write(to: web.appending(path: "icon-192.png"))
try draw(size: 512, dev: false, mac: false).write(to: web.appending(path: "icon-512.png"))
try draw(size: 180, dev: false, mac: false).write(to: web.appending(path: "apple-touch-icon.png"))
print("wrote web icons")
