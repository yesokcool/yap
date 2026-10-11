import CoreVideo
import Foundation
import ImageIO
import os
import Testing
@testable import YapScreenLink

/// The link between the screen-broadcast extension and the app: the bytes
/// on the socket, the sizes pictures are sent at, the scaler, and a real
/// socket end to end (both ends in this process, synthetic pictures).
@Suite("Screen link: the wire")
struct ScreenLinkWireTests {
    @Test func headerRoundTripsAndRefusesWhatIsntOurs() throws {
        let header = ScreenLink.Header(kind: .frame, length: 1_234_567)
        let bytes = header.encoded()
        #expect(bytes.count == ScreenLink.headerSize)
        #expect(Array(bytes.prefix(4)) == Array("YAPL".utf8))
        #expect(try ScreenLink.Header.decode(bytes) == header)

        var bad = bytes
        bad[0] = 0x58
        #expect(throws: ScreenLink.Failure.badMagic) { try ScreenLink.Header.decode(bad) }
        bad = bytes
        bad[4] = 9
        #expect(throws: ScreenLink.Failure.badVersion(9)) { try ScreenLink.Header.decode(bad) }
        bad = bytes
        bad[5] = 7
        #expect(throws: ScreenLink.Failure.unknownKind(7)) { try ScreenLink.Header.decode(bad) }
        // Settings are tiny; a frame no bigger than 4096 × 4096.
        #expect(throws: ScreenLink.Failure.tooLarge(100)) { try ScreenLink.Header.decode(ScreenLink.Header(kind: .settings, length: 100).encoded()) }
        let huge = 24 + 4096 * 4096 * 3 / 2 + 1
        #expect(throws: ScreenLink.Failure.tooLarge(huge)) { try ScreenLink.Header.decode(ScreenLink.Header(kind: .frame, length: huge).encoded()) }
    }

    @Test func frameInfoRoundTripsAndChecksItself() throws {
        let info = ScreenLink.FrameInfo(width: 884, height: 1920, pixelFormat: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                                        rotation: 90, timestampNs: 123_456_789_012)
        #expect(info.lumaBytes == 884 * 1920 && info.chromaBytes == 884 * 960)
        let bytes = info.encoded()
        #expect(bytes.count == ScreenLink.FrameInfo.size)
        #expect(try ScreenLink.FrameInfo.decode(bytes, length: info.payloadLength) == info)
        #expect(throws: ScreenLink.Failure.self) { try ScreenLink.FrameInfo.decode(bytes, length: info.payloadLength + 1) }
        for broken in [
            ScreenLink.FrameInfo(width: 883, height: 1920),
            ScreenLink.FrameInfo(width: 0, height: 2),
            ScreenLink.FrameInfo(width: 5000, height: 2),
            ScreenLink.FrameInfo(width: 2, height: 2, pixelFormat: kCVPixelFormatType_32BGRA),
            ScreenLink.FrameInfo(width: 2, height: 2, rotation: 45),
        ] {
            #expect(throws: ScreenLink.Failure.self) { try broken.validate() }
        }
    }

    @Test func settingsRoundTrip() throws {
        let settings = ScreenLink.Settings(maxLong: 2560, maxShort: 1440, fps: 30)
        #expect(try ScreenLink.Settings.decode(settings.encoded()) == settings)
        #expect(throws: ScreenLink.Failure.self) { try ScreenLink.Settings.decode(ScreenLink.Settings(maxLong: 1, maxShort: 1, fps: 0).encoded()) }
        #expect(ScreenLink.Settings.initial.fps == 1, "nothing much until the app says someone's watching")
    }

    @Test func picturesFitEitherWayRound() {
        // An iPhone held upright at Smooth (1080p): the long side to 1920, not the height to 1080.
        #expect(ScreenLink.fit(width: 1179, height: 2556, maxLong: 1920, maxShort: 1080) == (884, 1920))
        #expect(ScreenLink.fit(width: 2556, height: 1179, maxLong: 1920, maxShort: 1080) == (1920, 884))
        // An iPad upright: the short side is what limits it.
        #expect(ScreenLink.fit(width: 2048, height: 2732, maxLong: 1920, maxShort: 1080) == (1080, 1440))
        // Max (4K) never scales a phone up; odd sizes go even.
        #expect(ScreenLink.fit(width: 1179, height: 2556, maxLong: 3840, maxShort: 2160) == (1178, 2556))
        #expect(ScreenLink.fit(width: 0, height: 10, maxLong: 1920, maxShort: 1080) == (0, 0))
    }

    @Test func aPhoneTurnedSidewaysIsTurnedBack() {
        #expect(ScreenLink.rotation(forOrientation: nil) == 0)
        #expect(ScreenLink.rotation(forOrientation: CGImagePropertyOrientation.up.rawValue) == 0)
        #expect(ScreenLink.rotation(forOrientation: CGImagePropertyOrientation.left.rawValue) == 90)
        #expect(ScreenLink.rotation(forOrientation: CGImagePropertyOrientation.down.rawValue) == 180)
        #expect(ScreenLink.rotation(forOrientation: CGImagePropertyOrientation.right.rawValue) == 270)
    }

    @Test func theSocketLivesInTheAppGroup() {
        #expect(ScreenLink.socketName.utf8.count < 20, "a socket path can't be longer than 103 bytes")
        // On a phone: /private/var/mobile/Containers/Shared/AppGroup/<UUID>/yap-screen
        let phone = "/private/var/mobile/Containers/Shared/AppGroup/\(UUID().uuidString)/\(ScreenLink.socketName)"
        #expect(phone.utf8.count <= UnixSocket.maxPath)
    }
}

/// A 4:2:0 bi-planar picture with a known pattern: the left half dark, the
/// right half bright, a chroma ramp.
func testPicture(width: Int, height: Int, format: OSType = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, seed: UInt8 = 0) -> CVPixelBuffer {
    let pool = ScreenLinkPixels.pool(width: width, height: height, format: format)!
    var out: CVPixelBuffer?
    CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out)
    let buffer = out!
    CVPixelBufferLockBaseAddress(buffer, [])
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
    let y = CVPixelBufferGetBaseAddressOfPlane(buffer, 0)!.assumingMemoryBound(to: UInt8.self)
    let uv = CVPixelBufferGetBaseAddressOfPlane(buffer, 1)!.assumingMemoryBound(to: UInt8.self)
    let yStride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0), uvStride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 1)
    for row in 0..<height {
        for col in 0..<width { y[row * yStride + col] = (col < width / 2 ? 30 : 220) &+ seed }
    }
    for row in 0..<height / 2 {
        for col in 0..<width { uv[row * uvStride + col] = UInt8((col * 255) / max(1, width)) }
    }
    return buffer
}

/// The planes as packed bytes (to compare pictures).
func planes(_ buffer: CVPixelBuffer) -> [UInt8] {
    CVPixelBufferLockBaseAddress(buffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
    let width = CVPixelBufferGetWidth(buffer), height = CVPixelBufferGetHeight(buffer)
    var bytes = [UInt8](repeating: 0, count: width * height * 3 / 2)
    bytes.withUnsafeMutableBytes { raw in
        ScreenLinkPixels.pack(buffer, plane: 0, rows: height, rowBytes: width, into: raw.baseAddress!)
        ScreenLinkPixels.pack(buffer, plane: 1, rows: height / 2, rowBytes: width, into: raw.baseAddress! + width * height)
    }
    return bytes
}

@Suite("Screen link: scaling")
struct FrameScalerTests {
    @Test func aPictureThatFitsGoesAsItIs() {
        let picture = testPicture(width: 640, height: 360)
        let scaler = FrameScaler()
        #expect(scaler.fit(picture, to: .init(maxLong: 1920, maxShort: 1080, fps: 30)) === picture, "no copy")
    }

    @Test func aBigPictureShrinksKeepingItsShape() throws {
        let picture = testPicture(width: 1280, height: 720)
        let scaler = FrameScaler()
        let scaled = try #require(scaler.fit(picture, to: .init(maxLong: 640, maxShort: 360, fps: 30)))
        #expect(CVPixelBufferGetWidth(scaled) == 640 && CVPixelBufferGetHeight(scaled) == 360)
        #expect(CVPixelBufferGetPixelFormatType(scaled) == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange)
        let bytes = planes(scaled)
        // Still dark on the left and bright on the right.
        let row = 180 * 640
        #expect(bytes[row + 100] < 60 && bytes[row + 540] > 190, "\(bytes[row + 100]) / \(bytes[row + 540])")
        // A portrait one too (an iPhone held upright).
        let tall = try #require(scaler.fit(testPicture(width: 720, height: 1280), to: .init(maxLong: 640, maxShort: 360, fps: 30)))
        #expect(CVPixelBufferGetWidth(tall) == 360 && CVPixelBufferGetHeight(tall) == 640)
    }
}

/// Both ends in this process over a real socket.
@Suite("Screen link: end to end", .serialized)
final class ScreenLinkSocketTests {
    /// A Unix socket's path fits in 103 bytes; the simulator's temporary folder is longer.
    let path: String = {
        let name = "yl-\(UUID().uuidString.prefix(8))"
        let temporary = (NSTemporaryDirectory() as NSString).appendingPathComponent(name)
        return temporary.utf8.count <= 100 ? temporary : "/tmp/" + name
    }()

    deinit { unlink(path) }

    private func waitUntil(_ what: String, timeout: Duration = .seconds(5), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    struct Received: Sendable {
        var frames: [(bytes: [UInt8], width: Int, height: Int, rotation: Int, timestamp: UInt64)] = []
        var settings: [ScreenLink.Settings] = []
        var appClosed = false
        var extensionClosed = false
        var connections = 0
    }

    @Test func framesGoOneWaySettingsTheOtherAndEitherSideCanHangUp() async throws {
        let received = OSAllocatedUnfairLock(initialState: Received())
        let server = ScreenLinkServer(path: path)
        let app = OSAllocatedUnfairLock<ScreenLinkConnection?>(initialState: nil)
        server.onConnection = { connection in
            connection.onFrame = { buffer, rotation, timestamp in
                let bytes = planes(buffer)
                let (w, h) = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer))
                received.withLock { $0.frames.append((bytes, w, h, rotation, timestamp)) }
            }
            connection.onClose = { received.withLock { $0.appClosed = true } }
            connection.start()
            app.withLock { $0 = connection }
            received.withLock { $0.connections += 1 }
        }
        try server.start()
        defer { server.stop() }

        let client = try ScreenLinkClient.connect(path: path)
        client.onSettings = { settings in received.withLock { $0.settings.append(settings) } }
        client.onClose = { received.withLock { $0.extensionClosed = true } }
        client.start()
        try await waitUntil("the app accepts") { received.withLock { $0.connections } == 1 }

        // App → extension: the quality.
        let connection = try #require(app.withLock { $0 })
        let smooth = ScreenLink.Settings(maxLong: 1920, maxShort: 1080, fps: 60)
        connection.send(smooth)
        try await waitUntil("settings arrive") { received.withLock { $0.settings } == [smooth] }

        // Extension → app: three pictures, every byte, turned and timed.
        let pictures = (0..<3).map { testPicture(width: 640, height: 360, seed: UInt8($0 * 10)) }
        for (i, picture) in pictures.enumerated() {
            try await waitUntil("free to send") { !client.isBusy }
            #expect(client.send(picture, rotation: i == 2 ? 90 : 0, timestampNs: UInt64(1000 + i)))
        }
        try await waitUntil("three pictures") { received.withLock { $0.frames.count } == 3 }
        let frames = received.withLock { $0.frames }
        for (i, frame) in frames.enumerated() {
            #expect(frame.width == 640 && frame.height == 360)
            #expect(frame.bytes == planes(pictures[i]), "picture \(i) arrives byte for byte")
            #expect(frame.timestamp == UInt64(1000 + i))
        }
        #expect(frames.map(\.rotation) == [0, 0, 90])

        // One frame in flight: a second one right away is skipped, not queued.
        try await waitUntil("free") { !client.isBusy }
        let big = testPicture(width: 2048, height: 2048)
        #expect(client.send(big, rotation: 0, timestampNs: 1))
        #expect(!client.send(big, rotation: 0, timestampNs: 2), "the first is still going")

        // The app stops sharing: the extension hears it (and ends the broadcast).
        connection.close()
        try await waitUntil("the extension hears the app hang up") { received.withLock { $0.extensionClosed } }
        try await waitUntil("the app's end closes too") { received.withLock { $0.appClosed } }
        #expect(!client.send(pictures[0], rotation: 0, timestampNs: 9), "nothing goes once it's over")
    }

    @Test func stoppingFromControlCenterEndsItInTheApp() async throws {
        let closed = OSAllocatedUnfairLock(initialState: false)
        let server = ScreenLinkServer(path: path)
        server.onConnection = { connection in
            connection.onClose = { closed.withLock { $0 = true } }
            connection.start()
        }
        try server.start()
        defer { server.stop() }
        let client = try ScreenLinkClient.connect(path: path)
        client.start()
        // The broadcast finished (Control Center, the red pill): the extension hangs up.
        client.close()
        try await waitUntil("the app's share ends") { closed.withLock { $0 } }
    }

    @Test func notListeningMeansNotInACall() throws {
        #expect(throws: ScreenLink.Failure.self) { _ = try ScreenLinkClient.connect(path: path) }
        // And a stopped server takes its socket with it.
        let server = ScreenLinkServer(path: path)
        try server.start()
        #expect(FileManager.default.fileExists(atPath: path))
        server.stop()
        Thread.sleep(forTimeInterval: 0.1)
        #expect(throws: ScreenLink.Failure.self) { _ = try ScreenLinkClient.connect(path: path) }
    }
}
