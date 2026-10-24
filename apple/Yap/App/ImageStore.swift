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
///
/// Message pictures are decoded small (`picture(for:maxPixel:)`, ~2.4 MB
/// each at 900 px) and only the most recently shown stay, up to
/// `pictureBudget`: a long session through a photo-heavy channel used to
/// keep every one it ever showed. One that comes back on screen decodes
/// again. Every picture view on screen redraws when one arrives (they all
/// read `pictures`), which marks it shown, so the trim waits a moment for
/// that and never lets go of what's on screen. iPhone: a memory warning
/// lets go of all but what was shown in the last few seconds, and a call
/// going to the background (where Yap keeps running) of all but the last
/// few. Something that didn't load isn't asked for again for 30 s (every
/// redraw used to ask again).
@MainActor
@Observable
final class ImageStore {
    private(set) var images: [URL: PlatformImage] = [:]
    /// Message pictures, downsampled (see `picture(for:maxPixel:)`).
    private(set) var pictures: [String: PlatformImage] = [:]
    @ObservationIgnored private var loading: Set<URL> = []
    @ObservationIgnored private var loadingPictures: Set<String> = []
    @ObservationIgnored private var sized: [String: PlatformImage] = [:]
    /// Each kept picture's decoded size and when it was last shown.
    @ObservationIgnored private var pictureUse: [String: (bytes: Int, at: TimeInterval)] = [:]
    /// What failed to load, and when (a URL, or a picture's key).
    @ObservationIgnored private var failedAt: [String: TimeInterval] = [:]
    @ObservationIgnored private var trimTask: Task<Void, Never>?

    #if os(iOS)
    static let pictureBudget = 80 << 20
    #else
    static let pictureBudget = 256 << 20
    #endif
    /// In the background: about the last six pictures, so coming back is instant.
    static let backgroundBudget = 16 << 20
    /// Shown this recently = on screen: never let go of (a picture let go of
    /// while on screen would load again, and push out another).
    static let onScreenFor: TimeInterval = 2
    static let retryAfter: TimeInterval = 30

    /// Seconds, monotonic (a hand-cranked clock in tests).
    @ObservationIgnored var clock: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
    private var now: TimeInterval { clock() }

    init() {
        #if os(iOS)
        NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.memoryWarning() }
        }
        #endif
    }

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

    /// A kept picture (it counts as just shown).
    func cachedPicture(_ key: String) -> PlatformImage? {
        guard let picture = pictures[key] else { return nil }
        pictureUse[key]?.at = now
        return picture
    }

    func loadPicture(_ url: URL, key: String, maxPixel: Int) {
        guard !loadingPictures.contains(key), mayTry(key) else { return }
        loadingPictures.insert(key)
        Task {
            defer { loadingPictures.remove(key) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400 else { return failed(key) }
            let image = await Task.detached(priority: .userInitiated) { Self.downsample(data, maxPixel: maxPixel) }.value
            guard let image else { return failed(key) }
            keep(image, key: key)
        }
    }

    func loadSealedPicture(_ url: URL, key: String, maxPixel: Int, open: @escaping @MainActor (Data) throws -> Data) {
        guard !loadingPictures.contains(key), mayTry(key) else { return }
        loadingPictures.insert(key)
        Task {
            defer { loadingPictures.remove(key) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400,
                  let plain = try? open(data) else { return failed(key) }
            let image = await Task.detached(priority: .userInitiated) { Self.downsample(plain, maxPixel: maxPixel) }.value
            guard let image else { return failed(key) }
            keep(image, key: key)
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

    /// Let go of the pictures shown longest ago until they fit in `budget`
    /// bytes (never one shown in the last `onScreenFor` seconds).
    func trimPictures(to budget: Int, sparing onScreenFor: TimeInterval = ImageStore.onScreenFor) {
        var total = pictureUse.values.reduce(0) { $0 + $1.bytes }
        guard total > budget else { return }
        let cutoff = now - onScreenFor
        for (key, use) in pictureUse.sorted(by: { $0.value.at < $1.value.at }) {
            guard total > budget else { break }
            guard use.at < cutoff else { continue }
            pictureUse.removeValue(forKey: key)
            pictures.removeValue(forKey: key)
            total -= use.bytes
        }
    }

    /// Keeps pictures shown within the last 5 s; everything else can go.
    private func memoryWarning() {
        trimPictures(to: 0, sparing: 5)
        sized.removeAll()
    }

    func keep(_ image: CGImage, key: String) {
        #if os(iOS)
        pictures[key] = UIImage(cgImage: image)
        #else
        pictures[key] = NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
        #endif
        pictureUse[key] = (image.bytesPerRow * image.height, now)
        // After the views on screen have redrawn (and said so).
        guard trimTask == nil else { return }
        trimTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            self?.trimTask = nil
            self?.trimPictures(to: Self.pictureBudget)
        }
    }

    private func mayTry(_ key: String) -> Bool {
        guard let at = failedAt[key] else { return true }
        if now - at < Self.retryAfter { return false }
        failedAt.removeValue(forKey: key)
        return true
    }

    private func failed(_ key: String) {
        failedAt[key] = now
    }

    /// Avatars and custom emoji are drawn small: decoded no bigger than
    /// this, however big the file says it is (a few hundred bytes can claim
    /// 20000 × 20000, gigabytes once decoded in full).
    static let smallImagePixels = 512

    private func load(_ url: URL) {
        let key = url.absoluteString
        guard !loading.contains(url), mayTry(key) else { return }
        loading.insert(url)
        Task {
            defer { loading.remove(url) }
            guard let (data, response) = try? await URLSession.shared.data(from: url),
                  (response as? HTTPURLResponse)?.statusCode ?? 200 < 400 else { return failed(key) }
            let side = Self.smallImagePixels
            let small = await Task.detached(priority: .userInitiated) { Self.downsample(data, maxPixel: side) }.value
            guard let small else { return failed(key) }
            #if os(iOS)
            images[url] = UIImage(cgImage: small)
            #else
            images[url] = NSImage(cgImage: small, size: NSSize(width: small.width, height: small.height))
            #endif
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
