import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Gets a picture ready for `POST /api/image` (≤ 1.9 MB of PNG, JPEG, WebP
/// or GIF), like the web's `preparePicture`:
/// - **Compressed** (the default): redrawn upright at ≤ 1600 px as JPEG
///   (~0.78), stepping down to 1280 / 1024 px until it fits — a phone photo
///   lands around 150–300 KB. An original that's already smaller (and no
///   bigger in pixels) goes as it is.
/// - **Full res**: the original when it's a PNG/JPEG/WebP that fits and is
///   upright; otherwise the sharpest JPEG that fits (full size at 0.92, then
///   3072 / 2048 / 1600 px). HEIC always becomes JPEG (browsers can't show it).
/// - A GIF that fits stays as it is (animated) either way.
public enum PictureEncoder {
    public struct Prepared: Sendable, Equatable {
        public var data: Data
        public var mime: String
        public var width: Int
        public var height: Int
    }

    public enum Failure: Error, LocalizedError, Equatable {
        case unreadable
        case tooBig

        public var errorDescription: String? {
            switch self {
            case .unreadable: "That file isn't a picture Yap can read."
            case .tooBig: "That picture is too big."
            }
        }
    }

    /// The compressed picture's long side.
    public static let maxSide = 1600
    static let compressedSteps: [(side: Int?, quality: Double)] = [(1600, 0.78), (1280, 0.72), (1024, 0.66)]
    /// nil = the picture's own size.
    static let fullSteps: [(side: Int?, quality: Double)] = [(nil, 0.92), (3072, 0.9), (2048, 0.88), (1600, 0.82)]
    static let passthrough: [String: String] = [
        UTType.png.identifier: "image/png",
        UTType.jpeg.identifier: "image/jpeg",
        UTType.gif.identifier: "image/gif",
        "org.webmproject.webp": "image/webp",
    ]

    public static func prepare(_ data: Data, full: Bool = false, maxBytes: Int = ServerAPI.maxImageBytes) throws -> Prepared {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let type = CGImageSourceGetType(source) as String?,
              CGImageSourceGetCount(source) > 0,
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any] else { throw Failure.unreadable }
        let width = props[kCGImagePropertyPixelWidth] as? Int ?? 0
        let height = props[kCGImagePropertyPixelHeight] as? Int ?? 0
        let orientation = props[kCGImagePropertyOrientation] as? Int ?? 1
        guard width > 0, height > 0 else { throw Failure.unreadable }
        let longSide = max(width, height)

        // The bytes as they are, when the server and every client can show them.
        var original: Prepared?
        if let mime = passthrough[type], data.count <= maxBytes, orientation == 1 || mime == "image/gif" {
            original = Prepared(data: data, mime: mime, width: width, height: height)
        }
        if let original, original.mime == "image/gif" || full { return original }

        for step in full ? fullSteps : compressedSteps {
            let side = min(step.side ?? longSide, longSide)
            guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: side,
            ] as CFDictionary) else { throw Failure.unreadable }
            guard let jpeg = jpegData(opaque(image), quality: step.quality) else { throw Failure.unreadable }
            guard jpeg.count <= maxBytes else { continue }
            // Already small: don't re-encode into something bigger.
            if let original, original.data.count <= jpeg.count, longSide <= side { return original }
            return Prepared(data: jpeg, mime: "image/jpeg", width: image.width, height: image.height)
        }
        throw Failure.tooBig
    }

    /// "47 KB", "1.2 MB" (like the web's tray).
    public static func sizeLabel(_ bytes: Int) -> String {
        bytes >= 1_000_000
            ? String(format: "%.1f MB", Double(bytes) / 1_000_000)
            : "\(max(1, Int((Double(bytes) / 1000).rounded()))) KB"
    }

    /// JPEG has no alpha: put transparent pictures on white first.
    static func opaque(_ image: CGImage) -> CGImage {
        switch image.alphaInfo {
        case .none, .noneSkipLast, .noneSkipFirst: return image
        default: break
        }
        guard let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return image }
        let rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
        context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
        context.fill(rect)
        context.draw(image, in: rect)
        return context.makeImage() ?? image
    }

    static func jpegData(_ image: CGImage, quality: Double) -> Data? {
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, image, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return out as Data
    }
}
