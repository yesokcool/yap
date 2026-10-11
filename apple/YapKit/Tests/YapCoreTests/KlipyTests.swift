import Foundation
import Testing
@testable import YapCore

/// A fake KLIPY at the network layer (like the web e2e test's Playwright route).
final class FakeKlipy: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var log: [String] = []
    nonisolated(unsafe) static var status = 200
    static let lock = NSLock()

    static func session() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [FakeKlipy.self]
        return URLSession(configuration: config)
    }

    override class func canInit(with request: URLRequest) -> Bool { request.url?.host() == "api.klipy.com" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let url = request.url!
        var path = url.path()
        // Drop /api/v1/<appKey>.
        path = "/" + path.split(separator: "/").dropFirst(3).joined(separator: "/")
        Self.lock.withLock { Self.log.append("\(request.httpMethod ?? "GET") \(path)?\(url.query() ?? "")") }
        let body: Data
        if request.httpMethod == "POST" {
            body = Data(#"{"result":true}"#.utf8)
        } else {
            let q = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "q" }?.value ?? "trending"
            let items = (0..<3).map { i in
                """
                {"id":\(i),"slug":"\(q)-\(i)","title":"\(q) \(i)","type":"gif","file":{
                  "md":{"mp4":{"url":"https://static.klipy.com/t/\(q)-\(i).mp4","width":498,"height":280,"size":1},
                        "webp":{"url":"https://static.klipy.com/t/\(q)-\(i).webp","width":498,"height":280,"size":1},
                        "gif":{"url":"https://static.klipy.com/t/\(q)-\(i).gif","width":498,"height":280,"size":1},
                        "jpg":{"url":"https://static1.klipy.com/t/\(q)-\(i).jpg","width":498,"height":280,"size":1}},
                  "sm":{"mp4":{"url":"https://static.klipy.com/t/sm.mp4","width":200,"height":112}}}}
                """
            }
            body = Data(#"{"result":true,"data":{"data":[\#(items.joined(separator: ",")),{"slug":"broken"}],"current_page":1,"per_page":24,"has_next":true}}"#.utf8)
        }
        let response = HTTPURLResponse(url: url, statusCode: Self.status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

@Suite("KLIPY", .serialized)
struct KlipyTests {
    @Test func customerIdIsStableAndAnonymous() {
        let id = KlipyClient.customerId(for: "u1")
        #expect(id.count == 32)
        #expect(id == KlipyClient.customerId(for: "u1"))
        #expect(id != KlipyClient.customerId(for: "u2"))
        // SHA-256("yap:u1"), first 16 bytes — same as the web client.
        #expect(id == "434a711be9c675103e01a5b63dd439ea")
    }

    @Test func trendingSearchRecentAndShare() async throws {
        FakeKlipy.log = []
        FakeKlipy.status = 200
        let klipy = KlipyClient(appKey: "k", uid: "u1", locale: Locale(identifier: "en_US"), session: FakeKlipy.session())
        let trending = try await klipy.page(.trending)
        #expect(trending.gifs.map(\.slug) == ["trending-0", "trending-1", "trending-2"], "KLIPY's order, broken items dropped")
        #expect(trending.hasNext)
        let first = try #require(trending.gifs.first)
        #expect(first.mp4 == "https://static.klipy.com/t/trending-0.mp4")
        #expect(first.still == "https://static1.klipy.com/t/trending-0.jpg")
        #expect(first.width == 498 && first.height == 280)

        let cats = try await klipy.page(.search, query: "cats", page: 2)
        #expect(cats.gifs.first?.title == "cats 0")
        _ = try await klipy.page(.recent)
        await klipy.reportShare(slug: "cats-0", query: "cats")

        let log = FakeKlipy.log
        #expect(log.contains { $0.hasPrefix("GET /gifs/trending?") && $0.contains("customer_id=\(klipy.customerId)") && $0.contains("content_filter=medium") && $0.contains("locale=us") })
        #expect(log.contains { $0.hasPrefix("GET /gifs/search?") && $0.contains("q=cats") && $0.contains("page=2") })
        #expect(log.contains { $0 == "GET /gifs/recent/\(klipy.customerId)?page=1&per_page=24" })
        #expect(log.contains { $0.hasPrefix("POST /gifs/share/cats-0") })
    }

    @Test func rateLimitAndMissingKey() async {
        FakeKlipy.status = 429
        defer { FakeKlipy.status = 200 }
        let klipy = KlipyClient(appKey: "k", uid: "u1", session: FakeKlipy.session())
        await #expect(throws: KlipyClient.Failure.rateLimited) { try await klipy.page(.trending) }
        let none = KlipyClient(appKey: nil, uid: "u1", session: FakeKlipy.session())
        await #expect(throws: KlipyClient.Failure.notConfigured) { try await none.page(.trending) }
    }
}
