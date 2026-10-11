import Foundation
import ImageIO
import Observation
import SwiftUI
#if os(iOS)
import UIKit
typealias PlatformImage = UIImage
#else
import AppKit
typealias PlatformImage = NSImage
#endif

extension Image {
    init(platformImage: PlatformImage) {
        #if os(iOS)
        self.init(uiImage: platformImage)
        #else
        self.init(nsImage: platformImage)
        #endif
    }
}

/// Custom emoji and avatars from the Yap server (`/emoji/<id>`, `/media/<id>`).
/// Both are content-addressed and immutable, so they're cached for the
/// process and also land in URLCache. Views read `image(for:)`; the first
/// read starts a load and the view updates when it arrives.
@MainActor
@Observable
final class ImageStore {
    private(set) var images: [URL: PlatformImage] = [:]
    /// Message pictures, downsampled (see `picture(for:maxPixel:)`).
    private(set) var pictures: [String: PlatformImage] = [:]
    @ObservationIgnored private var loading: Set<URL> = []
    @ObservationIgnored private var loadingPictures: Set<String> = []
    @ObservationIgnored private var sized: [String: PlatformImage] = [:]

    func image(for url: URL) -> PlatformImage? {
        if let image = images[url] { return image }
        load(url)
        return nil
    }

    /// For inline use in `Text`: scaled to `side` points tall.
    func inlineImage(for url: URL, side: CGFloat) -> PlatformImage? {
        guard let image = image(for: url) else { return nil }
        let key = "\(url.absoluteString)@\(side)"
        if let hit = sized[key] { return hit }
        let scaled = image.scaled(toHeight: side)
        sized[key] = scaled
        return scaled
    }

    func loadPicture(_ url: URL, key: String, maxPixel: Int) {
        guard !loadingPictures.contains(key) else { return }
        loadingPictures.insert(key)
        Task {
            defer { loadingPictures.remove(key) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400 else { return }
            let image = await Task.detached(priority: .userInitiated) { Self.downsample(data, maxPixel: maxPixel) }.value
            if let image {
                #if os(iOS)
                pictures[key] = UIImage(cgImage: image)
                #else
                pictures[key] = NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
                #endif
            }
        }
    }

    func loadSealedPicture(_ url: URL, key: String, maxPixel: Int, open: @escaping @MainActor (Data) throws -> Data) {
        guard !loadingPictures.contains(key) else { return }
        loadingPictures.insert(key)
        Task {
            defer { loadingPictures.remove(key) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400,
                  let plain = try? open(data) else { return }
            let image = await Task.detached(priority: .userInitiated) { Self.downsample(plain, maxPixel: maxPixel) }.value
            if let image {
                #if os(iOS)
                pictures[key] = UIImage(cgImage: image)
                #else
                pictures[key] = NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
                #endif
            }
        }
    }

    nonisolated static func downsample(_ data: Data, maxPixel: Int) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, 0, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
        ] as CFDictionary)
    }

    private func load(_ url: URL) {
        guard !loading.contains(url) else { return }
        loading.insert(url)
        Task {
            defer { loading.remove(url) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400,
                  let image = PlatformImage(data: data) else { return }
            images[url] = image
        }
    }
}

extension PlatformImage {
    func scaled(toHeight height: CGFloat) -> PlatformImage {
        let ratio = size.height > 0 ? size.width / size.height : 1
        let target = CGSize(width: max(1, (height * ratio).rounded()), height: height)
        #if os(iOS)
        return UIGraphicsImageRenderer(size: target).image { _ in draw(in: CGRect(origin: .zero, size: target)) }
        #else
        let image = NSImage(size: target)
        image.lockFocus()
        draw(in: CGRect(origin: .zero, size: target), from: .zero, operation: .copy, fraction: 1)
        image.unlockFocus()
        return image
        #endif
    }
}
