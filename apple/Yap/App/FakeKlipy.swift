#if DEBUG
import Foundation
import YapCore
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// DEBUG only (`YAP_FAKE_KLIPY=1`): a pretend KLIPY so the GIF picker can be
/// screenshotted without a real key or network, like the web e2e test's
/// Playwright route. Tiles are colored PNGs with a label.
nonisolated final class FakeKlipy: URLProtocol, @unchecked Sendable {
    static func install() {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [FakeKlipy.self]
        KlipyClient.mediaSession = URLSession(configuration: config)
    }

    override class func canInit(with request: URLRequest) -> Bool {
        guard let host = request.url?.host() else { return false }
        return host == "api.klipy.com" || host.hasSuffix(".klipy.com")
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let url = request.url else { return }
        let (data, type): (Data, String)
        if url.host() == "api.klipy.com" {
            (data, type) = (Self.page(for: url), "application/json")
        } else {
            (data, type) = (Self.tile(named: url.deletingPathExtension().lastPathComponent), "image/png")
        }
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": type])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static let words = ["yay", "lets-go", "nope", "dance", "cozy", "gg", "snacks", "wave", "hype", "sleepy", "oops", "cheers"]
    private static let sizes = [(480, 270), (400, 400), (360, 480), (480, 300)]

    static func page(for url: URL) -> Data {
        let items = words.enumerated().map { i, word -> [String: Any] in
            let (w, h) = sizes[i % sizes.count]
            func file(_ ext: String) -> [String: Any] { ["url": "https://static.klipy.com/fake/\(word)-\(i).\(ext)", "width": w, "height": h] }
            return ["slug": "\(word)-\(i)", "title": word, "file": ["md": ["webp": file("webp"), "jpg": file("jpg")]]]
        }
        let body: [String: Any] = ["result": true, "data": ["data": items, "has_next": false]]
        return (try? JSONSerialization.data(withJSONObject: body)) ?? Data()
    }

    static func tile(named name: String) -> Data {
        let palette: [UInt32] = [0xff7eb6, 0x4ee6c5, 0x7aa7ff, 0xffd166, 0xff5d6c, 0x4ee68a, 0xffb547, 0x8e4ec6]
        let index = abs(name.hashValue) % palette.count
        let size = CGSize(width: 240, height: 160)
        #if os(iOS)
        let image = UIGraphicsImageRenderer(size: size).image { ctx in
            UIColor(rgb: palette[index]).setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
            let label = name.split(separator: "-").first.map(String.init) ?? name
            let attrs: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 34, weight: .heavy), .foregroundColor: UIColor(rgb: 0x1b1530)]
            let text = NSAttributedString(string: label.uppercased(), attributes: attrs)
            let bounds = text.size()
            text.draw(at: CGPoint(x: (size.width - bounds.width) / 2, y: (size.height - bounds.height) / 2))
        }
        return image.pngData() ?? Data()
        #else
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor(rgb: palette[index]).setFill()
        CGRect(origin: .zero, size: size).fill()
        image.unlockFocus()
        return image.tiffRepresentation ?? Data()
        #endif
    }
}
#endif
