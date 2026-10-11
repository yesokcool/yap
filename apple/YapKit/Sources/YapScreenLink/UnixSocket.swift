import Darwin
import Foundation

/// The few POSIX calls the link needs: a stream socket at a path, blocking
/// reads and writes of whole buffers, no SIGPIPE when the other side is gone.
enum UnixSocket {
    /// `sockaddr_un.sun_path` (104 bytes on Darwin, with the terminating 0).
    static let maxPath = 103

    private static func address(_ path: String) throws(ScreenLink.Failure) -> sockaddr_un {
        let bytes = Array(path.utf8)
        guard !bytes.isEmpty, bytes.count <= maxPath else { throw .socket("path too long (\(bytes.count) bytes): \(path)") }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        return address
    }

    private static func failure(_ what: String) -> ScreenLink.Failure {
        .socket("\(what): \(String(cString: strerror(errno)))")
    }

    private static func makeSocket() throws(ScreenLink.Failure) -> Int32 {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw failure("socket") }
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        return fd
    }

    /// A listening socket at `path` (an old one left behind is replaced).
    static func listen(at path: String) throws(ScreenLink.Failure) -> Int32 {
        var address = try address(path)
        let fd = try makeSocket()
        unlink(path)
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard bound == 0 else {
            let error = failure("bind")
            Darwin.close(fd)
            throw error
        }
        guard Darwin.listen(fd, 2) == 0 else {
            let error = failure("listen")
            Darwin.close(fd)
            throw error
        }
        // Accepting is driven by a dispatch source: never block on it.
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
        return fd
    }

    /// A connection waiting on `listener`, or nil if there's none right now.
    static func accept(_ listener: Int32) -> Int32? {
        let fd = Darwin.accept(listener, nil, nil)
        guard fd >= 0 else { return nil }
        // Blocking reads and writes on the connection itself.
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) & ~O_NONBLOCK)
        var on: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        return fd
    }

    static func connect(to path: String) throws(ScreenLink.Failure) -> Int32 {
        var address = try address(path)
        let fd = try makeSocket()
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard connected == 0 else {
            let error = failure("connect")
            Darwin.close(fd)
            throw error
        }
        return fd
    }

    /// Exactly `count` bytes, or `.closed` when the other side hangs up.
    static func read(_ fd: Int32, into buffer: UnsafeMutableRawPointer, count: Int) throws(ScreenLink.Failure) {
        var done = 0
        while done < count {
            let n = Darwin.read(fd, buffer + done, count - done)
            if n > 0 {
                done += n
            } else if n == 0 {
                throw .closed
            } else if errno != EINTR {
                throw .closed
            }
        }
    }

    static func write(_ fd: Int32, _ buffer: UnsafeRawPointer, count: Int) throws(ScreenLink.Failure) {
        var done = 0
        while done < count {
            let n = Darwin.write(fd, buffer + done, count - done)
            if n > 0 {
                done += n
            } else if n < 0, errno == EINTR {
                continue
            } else {
                throw .closed
            }
        }
    }

    static func write(_ fd: Int32, _ bytes: [UInt8]) throws(ScreenLink.Failure) {
        try bytes.withRawBytes { raw throws(ScreenLink.Failure) in try write(fd, raw.baseAddress!, count: raw.count) }
    }

    /// Ends the conversation: a blocked read on `fd` returns (either side).
    /// Closing it is its owner's job, once nothing reads or writes it.
    static func hangUp(_ fd: Int32) {
        Darwin.shutdown(fd, SHUT_RDWR)
    }

    static func close(_ fd: Int32) {
        Darwin.close(fd)
    }
}

extension Array where Element == UInt8 {
    /// `withUnsafeBytes`, keeping the link's error type.
    func withRawBytes(_ body: (UnsafeRawBufferPointer) throws(ScreenLink.Failure) -> Void) throws(ScreenLink.Failure) {
        var failure: ScreenLink.Failure?
        withUnsafeBytes { raw in
            do throws(ScreenLink.Failure) { try body(raw) } catch { failure = error }
        }
        if let failure { throw failure }
    }

    mutating func withMutableRawBytes(_ body: (UnsafeMutableRawBufferPointer) throws(ScreenLink.Failure) -> Void) throws(ScreenLink.Failure) {
        var failure: ScreenLink.Failure?
        withUnsafeMutableBytes { raw in
            do throws(ScreenLink.Failure) { try body(raw) } catch { failure = error }
        }
        if let failure { throw failure }
    }
}
