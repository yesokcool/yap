import CoreVideo
import Foundation
import ImageIO

/// The link between Yap's screen-broadcast extension and the app (iPhone
/// and iPad sharing): ReplayKit hands the extension the screen, the
/// extension scales each picture down to the share's quality and passes it
/// over a Unix domain socket in the App Group's container; the app feeds it
/// to its screen video source, exactly like the Mac's capture. Nothing here
/// leaves the device, and it's not the server's protocol.
///
/// Every message: a 12-byte header, then its payload.
///
///     "YAPL"  version(1)  kind(1)  flags(2, 0)  payload length(4, big-endian)
///
/// - `frame` (extension → app): `FrameInfo` (24 bytes), then the luma plane
///   (width × height bytes) and the interleaved chroma plane (width ×
///   height/2 bytes), rows packed tight. 4:2:0 bi-planar only (NV12, full
///   or video range).
/// - `settings` (app → extension): the largest picture (long side × short
///   side, either orientation) and the frame rate wanted right now (the
///   quality's when someone watches, 1 fps while nobody does).
public enum ScreenLink {
    public static let magic: [UInt8] = Array("YAPL".utf8)
    public static let version: UInt8 = 1
    public static let headerSize = 12
    /// The socket's file name in the App Group container (kept short:
    /// a Unix socket's path can't be longer than 103 bytes).
    public static let socketName = "yap-screen"
    /// Larger than any screen we'd send (and than 50 MB of extension memory allows).
    public static let maxSide = 4096

    public enum Kind: UInt8, Sendable {
        case frame = 1
        case settings = 2
    }

    public enum Failure: Error, Equatable, Sendable {
        case badMagic
        case badVersion(UInt8)
        case unknownKind(UInt8)
        case badFrame(String)
        case tooLarge(Int)
        /// The other side hung up (or the socket broke).
        case closed
        case socket(String)
    }

    // MARK: Header

    public struct Header: Equatable, Sendable {
        public var kind: Kind
        public var length: Int

        public init(kind: Kind, length: Int) {
            self.kind = kind
            self.length = length
        }

        public func encoded() -> [UInt8] {
            var bytes = ScreenLink.magic
            bytes += [ScreenLink.version, kind.rawValue, 0, 0]
            bytes += UInt32(length).bigEndianBytes
            return bytes
        }

        public static func decode(_ bytes: some Collection<UInt8>) throws(Failure) -> Header {
            let b = Array(bytes)
            guard b.count >= ScreenLink.headerSize else { throw .badFrame("short header") }
            guard Array(b[0..<4]) == ScreenLink.magic else { throw .badMagic }
            guard b[4] == ScreenLink.version else { throw .badVersion(b[4]) }
            guard let kind = Kind(rawValue: b[5]) else { throw .unknownKind(b[5]) }
            let length = Int(UInt32(bigEndian: b, at: 8))
            let limit = kind == .frame ? FrameInfo.size + FrameInfo.planeBytes(width: maxSide, height: maxSide) : 64
            guard length <= limit else { throw .tooLarge(length) }
            return Header(kind: kind, length: length)
        }
    }

    // MARK: Frames

    public struct FrameInfo: Equatable, Sendable {
        public static let size = 24
        public var width: Int
        public var height: Int
        /// `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` or `…VideoRange`.
        public var pixelFormat: OSType
        /// Clockwise degrees the picture must be turned to be upright: 0, 90, 180, 270.
        public var rotation: Int
        public var timestampNs: UInt64

        public init(width: Int, height: Int, pixelFormat: OSType = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                    rotation: Int = 0, timestampNs: UInt64 = 0) {
            self.width = width
            self.height = height
            self.pixelFormat = pixelFormat
            self.rotation = rotation
            self.timestampNs = timestampNs
        }

        public static let formats: Set<OSType> = [kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]

        public static func planeBytes(width: Int, height: Int) -> Int { width * height + width * (height / 2) }
        public var lumaBytes: Int { width * height }
        public var chromaBytes: Int { width * (height / 2) }
        public var payloadLength: Int { Self.size + lumaBytes + chromaBytes }

        public func validate() throws(Failure) {
            guard (2...ScreenLink.maxSide).contains(width), (2...ScreenLink.maxSide).contains(height),
                  width % 2 == 0, height % 2 == 0 else { throw .badFrame("size \(width)×\(height)") }
            guard Self.formats.contains(pixelFormat) else { throw .badFrame("pixel format \(pixelFormat)") }
            guard [0, 90, 180, 270].contains(rotation) else { throw .badFrame("rotation \(rotation)") }
        }

        public func encoded() -> [UInt8] {
            var bytes: [UInt8] = []
            bytes += UInt32(width).bigEndianBytes
            bytes += UInt32(height).bigEndianBytes
            bytes += UInt32(pixelFormat).bigEndianBytes
            bytes += UInt16(rotation).bigEndianBytes
            bytes += [0, 0]
            bytes += timestampNs.bigEndianBytes
            return bytes
        }

        public static func decode(_ bytes: some Collection<UInt8>, length: Int) throws(Failure) -> FrameInfo {
            let b = Array(bytes)
            guard b.count >= size else { throw .badFrame("short frame header") }
            let info = FrameInfo(width: Int(UInt32(bigEndian: b, at: 0)), height: Int(UInt32(bigEndian: b, at: 4)),
                                 pixelFormat: UInt32(bigEndian: b, at: 8), rotation: Int(UInt16(bigEndian: b, at: 12)),
                                 timestampNs: UInt64(bigEndian: b, at: 16))
            try info.validate()
            guard info.payloadLength == length else { throw .badFrame("length \(length) for \(info.width)×\(info.height)") }
            return info
        }
    }

    // MARK: Settings

    public struct Settings: Equatable, Sendable {
        public static let size = 12
        public var maxLong: Int
        public var maxShort: Int
        public var fps: Int

        public init(maxLong: Int, maxShort: Int, fps: Int) {
            self.maxLong = maxLong
            self.maxShort = maxShort
            self.fps = fps
        }

        /// Until the app says otherwise: 1080p, one picture a second.
        public static let initial = Settings(maxLong: 1920, maxShort: 1080, fps: 1)

        public func encoded() -> [UInt8] {
            UInt32(maxLong).bigEndianBytes + UInt32(maxShort).bigEndianBytes + UInt16(fps).bigEndianBytes + [0, 0]
        }

        public static func decode(_ bytes: some Collection<UInt8>) throws(Failure) -> Settings {
            let b = Array(bytes)
            guard b.count >= size else { throw .badFrame("short settings") }
            let settings = Settings(maxLong: Int(UInt32(bigEndian: b, at: 0)), maxShort: Int(UInt32(bigEndian: b, at: 4)),
                                    fps: Int(UInt16(bigEndian: b, at: 8)))
            guard settings.maxLong >= 2, settings.maxShort >= 2, settings.fps >= 1 else { throw .badFrame("settings \(settings)") }
            return settings
        }

        /// The size to send a `width`×`height` picture at (see `ScreenLink.fit`).
        public func fit(width: Int, height: Int) -> (width: Int, height: Int) {
            ScreenLink.fit(width: width, height: height, maxLong: maxLong, maxShort: maxShort)
        }
    }

    /// A picture scaled down (never up) so its long side fits `maxLong` and
    /// its short side `maxShort`, keeping its shape; even sizes (the encoder
    /// works in 2×2 blocks). A phone held upright is 1080p at 886×1920,
    /// not 498×1080.
    public static func fit(width: Int, height: Int, maxLong: Int, maxShort: Int) -> (width: Int, height: Int) {
        guard width > 0, height > 0 else { return (0, 0) }
        let long = Double(max(width, height)), short = Double(min(width, height))
        let scale = min(1, Double(maxLong) / long, Double(maxShort) / short)
        func even(_ value: Double) -> Int { max(2, Int((value / 2).rounded(.down)) * 2) }
        return (even(Double(width) * scale), even(Double(height) * scale))
    }

    /// How far (clockwise degrees) a ReplayKit picture must turn to be
    /// upright, from its `RPVideoSampleOrientationKey` (an EXIF orientation):
    /// the screen comes in the phone's own upright shape however it's held.
    public static func rotation(forOrientation raw: UInt32?) -> Int {
        switch raw.flatMap(CGImagePropertyOrientation.init(rawValue:)) {
        case .left?, .leftMirrored?: 90
        case .down?, .downMirrored?: 180
        case .right?, .rightMirrored?: 270
        default: 0
        }
    }

    /// The socket in the App Group container both sides share.
    public static func socketPath(appGroup: String) -> String? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup) else { return nil }
        return container.appendingPathComponent(socketName).path
    }

    /// Whole message bytes (tests, and the small messages).
    public static func message(_ kind: Kind, payload: [UInt8]) -> [UInt8] {
        Header(kind: kind, length: payload.count).encoded() + payload
    }
}

extension FixedWidthInteger {
    var bigEndianBytes: [UInt8] {
        withUnsafeBytes(of: bigEndian) { Array($0) }
    }

    init(bigEndian bytes: [UInt8], at offset: Int) {
        var value: Self = 0
        for i in 0..<MemoryLayout<Self>.size { value = value << 8 | Self(bytes[offset + i]) }
        self = value
    }
}
