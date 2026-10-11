import Foundation

/// A doodle reaction being drawn (PROTOCOL.md › Doodle reactions): strokes
/// on a 48 × 48 pixel box, hard pixel edges like Draw, optionally wiggling
/// over 3 frames. Exported at 2× as a GIF (animated when it wiggles), well
/// under the 64 KB cap.
public struct DoodleDrawing: Sendable, Equatable {
    public struct Point: Sendable, Equatable {
        public var x: Float
        public var y: Float
        public init(x: Float, y: Float) {
            self.x = x
            self.y = y
        }
    }

    public struct Stroke: Sendable, Equatable {
        /// Index into `DoodleDrawing.palette`; 0 (paper) = eraser.
        public var color: Int
        /// Brush diameter in pixels.
        public var size: Int
        public var points: [Point]
        public init(color: Int, size: Int, points: [Point]) {
            self.color = color
            self.size = size
            self.points = points
        }
    }

    /// Paper first, then the inks (sRGB).
    public static let palette: [(r: UInt8, g: UInt8, b: UInt8)] = [
        (255, 255, 255), (20, 20, 24), (229, 72, 77), (247, 107, 21), (245, 191, 36), (48, 164, 108),
        (0, 144, 255), (142, 78, 198), (214, 64, 159), (139, 90, 43), (141, 141, 141), (18, 165, 148),
    ]
    public static let side = Doodle.size
    public static let sizes = [1, 2, 4]

    public var strokes: [Stroke] = []
    public var wiggle = false

    public init(strokes: [Stroke] = [], wiggle: Bool = false) {
        self.strokes = strokes
        self.wiggle = wiggle
    }

    public var isEmpty: Bool { !strokes.contains { $0.color != 0 } }

    /// One frame's pixels (palette indices, row-major `side` × `side`).
    /// Wiggle frames nudge every point by up to a pixel, the same way each time.
    public func pixels(frame: Int = 0) -> [UInt8] {
        let side = Self.side
        var grid = [UInt8](repeating: 0, count: side * side)
        for (s, stroke) in strokes.enumerated() {
            let points = stroke.points.enumerated().map { i, p in
                guard wiggle, frame > 0 else { return p }
                return Point(x: p.x + Self.jitter(frame, s, i, 0), y: p.y + Self.jitter(frame, s, i, 1))
            }
            let radius = Float(max(1, stroke.size)) / 2
            func dab(_ cx: Float, _ cy: Float) {
                let lo = Int((cx - radius).rounded(.down)), hi = Int((cx + radius).rounded(.up))
                let top = Int((cy - radius).rounded(.down)), bottom = Int((cy + radius).rounded(.up))
                for y in max(0, top)...min(side - 1, max(0, bottom)) where y < side {
                    for x in max(0, lo)...min(side - 1, max(0, hi)) where x < side {
                        let dx = Float(x) + 0.5 - cx, dy = Float(y) + 0.5 - cy
                        if dx * dx + dy * dy <= radius * radius + 0.25 { grid[y * side + x] = UInt8(stroke.color) }
                    }
                }
            }
            guard let first = points.first else { continue }
            dab(first.x, first.y)
            for (a, b) in zip(points, points.dropFirst()) {
                let steps = max(1, Int((max(abs(b.x - a.x), abs(b.y - a.y)) * 2).rounded(.up)))
                for k in 1...steps {
                    let t = Float(k) / Float(steps)
                    dab(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
                }
            }
        }
        return grid
    }

    /// −1…1 px, from a hash (no randomness: the same doodle wiggles the same way).
    private static func jitter(_ frame: Int, _ stroke: Int, _ point: Int, _ axis: Int) -> Float {
        var h = UInt32(truncatingIfNeeded: frame &* 73_856_093 ^ stroke &* 19_349_663 ^ (point / 3) &* 83_492_791 ^ axis &* 2_654_435_761)
        h ^= h >> 13
        h = h &* 0x5bd1e995
        h ^= h >> 15
        return Float(Int(h % 3) - 1) * 0.9
    }

    /// The GIF to upload: 96 × 96 (2×), 3 frames at 0.14 s (like the web) looping when it
    /// wiggles, one otherwise.
    public func gif() -> Data {
        let scale = Doodle.scale
        let side = Self.side
        let out = side * scale
        let frameCount = wiggle ? Doodle.maxFrames : 1
        let frames: [[UInt8]] = (0..<frameCount).map { f in
            let small = pixels(frame: f)
            var big = [UInt8](repeating: 0, count: out * out)
            for y in 0..<out {
                for x in 0..<out { big[y * out + x] = small[(y / scale) * side + x / scale] }
            }
            return big
        }
        return GIFEncoder.encode(width: out, height: out, frames: frames, palette: Self.palette, delay: 14)
    }
}

/// A tiny GIF89a writer: one global palette, LZW, optional looping
/// animation. Enough for doodles (and anything palette-sized).
public enum GIFEncoder {
    /// `frames` are palette indices, row-major; `delay` in 1/100 s.
    public static func encode(width: Int, height: Int, frames: [[UInt8]], palette: [(r: UInt8, g: UInt8, b: UInt8)], delay: Int = 10) -> Data {
        var data = Data("GIF89a".utf8)
        func u16(_ v: Int) { data.append(UInt8(v & 0xff)); data.append(UInt8((v >> 8) & 0xff)) }
        // Palette size: a power of two ≥ 2 (≥ 4 so the LZW min code size is ≥ 2).
        var bits = 2
        while (1 << bits) < palette.count { bits += 1 }
        let tableSize = 1 << bits
        u16(width)
        u16(height)
        data.append(0x80 | UInt8((bits - 1) << 4) | UInt8(bits - 1)) // global table, color resolution, size
        data.append(0) // background
        data.append(0) // aspect
        for i in 0..<tableSize {
            let c = i < palette.count ? palette[i] : (0, 0, 0)
            data.append(contentsOf: [c.0, c.1, c.2])
        }
        if frames.count > 1 {
            // NETSCAPE2.0: loop forever.
            data.append(contentsOf: [0x21, 0xFF, 0x0B])
            data.append(contentsOf: Array("NETSCAPE2.0".utf8))
            data.append(contentsOf: [0x03, 0x01, 0x00, 0x00, 0x00])
        }
        for frame in frames {
            // Graphic control: delay, no transparency, "do not dispose".
            data.append(contentsOf: [0x21, 0xF9, 0x04, 0x04])
            u16(frames.count > 1 ? delay : 0)
            data.append(contentsOf: [0x00, 0x00])
            data.append(0x2C)
            u16(0); u16(0); u16(width); u16(height)
            data.append(0) // no local table, not interlaced
            data.append(UInt8(bits))
            let lzw = compress(frame, minCodeSize: bits)
            var i = 0
            while i < lzw.count {
                let n = min(255, lzw.count - i)
                data.append(UInt8(n))
                data.append(contentsOf: lzw[i..<i + n])
                i += n
            }
            data.append(0)
        }
        data.append(0x3B)
        return data
    }

    /// GIF's variable-width LZW (codes up to 12 bits, LSB-first packing).
    static func compress(_ indices: [UInt8], minCodeSize: Int) -> [UInt8] {
        let clear = 1 << minCodeSize
        let end = clear + 1
        var codeSize = minCodeSize + 1
        var next = end + 1
        var table: [Int: Int] = [:] // (prefix << 8 | byte) → code
        var out: [UInt8] = []
        var buffer = 0, count = 0
        func emit(_ code: Int) {
            buffer |= code << count
            count += codeSize
            while count >= 8 {
                out.append(UInt8(buffer & 0xff))
                buffer >>= 8
                count -= 8
            }
        }
        emit(clear)
        guard var prefix = indices.first.map(Int.init) else {
            emit(end)
            if count > 0 { out.append(UInt8(buffer & 0xff)) }
            return out
        }
        for byte in indices.dropFirst() {
            let key = prefix << 8 | Int(byte)
            if let code = table[key] {
                prefix = code
                continue
            }
            emit(prefix)
            if next < 4096 {
                table[key] = next
                next += 1
                if next > (1 << codeSize), codeSize < 12 { codeSize += 1 }
            } else {
                emit(clear)
                table = [:]
                codeSize = minCodeSize + 1
                next = end + 1
            }
            prefix = Int(byte)
        }
        emit(prefix)
        emit(end)
        if count > 0 { out.append(UInt8(buffer & 0xff)) }
        return out
    }
}
