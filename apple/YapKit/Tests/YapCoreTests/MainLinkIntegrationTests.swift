import Foundation
import Testing
@testable import YapCore

/// Resetting the main link changes a server's key, which would trip up
/// every other suite connecting meanwhile, so these run against a server of
/// their own (at the end the admin API puts YAP_KEY back):
///   YAP_TEST_RESET_URL=http://127.0.0.1:8799 (key YAP_TEST_KEY, admin YAP_TEST_ADMIN)
enum ResetServer {
    static let env = ProcessInfo.processInfo.environment
    static var configured: Bool { env["YAP_TEST_RESET_URL"] != nil && env["YAP_TEST_ADMIN"] != nil }
    static var baseURL: URL { URL(string: env["YAP_TEST_RESET_URL"] ?? "http://127.0.0.1:8799")! }
    static var endpoint: ServerEndpoint { ServerEndpoint(baseURL: baseURL, key: env["YAP_TEST_KEY"] ?? "") }

    static func admin(_ path: String, query: [String: String] = [:], method: String = "POST") async throws {
        var c = URLComponents(url: baseURL.appending(path: path), resolvingAgainstBaseURL: false)!
        c.queryItems = query.isEmpty ? nil : query.map { URLQueryItem(name: $0.key, value: $0.value) }
        var request = URLRequest(url: c.url!)
        request.httpMethod = method
        request.setValue("Bearer \(env["YAP_TEST_ADMIN"] ?? "")", forHTTPHeaderField: "Authorization")
        let (_, response) = try await URLSession.shared.data(for: request)
        #expect((response as? HTTPURLResponse)?.statusCode == 200, "admin \(path)")
    }

    /// Sign up on the throwaway server and get let in (or be its first person, the owner).
    @MainActor
    static func person(_ name: String, role: String = "member", key: String? = nil) async throws -> (YapClient, MemoryCredentialStore) {
        var endpoint = Self.endpoint
        if let key { endpoint.key = key }
        let store = MemoryCredentialStore(endpoint: endpoint)
        let client = YapClient(credentials: store, client: "test-ios")
        client.signUp(name: name, pin: TestServer.password, color: "#5b6cff")
        try await waitUntil("\(name) signs up") {
            if case .waiting = client.phase { return true }
            return client.phase == .online
        }
        if client.phase != .online {
            try await admin("/api/admin/role", query: ["uid": client.uid, "role": "member"])
            try await waitUntil("\(name) let in") { client.phase == .online }
        }
        if client.myRole.rawValue != role {
            try await admin("/api/admin/role", query: ["uid": client.uid, "role": role])
            try await waitUntil("\(name) is \(role)") { client.myRole.rawValue == role }
        }
        return (client, store)
    }
}

@MainActor
@Suite("Resetting the main link (throwaway server)", .serialized, .enabled(if: ResetServer.configured))
struct MainLinkIntegrationTests {
    @Test func resetKeepsEveryoneSignedIn() async throws {
        let old = ResetServer.endpoint
        try #require(try await ServerAPI(endpoint: old).check().ok, "YAP_TEST_RESET_URL opens with YAP_TEST_KEY")
        let run = TestServer.run
        let (owner, _) = try await ResetServer.person("Rowan\(run)", role: "owner")
        let (ben, benStore) = try await ResetServer.person("Rben\(run)")
        let (mod, _) = try await ResetServer.person("Rmod\(run)", role: "mod")

        // Ben's other device, offline during the reset: it keeps the old link.
        let (laptop, laptopStore) = device(endpoint: old)
        laptop.logIn(name: ben.me?.name ?? "", pin: TestServer.password)
        try await waitUntil("Ben's laptop online") { laptop.phase == .online }
        let laptopToken = try #require(laptopStore.token)
        laptop.connection?.stop()

        // A personal link made before the reset.
        owner.createInvite(label: "for Dee")
        try await waitUntil("the personal link") { owner.createdInvite != nil }
        let personal = try #require(ServerEndpoint.parse(invite: owner.createdInvite!.link))

        // A mod can't reset it: only the owner.
        mod.resetMainLink()
        try await waitUntil("the mod is told no") { mod.notice != nil }
        #expect(mod.newMainLink == nil && mod.endpoint?.key == old.key)

        // The owner resets: the new link comes back to copy, and everyone
        // connected saves it as their link.
        owner.resetMainLink()
        try await waitUntil("the new link") { owner.newMainLink != nil }
        let newKey = try #require(owner.endpoint?.key)
        #expect(newKey != old.key && !newKey.isEmpty)
        #expect(owner.newMainLink == ServerEndpoint(baseURL: old.baseURL, key: newKey).inviteLink)
        try await waitUntil("Ben's app saved the new key") { benStore.endpoint?.key == newKey }
        #expect(ben.newMainLink == nil, "only whoever reset it sees the link")
        try await waitUntil("the mod's app too") { mod.endpoint?.key == newKey }

        // HTTP: the old key is turned away, the new one and personal links work.
        #expect(try await ServerAPI(endpoint: old).check().ok == false)
        #expect(try await ServerAPI(endpoint: ServerEndpoint(baseURL: old.baseURL, key: newKey)).check().ok)
        #expect(try await ServerAPI(endpoint: personal).check().ok, "personal invite links keep working")
        await #expect(throws: ServerAPIError.badKey) { _ = try await ServerAPI(endpoint: old).config() }

        // Ben's laptop comes back with the old link and its token: it gets
        // in and takes the current key from ready.
        let (back, backStore) = device(endpoint: old, token: laptopToken)
        #expect(back.signsInWithToken(at: old), "the invite screen lets it through despite /api/check")
        back.start()
        try await waitUntil("the laptop back online with the old link") { back.phase == .online }
        #expect(backStore.endpoint?.key == newKey && back.endpoint?.key == newKey)

        // A reconnect after the reset uses the new key (retargeted).
        ben.dropConnectionForTesting()
        try await waitUntil("Ben reconnects") { ben.phase == .online && ben.isConnected }
        #expect(ben.connection?.protocols == ["yap", "key.\(newKey)"])

        // Signing in with the old link and no token: 4003, wrong key for real.
        let (stranger, _) = device(endpoint: old)
        stranger.logIn(name: ben.me?.name ?? "", pin: TestServer.password)
        try await waitUntil("turned away with 4003") { stranger.phase == .needsInvite }
        #expect(stranger.authError?.code == "bad_key")
        // An old link with a token that doesn't work: the same.
        let (revoked, _) = device(endpoint: old, token: "not-a-real-token")
        revoked.start()
        try await waitUntil("a bad token on the old key: 4003") { revoked.phase == .needsInvite }

        // The mod log says so.
        owner.refreshModLog()
        try await waitUntil("the mod log") { owner.modLog?.contains { $0.action == "invite.resetMain" } == true }
        let entry = try #require(owner.modLog?.first { $0.action == "invite.resetMain" })
        #expect(entry.actor == owner.uid && entry.phrase { owner.name(of: $0) } == "reset the main link")

        // Someone new with the new link reaches the waiting room.
        let newcomer = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: old.baseURL, key: newKey)), client: "test-ios")
        newcomer.signUp(name: "Rdee\(run)", pin: TestServer.password, color: "#30a46c")
        try await waitUntil("the newcomer waits") { if case .waiting = newcomer.phase { return true } else { return false } }

        for client in [owner, ben, mod, back, newcomer] { client.signOut() }
        // Back to YAP_KEY (the admin API), so the next run starts the same.
        try await ResetServer.admin("/api/admin/main-key", method: "DELETE")
        #expect(try await ServerAPI(endpoint: old).check().ok)
    }
}

/// A device on the given link (the dev server's, unless said otherwise).
@MainActor
func device(endpoint: ServerEndpoint, token: String? = nil) -> (YapClient, MemoryCredentialStore) {
    let store = MemoryCredentialStore(endpoint: endpoint, token: token)
    return (YapClient(credentials: store, client: "test-ios"), store)
}
