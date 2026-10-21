import CoreGraphics
import Foundation
import Testing
@testable import Yap

/// Message pictures stay decoded only while they're among the most recently
/// shown (a long session through a photo-heavy channel kept every one), and
/// never while they're on screen.
@MainActor
@Suite("Decoded pictures")
struct ImageStoreTests {
    /// A 1,024 × 1,024 picture: 4 MB decoded.
    static func picture() -> CGImage {
        let context = CGContext(data: nil, width: 1024, height: 1024, bitsPerComponent: 8, bytesPerRow: 1024 * 4,
                                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        return context.makeImage()!
    }

    @Test func theOnesShownLongestAgoGoFirst() {
        let store = ImageStore()
        var time = 1000.0
        store.clock = { time }
        let image = Self.picture()
        for n in 0..<40 {
            store.keep(image, key: "p\(n)")
            time += 1
        }
        // p3 is shown again (it's back on screen).
        #expect(store.cachedPicture("p3") != nil)
        time += 10
        store.trimPictures(to: 40 << 20)
        #expect(store.pictures.count == 10, "4 MB each, 40 MB")
        #expect(store.pictures["p3"] != nil, "shown just now")
        #expect(store.pictures["p39"] != nil && store.pictures["p31"] != nil, "the latest stay")
        #expect(store.pictures["p0"] == nil && store.pictures["p30"] == nil)
    }

    @Test func whatsOnScreenIsNeverLetGo() {
        let store = ImageStore()
        var time = 1000.0
        store.clock = { time }
        let image = Self.picture()
        for n in 0..<10 { store.keep(image, key: "p\(n)") }
        time += 1
        // Every one was shown a second ago: over budget, but all on screen.
        store.trimPictures(to: 8 << 20)
        #expect(store.pictures.count == 10)
        time += ImageStore.onScreenFor
        _ = store.cachedPicture("p7")
        store.trimPictures(to: 4 << 20)
        #expect(Set(store.pictures.keys) == ["p7"], "only the one still on screen; the rest went")
    }
}
