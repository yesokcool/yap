import CoreVideo
import Foundation
import os

/// One end of the link: a thread that reads, a queue that writes, one goodbye.
/// The socket is closed only after the reader is out and no write is in
/// flight, so its number is never reused under anyone's feet.
public class ScreenLinkEndpoint: @unchecked Sendable {
    let fd: Int32
    let writer: DispatchQueue
    private let state = OSAllocatedUnfairLock(initialState: (closed: false, reported: false))
    private var onCloseHandler: (@Sendable () -> Void)?

    init(fd: Int32, label: String) {
        self.fd = fd
        writer = DispatchQueue(label: label)
    }

    /// Called once when the link ends (the other side hung up, or `close()`).
    public var onClose: (@Sendable () -> Void)? {
        get { state.withLock { _ in onCloseHandler } }
        set { state.withLock { _ in onCloseHandler = newValue } }
    }

    public var isClosed: Bool { state.withLock { $0.closed } }

    /// Starts reading (set the handlers first).
    public func start() {
        let thread = Thread { [self] in
            do {
                try readLoop()
            } catch {
                // The other side hung up (or sent something we don't read).
            }
            finish()
        }
        thread.name = "yap.screenlink.read"
        thread.qualityOfService = .userInteractive
        thread.start()
    }

    /// Hang up. The other side's reads end; ours do too.
    public func close() {
        let first = state.withLock { s -> Bool in
            guard !s.closed else { return false }
            s.closed = true
            return true
        }
        if first { UnixSocket.hangUp(fd) }
    }

    func readLoop() throws(ScreenLink.Failure) {}

    private func finish() {
        close()
        let fd = fd
        writer.async { UnixSocket.close(fd) }
        let handler = state.withLock { s -> (@Sendable () -> Void)? in
            guard !s.reported else { return nil }
            s.reported = true
            return onCloseHandler
        }
        handler?()
    }

    /// On the writer queue, unless the link has ended. A failed write ends it.
    func write(_ body: @escaping @Sendable (Int32) throws(ScreenLink.Failure) -> Void) {
        writer.async { [self] in
            guard !isClosed else { return }
            do {
                try body(fd)
            } catch {
                close()
            }
        }
    }

    func skip(_ count: Int) throws(ScreenLink.Failure) {
        var left = count
        var scratch = [UInt8](repeating: 0, count: min(count, 4096))
        while left > 0 {
            let n = min(left, scratch.count)
            try scratch.withMutableRawBytes { raw throws(ScreenLink.Failure) in try UnixSocket.read(fd, into: raw.baseAddress!, count: n) }
            left -= n
        }
    }

    func readBytes(_ count: Int) throws(ScreenLink.Failure) -> [UInt8] {
        var bytes = [UInt8](repeating: 0, count: count)
        try bytes.withMutableRawBytes { raw throws(ScreenLink.Failure) in try UnixSocket.read(fd, into: raw.baseAddress!, count: count) }
        return bytes
    }
}

// MARK: - The app's side

/// Listens at the socket for Yap's broadcast extension (one at a time;
/// each connection is handed over as it comes).
public final class ScreenLinkServer: @unchecked Sendable {
    public let path: String
    private var source: DispatchSourceRead?
    private let queue = DispatchQueue(label: "yap.screenlink.accept")
    /// A broadcast connected (called on a background queue).
    public var onConnection: (@Sendable (ScreenLinkConnection) -> Void)?

    public init(path: String) {
        self.path = path
    }

    public var isListening: Bool { source != nil }

    public func start() throws(ScreenLink.Failure) {
        guard source == nil else { return }
        let fd = try UnixSocket.listen(at: path)
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in
            while let connection = UnixSocket.accept(fd) {
                guard let handler = self?.onConnection else {
                    UnixSocket.close(connection)
                    continue
                }
                handler(ScreenLinkConnection(fd: connection))
            }
        }
        let path = path
        source.setCancelHandler {
            UnixSocket.close(fd)
            unlink(path)
        }
        source.resume()
        self.source = source
    }

    public func stop() {
        source?.cancel()
        source = nil
    }

    deinit { source?.cancel() }
}

/// The app's end of one broadcast: frames in (as pixel buffers ready for
/// the encoder), settings out.
public final class ScreenLinkConnection: ScreenLinkEndpoint, @unchecked Sendable {
    /// A picture (called on the reading thread): the buffer, how far to
    /// turn it clockwise to be upright, and when it was captured.
    public var onFrame: (@Sendable (CVPixelBuffer, _ rotation: Int, _ timestampNs: UInt64) -> Void)?
    /// Frames received so far (tests).
    public private(set) var framesReceived = 0
    private var pool: CVPixelBufferPool?
    private var poolKey: (width: Int, height: Int, format: OSType)?
    private var scratch: [UInt8] = []

    init(fd: Int32) {
        super.init(fd: fd, label: "yap.screenlink.app.write")
    }

    public func send(_ settings: ScreenLink.Settings) {
        let bytes = ScreenLink.message(.settings, payload: settings.encoded())
        write { fd throws(ScreenLink.Failure) in try UnixSocket.write(fd, bytes) }
    }

    override func readLoop() throws(ScreenLink.Failure) {
        while true {
            let header = try ScreenLink.Header.decode(readBytes(ScreenLink.headerSize))
            switch header.kind {
            case .settings:
                try skip(header.length)
            case .frame:
                let info = try ScreenLink.FrameInfo.decode(readBytes(ScreenLink.FrameInfo.size), length: header.length)
                let planes = info.lumaBytes + info.chromaBytes
                if scratch.count < planes { scratch = [UInt8](repeating: 0, count: planes) }
                try scratch.withMutableRawBytes { raw throws(ScreenLink.Failure) in try UnixSocket.read(fd, into: raw.baseAddress!, count: planes) }
                guard let buffer = makeBuffer(info) else { continue }
                framesReceived += 1
                onFrame?(buffer, info.rotation, info.timestampNs)
            }
        }
    }

    /// The planes, row by row, into a buffer the encoder can take as is.
    private func makeBuffer(_ info: ScreenLink.FrameInfo) -> CVPixelBuffer? {
        if poolKey.map({ $0.width != info.width || $0.height != info.height || $0.format != info.pixelFormat }) ?? true {
            pool = ScreenLinkPixels.pool(width: info.width, height: info.height, format: info.pixelFormat)
            poolKey = (info.width, info.height, info.pixelFormat)
        }
        guard let pool else { return nil }
        var out: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess, let buffer = out else { return nil }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        scratch.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            ScreenLinkPixels.unpack(base, plane: 0, rows: info.height, rowBytes: info.width, into: buffer)
            ScreenLinkPixels.unpack(base + info.lumaBytes, plane: 1, rows: info.height / 2, rowBytes: info.width, into: buffer)
        }
        return buffer
    }
}

// MARK: - The extension's side

/// The extension's end: frames out (one in flight; a busy link skips
/// frames rather than queueing them), settings in.
public final class ScreenLinkClient: ScreenLinkEndpoint, @unchecked Sendable {
    public var onSettings: (@Sendable (ScreenLink.Settings) -> Void)?
    private let busy = OSAllocatedUnfairLock(initialState: false)
    private var scratch: [UInt8] = []

    /// Connects to the app's socket; throws when Yap isn't listening (not in a call).
    public static func connect(path: String) throws(ScreenLink.Failure) -> ScreenLinkClient {
        ScreenLinkClient(fd: try UnixSocket.connect(to: path))
    }

    init(fd: Int32) {
        super.init(fd: fd, label: "yap.screenlink.extension.write")
    }

    /// Still writing the last frame.
    public var isBusy: Bool { busy.withLock { $0 } }

    /// Sends a 4:2:0 bi-planar picture; false (and nothing sent) while the
    /// last one is still going, or once the link has ended.
    @discardableResult
    public func send(_ buffer: CVPixelBuffer, rotation: Int, timestampNs: UInt64) -> Bool {
        guard !isClosed else { return false }
        let format = CVPixelBufferGetPixelFormatType(buffer)
        guard ScreenLink.FrameInfo.formats.contains(format) else { return false }
        let free = busy.withLock { inFlight -> Bool in
            guard !inFlight else { return false }
            inFlight = true
            return true
        }
        guard free else { return false }
        nonisolated(unsafe) let buffer = buffer
        write { [self] fd throws(ScreenLink.Failure) in
            defer { busy.withLock { $0 = false } }
            let width = CVPixelBufferGetWidth(buffer) & ~1, height = CVPixelBufferGetHeight(buffer) & ~1
            let info = ScreenLink.FrameInfo(width: width, height: height, pixelFormat: format, rotation: rotation, timestampNs: timestampNs)
            try info.validate()
            let planes = info.lumaBytes + info.chromaBytes
            if scratch.count < planes { scratch = [UInt8](repeating: 0, count: planes) }
            CVPixelBufferLockBaseAddress(buffer, .readOnly)
            scratch.withUnsafeMutableBytes { raw in
                guard let base = raw.baseAddress else { return }
                ScreenLinkPixels.pack(buffer, plane: 0, rows: height, rowBytes: width, into: base)
                ScreenLinkPixels.pack(buffer, plane: 1, rows: height / 2, rowBytes: width, into: base + info.lumaBytes)
            }
            CVPixelBufferUnlockBaseAddress(buffer, .readOnly)
            try UnixSocket.write(fd, ScreenLink.Header(kind: .frame, length: info.payloadLength).encoded() + info.encoded())
            try scratch.withRawBytes { raw throws(ScreenLink.Failure) in try UnixSocket.write(fd, raw.baseAddress!, count: planes) }
        }
        return true
    }

    override func readLoop() throws(ScreenLink.Failure) {
        while true {
            let header = try ScreenLink.Header.decode(readBytes(ScreenLink.headerSize))
            switch header.kind {
            case .settings:
                let settings = try ScreenLink.Settings.decode(readBytes(header.length))
                onSettings?(settings)
            case .frame:
                try skip(header.length)
            }
        }
    }
}

// MARK: - Pixels

/// Plane rows in and out of pixel buffers, packed tight on the wire.
public enum ScreenLinkPixels {
    /// IOSurface-backed, so the hardware encoder takes them without a copy.
    public static func pool(width: Int, height: Int, format: OSType) -> CVPixelBufferPool? {
        let attributes: [String: Any] = [
            kCVPixelBufferPixelFormatTypeKey as String: format,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
            kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
        ]
        var pool: CVPixelBufferPool?
        CVPixelBufferPoolCreate(nil, [kCVPixelBufferPoolMinimumBufferCountKey as String: 3] as CFDictionary, attributes as CFDictionary, &pool)
        return pool
    }

    static func pack(_ buffer: CVPixelBuffer, plane: Int, rows: Int, rowBytes: Int, into out: UnsafeMutableRawPointer) {
        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) else { return }
        let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
        let rows = min(rows, CVPixelBufferGetHeightOfPlane(buffer, plane))
        let bytes = min(rowBytes, stride)
        for row in 0..<rows { (out + row * rowBytes).copyMemory(from: base + row * stride, byteCount: bytes) }
    }

    static func unpack(_ input: UnsafeRawPointer, plane: Int, rows: Int, rowBytes: Int, into buffer: CVPixelBuffer) {
        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) else { return }
        let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
        let rows = min(rows, CVPixelBufferGetHeightOfPlane(buffer, plane))
        let bytes = min(rowBytes, stride)
        for row in 0..<rows { (base + row * stride).copyMemory(from: input + row * rowBytes, byteCount: bytes) }
    }
}
