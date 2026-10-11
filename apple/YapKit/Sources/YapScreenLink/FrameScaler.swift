import CoreVideo
import Foundation
import VideoToolbox

/// Shrinks a screen picture to the share's quality before it crosses the
/// link (the hardware scaler; the extension has 50 MB to live in, and a
/// phone's full screen is twice the pixels of Smooth's 1080p). The result
/// is always 4:2:0 bi-planar, from a small pool of its own.
public final class FrameScaler: @unchecked Sendable {
    private var session: VTPixelTransferSession?
    private var pool: CVPixelBufferPool?
    private var poolSize: (width: Int, height: Int)?

    public init() {}

    deinit {
        if let session { VTPixelTransferSessionInvalidate(session) }
    }

    /// `buffer` at no more than `settings`' size (either orientation): the
    /// same buffer when it already fits and is 4:2:0 bi-planar, else a
    /// scaled copy (nil if that failed).
    public func fit(_ buffer: CVPixelBuffer, to settings: ScreenLink.Settings) -> CVPixelBuffer? {
        let width = CVPixelBufferGetWidth(buffer), height = CVPixelBufferGetHeight(buffer)
        let (w, h) = settings.fit(width: width, height: height)
        let format = CVPixelBufferGetPixelFormatType(buffer)
        if w == width & ~1, h == height & ~1, ScreenLink.FrameInfo.formats.contains(format) { return buffer }
        return scale(buffer, width: w, height: h)
    }

    public func scale(_ buffer: CVPixelBuffer, width: Int, height: Int) -> CVPixelBuffer? {
        if session == nil {
            var created: VTPixelTransferSession?
            guard VTPixelTransferSessionCreate(allocator: nil, pixelTransferSessionOut: &created) == noErr, let created else { return nil }
            session = created
        }
        if poolSize.map({ $0.width != width || $0.height != height }) ?? true {
            pool = ScreenLinkPixels.pool(width: width, height: height, format: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange)
            poolSize = (width, height)
        }
        guard let session, let pool else { return nil }
        var out: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess, let scaled = out else { return nil }
        guard VTPixelTransferSessionTransferImage(session, from: buffer, to: scaled) == noErr else { return nil }
        return scaled
    }
}
