#if os(iOS)
import Foundation
import Testing
@testable import YapCore
@testable import YapVoice

/// Native ↔ web voice, for real: this client (on the iOS simulator, with a
/// synthetic mic so no hardware is touched) and the web client in headless
/// Chromium (Tools/web-voice-peer.mjs) join the same voice channel on a local
/// dev server and must reach `connected` with Opus flowing both ways, keep
/// the call when either side's socket drops, and watch each other's screen
/// (synthetic screens on both sides; H.264 decoded both ways).
/// Run by Tools/run-voice-interop.sh, which starts the web peer first and
/// passes these through TEST_RUNNER_*:
///   YAP_TEST_URL, YAP_TEST_KEY, YAP_TEST_ADMIN, YAP_PEER_STATUS (the web peer's JSON)
enum VoiceTestEnv {
    static let env = ProcessInfo.processInfo.environment
    static var configured: Bool { env["YAP_PEER_STATUS"] != nil && env["YAP_TEST_URL"] != nil }
    static var endpoint: ServerEndpoint { ServerEndpoint(baseURL: URL(string: env["YAP_TEST_URL"]!)!, key: env["YAP_TEST_KEY"] ?? "") }
    static var statusURL: URL { URL(fileURLWithPath: env["YAP_PEER_STATUS"]!) }
    static let label = "yap-voice-test"
    static let run = String(UUID().uuidString.prefix(4)).lowercased()

    struct WebStatus: Decodable {
        struct Scenario: Decodable {
            var connected: Bool?
            var maxBytesReceived: Double?
            var maxPacketsReceived: Int?
            var maxLevel: Double?
            var mime: String?
            var fmtp: String?
            var state: String?
            var remoteDescriptionHasTunedOpus: Bool?
            var latest: Latest?
            /// Gaps over 100 ms in the audio the web receives: [epoch ms, length ms].
            var gaps: [[Double]]?
            /// RTCPeerConnections the web had with us in this scenario.
            var connections: Int?
            var chimes: Int?
            var webDropAt: Double?
            // Screen sharing (webShares / nativeShares).
            var sharing: Bool?
            var watchedBy: String?
            var sentFrames: Int?
            var sentMime: String?
            var stoppedWatching: Bool?
            var watching: Bool?
            var framesDecoded: Int?
            var frameWidth: Int?
            var frameHeight: Int?
            var stopped: Bool?
        }
        struct Latest: Decodable {
            var level: Double
            var mutedFlag: Bool
            var speaking: Bool
            var bytesReceived: Double
        }
        var phase: String
        var sid: String?
        var uid: String?
        var name: String
        var scenarios: [String: Scenario]
        var error: String?
    }

    static func webStatus() -> WebStatus? {
        guard let data = try? Data(contentsOf: statusURL) else { return nil }
        return try? JSONDecoder().decode(WebStatus.self, from: data)
    }

    static func admin(_ path: String, query: [String: String], method: String = "POST") async throws -> Data {
        var c = URLComponents(url: endpoint.baseURL.appending(path: path), resolvingAgainstBaseURL: false)!
        c.queryItems = query.map { URLQueryItem(name: $0.key, value: $0.value) }
        var request = URLRequest(url: c.url!)
        request.httpMethod = method
        request.setValue("Bearer \(env["YAP_TEST_ADMIN"] ?? "")", forHTTPHeaderField: "Authorization")
        return try await URLSession.shared.data(for: request).0
    }

    static func letIn(_ name: String) async throws {
        let data = try await admin("/api/admin/export", query: ["after": "0"], method: "GET")
        let users = (try JSONSerialization.jsonObject(with: data) as? [String: Any])?["users"] as? [[String: Any]] ?? []
        guard let uid = users.first(where: { ($0["name"] as? String) == name })?["id"] as? String else { throw URLError(.fileDoesNotExist) }
        _ = try await admin("/api/admin/role", query: ["uid": uid, "role": "member"])
    }
}

@MainActor
private func waitUntil(_ what: String, timeout: Duration = .seconds(20), _ condition: () -> Bool) async throws {
    let deadline = ContinuousClock.now + timeout
    while !condition() {
        if ContinuousClock.now > deadline {
            Issue.record("timed out waiting for: \(what)")
            throw CancellationError()
        }
        try await Task.sleep(for: .milliseconds(50))
    }
}

/// The longest stretch without a new packet from anyone in the call, sampled every 20 ms.
@MainActor
final class PacketGaps {
    let engine: VoiceEngine
    private var task: Task<Void, Never>?
    private(set) var longest: Duration = .zero

    init(engine: VoiceEngine) {
        self.engine = engine
        task = Task { [weak self] in
            var last = -1
            var lastAt = ContinuousClock.now
            while !Task.isCancelled, let self {
                let total = await self.engine.peerStats().reduce(0) { $0 + $1.packetsReceived }
                let now = ContinuousClock.now
                if total != last {
                    if last >= 0 { self.longest = max(self.longest, now - lastAt) }
                    last = total
                    lastAt = now
                }
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    func stop() { task?.cancel() }
}

@MainActor
@Suite("Voice interop with the web client", .serialized, .enabled(if: VoiceTestEnv.configured))
final class VoiceInteropTests {
    let device = SyntheticAudioDevice()
    let client: YapClient
    let voice: VoiceController

    init() async throws {
        client = YapClient(credentials: MemoryCredentialStore(endpoint: VoiceTestEnv.endpoint), client: VoiceTestEnv.label)
        let defaults = UserDefaults(suiteName: "yap-voice-test-\(VoiceTestEnv.run)")!
        voice = VoiceController(client: client, engine: VoiceEngine(audioDevice: device), defaults: defaults, requestsMicPermission: false)
        voice.settings.mode = .openMic
        let name = "Sim \(VoiceTestEnv.run) \(Int.random(in: 100...999))"
        client.signUp(name: name, pin: "582039", color: "#0090ff")
        try await waitUntil("signed up") { if case .waiting = client.phase { true } else { client.phase == .online } }
        if case .waiting = client.phase {
            try await VoiceTestEnv.letIn(name)
            try await waitUntil("let in") { client.phase == .online }
        }
    }

    /// Both scenarios, in order (the web peer plays them in this order too).
    @Test func nativeAndWebInterop() async throws {
        try await nativeOffersToWeb()
        try await webOffersToNative()
        try await watchTheWebsScreen()
        try await shareOurScreenWithTheWeb()
    }

    /// PROTOCOL.md › Screen sharing, web → native: the web shares (its own
    /// `startScreen`, a synthetic canvas + tone); we watch from outside the
    /// channel (Watch screen joins it), decode H.264 and hear its sound, then
    /// stop watching and the web stops sending to us.
    func watchTheWebsScreen() async throws {
        let webSid = try #require(VoiceTestEnv.webStatus()?.sid)
        try await waitUntil("the web's LIVE flag", timeout: .seconds(20)) { client.sessions[webSid]?.voice?.screen == true }
        voice.settings.mode = .openMic
        await voice.watchScreen(webSid)
        try await waitUntil("joined Lounge and connected", timeout: .seconds(30)) {
            voice.channelId == "lounge" && voice.engine.connectionState(of: webSid) == .connected
        }
        try await waitUntil("the web's screen plays", timeout: .seconds(30)) {
            voice.watch?.phase == .playing && voice.screenRenderer.counters.frames >= 30
        }
        try await waitUntil("its sound comes too", timeout: .seconds(10)) { voice.watchedHasSound }
        try await Task.sleep(for: .seconds(2))
        let stats = try #require(await voice.engine.peerStats().first)
        let web = VoiceTestEnv.webStatus()?.scenarios["webShares"]
        print("SCREEN web → native: decoded \(stats.videoFramesDecoded) frames \(stats.videoFrameWidth)x\(stats.videoFrameHeight) \(stats.inboundVideoCodec ?? "?") [\(stats.inboundVideoFmtp ?? "")], rendered \(voice.screenRenderer.counters.frames), share sound \(Int(stats.screenAudioBytesReceived)) B; web sent \(web?.sentFrames ?? -1) frames \(web?.sentMime ?? "?") to \(web?.watchedBy ?? "?")")
        #expect(stats.inboundVideoCodec == "video/H264", "H.264, the common codec")
        #expect(stats.inboundVideoFmtp?.contains("packetization-mode=1") == true)
        #expect(stats.videoFramesDecoded >= 30)
        #expect(stats.screenAudioBytesReceived > 0, "the share's sound rides on its own track")
        #expect(web?.watchedBy == client.mySid, "the web sends its screen to us, the one who asked")
        #expect(voice.watch?.status(name: "Webby") == nil, "playing: no line over the picture")

        voice.setScreenVolume(0.5, for: voice.watch?.uid ?? "")
        voice.stopWatching()
        #expect(voice.watch == nil)
        try await waitUntil("the web stopped sending to us", timeout: .seconds(15)) {
            VoiceTestEnv.webStatus()?.scenarios["webShares"]?.stoppedWatching == true
        }
        try await waitUntil("its video left our connection", timeout: .seconds(10)) { voice.engine.remoteScreens[webSid] == nil }
        voice.leave()
    }

    /// Native → web: we share a synthetic screen; the web clicks our LIVE
    /// badge, decodes H.264, and Stop watching takes it off our list and
    /// stops our encoding for it.
    func shareOurScreenWithTheWeb() async throws {
        try await waitUntil("the web waiting for our share", timeout: .seconds(60)) { VoiceTestEnv.webStatus()?.phase == "waiting-for-native-share" }
        let status = try #require(VoiceTestEnv.webStatus())
        let webSid = try #require(status.sid)
        let webUid = try #require(status.uid)
        await voice.join("lounge")
        try await waitUntil("connected to the web", timeout: .seconds(30)) { voice.engine.connectionState(of: webSid) == .connected }
        let feeder = SyntheticScreenFeeder(width: 640, height: 360, fps: 30)
        try await voice.startSharing(feeder, quality: .smooth)
        #expect(voice.sharing == .smooth)
        #expect(voice.screenWatchers.isEmpty && !voice.engine.sendsScreen(to: webSid), "nobody asked yet: nothing sent")
        try await waitUntil("the web asked for it", timeout: .seconds(20)) { voice.screenWatchers == [webUid] }
        let name = client.user(webUid)?.name ?? "?"
        #expect(ScreenAudience.text(names: voice.screenWatchers.map { client.user($0)?.name ?? "?" }) == "Watching: \(name)")
        try await waitUntil("the web decodes our screen", timeout: .seconds(30)) {
            (VoiceTestEnv.webStatus()?.scenarios["nativeShares"]?.framesDecoded ?? 0) >= 60
        }
        let web = try #require(VoiceTestEnv.webStatus()?.scenarios["nativeShares"])
        let sent = try #require(await voice.engine.peerStats().first)
        print("SCREEN native → web: web decoded \(web.framesDecoded ?? 0) frames \(web.frameWidth ?? 0)x\(web.frameHeight ?? 0) \(web.mime ?? "?") [\(web.fmtp ?? "")]; we encoded \(sent.videoFramesEncoded) with \(sent.videoEncoder ?? "?")")
        #expect(web.mime == "video/H264")
        #expect(web.fmtp?.contains("packetization-mode=1") == true)
        #expect(web.fmtp?.contains("profile-level-id=42e01f") == true, "constrained baseline")
        #expect((web.frameWidth ?? 0) > 0 && (web.frameWidth ?? 0) <= 640)

        // Quality changes while sharing, without stopping.
        voice.setShareQuality(.sharp)
        #expect(voice.sharing == .sharp && voice.engine.sendsScreen(to: webSid))

        try await waitUntil("the web stopped watching", timeout: .seconds(20)) {
            voice.screenWatchers.isEmpty && !voice.engine.sendsScreen(to: webSid)
        }
        voice.stopSharing()
        #expect(voice.sharing == nil)
        voice.leave()
        try await waitUntil("web done", timeout: .seconds(20)) { VoiceTestEnv.webStatus()?.phase == "done" }
    }

    /// Native joins second and offers; then mute shows on the web side and silences audio.
    func nativeOffersToWeb() async throws {
        try await waitUntil("web peer sitting in Lounge", timeout: .seconds(90)) { VoiceTestEnv.webStatus()?.phase == "web-first" }
        let web = try #require(VoiceTestEnv.webStatus())
        let webSid = try #require(web.sid)
        try await waitUntil("web session visible") { client.sessions[webSid]?.voice?.channelId == "lounge" }

        await voice.join("lounge")
        try await waitUntil("own voice echo") { voice.channelId == "lounge" }
        #expect(voice.engine.peerCount == 1, "offered to the one person already there")
        try await waitUntil("connected to the web peer", timeout: .seconds(30)) { voice.engine.connectionState(of: webSid) == .connected }

        // Let the bitrate ramp up, then measure both directions.
        try await Task.sleep(for: .seconds(3))
        let s1 = try #require(await voice.engine.peerStats().first)
        try await Task.sleep(for: .seconds(4))
        let s2 = try #require(await voice.engine.peerStats().first)
        let receiveKbps = (s2.bytesReceived - s1.bytesReceived) * 8 / 4000
        let sendKbps = (s2.bytesSent - s1.bytesSent) * 8 / 4000
        print("VOICE nativeOffersToWeb: native received \(Int(receiveKbps)) kbps, sent \(Int(sendKbps)) kbps, inbound \(s2.inboundCodec ?? "?") [\(s2.inboundFmtp ?? "")], outbound fmtp [\(s2.outboundFmtp ?? "")], rtt \(s2.rttMs ?? -1) ms, played frames \(device.playedFrames), playout RMS \(device.recentPlayoutRMS), local level \(s2.localAudioLevel)")
        #expect(s2.inboundCodec == "audio/opus")
        #expect(s2.outboundCodec == "audio/opus")
        #expect(s2.packetsReceived > 100)
        #expect(receiveKbps > 128 * 0.6, "web → native at ~128 kbps: \(receiveKbps)")
        #expect(sendKbps > 128 * 0.6, "native → web at ~128 kbps: \(sendKbps)")
        #expect(s2.outboundFmtp?.contains("maxaveragebitrate=128000") == true || s2.inboundFmtp?.contains("maxaveragebitrate=128000") == true)
        #expect(device.recentPlayoutRMS > 0.001, "decoded audio actually reaches the speaker")
        try await waitUntil("web speaking ring visible to native") { voice.engine.speaking.contains(webSid) }

        let webSide = try #require(VoiceTestEnv.webStatus()?.scenarios["webFirst"])
        print("VOICE web side (webFirst): connected=\(webSide.connected ?? false) bytes=\(webSide.maxBytesReceived ?? 0) packets=\(webSide.maxPacketsReceived ?? 0) level=\(webSide.maxLevel ?? 0) \(webSide.mime ?? "?") fmtp=\(webSide.fmtp ?? "")")
        #expect(webSide.connected == true)
        #expect(webSide.mime == "audio/opus")
        #expect((webSide.maxPacketsReceived ?? 0) > 100)
        #expect((webSide.maxLevel ?? 0) > 0.01, "the web hears the native synthetic mic")
        #expect(webSide.remoteDescriptionHasTunedOpus == true, "native's SDP carries the Opus tuning")

        // Mute: the web sees the flag and the level drops.
        voice.toggleMute()
        try await waitUntil("web sees mute flag and silence", timeout: .seconds(8)) {
            guard let latest = VoiceTestEnv.webStatus()?.scenarios["webFirst"]?.latest else { return false }
            return latest.mutedFlag && latest.level < 0.001
        }
        // Voice activity measures the mic before the mute gate (so it can reopen).
        let muted = try #require(await voice.engine.peerStats().first)
        print("VOICE local level while muted: \(muted.localAudioLevel)")
        voice.toggleMute()

        voice.leave()
        #expect(voice.engine.peerCount == 0)
        try await waitUntil("web saw us leave", timeout: .seconds(15)) { VoiceTestEnv.webStatus()?.phase == "waiting-for-native" }
    }

    /// Native sits in Lounge first; the web joins and offers; native answers.
    /// Uses voice-activity mode, so audio only flows if the gate opens.
    func webOffersToNative() async throws {
        try await waitUntil("web waiting for us", timeout: .seconds(90)) {
            let phase = VoiceTestEnv.webStatus()?.phase
            return phase == "waiting-for-native" || phase == "native-first"
        }
        voice.settings.mode = .voiceActivity
        voice.settings.vadThreshold = VoiceSettings.defaultVadThreshold
        await voice.join("lounge")
        try await waitUntil("in Lounge") { voice.channelId == "lounge" }
        #expect(voice.engine.peerCount == 0, "alone: nobody to offer to")
        // Alone, the mic is still measured (the loopback monitor): the meter
        // moves and voice activity opens on the chord.
        try await waitUntil("mic level while alone", timeout: .seconds(10)) { voice.engine.localLevel > 0.01 && voice.engine.vadOpen }
        print("VOICE alone: local level \(voice.engine.localLevel) (\(Int(VoiceGate.dB(voice.engine.localLevel))) dBFS), vadOpen=\(voice.engine.vadOpen)")
        try await waitUntil("web offered and we connected", timeout: .seconds(45)) {
            voice.engine.peerStates.values.contains(.connected)
        }
        let webSid = try #require(voice.engine.peerStates.first { $0.value == .connected }?.key)
        try await Task.sleep(for: .seconds(6))
        let stats = try #require(await voice.engine.peerStats().first)
        print("VOICE webOffersToNative: native received \(stats.bytesReceived) B, sent \(stats.bytesSent) B, vadOpen=\(voice.engine.vadOpen) local level \(stats.localAudioLevel), inbound fmtp [\(stats.inboundFmtp ?? "")]")
        #expect(stats.packetsReceived > 100)
        #expect(voice.engine.vadOpen, "voice activity opened on the synthetic chord")
        try await waitUntil("self speaking ring") { voice.engine.speaking.contains(client.mySid ?? "-") }
        try await waitUntil("web hears native", timeout: .seconds(10)) {
            (VoiceTestEnv.webStatus()?.scenarios["nativeFirst"]?.maxLevel ?? 0) > 0.01
        }
        let webSide = try #require(VoiceTestEnv.webStatus()?.scenarios["nativeFirst"])
        print("VOICE web side (nativeFirst): connected=\(webSide.connected ?? false) packets=\(webSide.maxPacketsReceived ?? 0) level=\(webSide.maxLevel ?? 0) fmtp=\(webSide.fmtp ?? "")")
        #expect(webSide.connected == true)
        #expect((webSide.maxPacketsReceived ?? 0) > 100)

        try await callSurvivesDroppedSockets(webSid: webSid)

        // Silence closes the gate again.
        device.silent.withLock { $0 = true }
        try await waitUntil("gate closes in silence", timeout: .seconds(5)) { !voice.engine.vadOpen }
        device.silent.withLock { $0 = false }

        voice.leave()
        try await waitUntil("web sharing next", timeout: .seconds(20)) { VoiceTestEnv.webStatus()?.phase == "web-sharing" }
    }

    /// PROTOCOL.md › Voice › Keeping calls across reconnects: our socket
    /// drops, then the web's (it waits 3 s after our resume). Both times the
    /// same connection carries on under the new session ids: no gap, no chimes.
    func callSurvivesDroppedSockets(webSid: String) async throws {
        var events: [VoiceEvent] = []
        voice.onEvent = { events.append($0) }
        let gaps = PacketGaps(engine: voice.engine)
        let webChimes = VoiceTestEnv.webStatus()?.scenarios["nativeFirst"]?.chimes ?? 0
        let dropAt = Date().timeIntervalSince1970 * 1000
        let ourSid = client.mySid
        client.dropConnectionForTesting()
        try await waitUntil("our call handed over to our new session", timeout: .seconds(15)) {
            client.mySid != nil && client.mySid != ourSid && voice.channelId == "lounge" && !voice.isResuming
                && voice.engine.connectionState(of: webSid) == .connected
        }
        #expect(voice.engine.peerCount == 1, "the same connection, nobody new")
        try await waitUntil("the web dropped its socket and is back", timeout: .seconds(20)) {
            guard let status = VoiceTestEnv.webStatus(), status.scenarios["nativeFirst"]?.webDropAt != nil,
                  let now = status.sid, now != webSid else { return false }
            return voice.engine.connectionState(of: now) == .connected && client.sessions[now]?.voice?.channelId == "lounge"
        }
        let webNow = try #require(VoiceTestEnv.webStatus()?.sid)
        #expect(voice.engine.peerCount == 1 && voice.engine.connectionState(of: webSid) == nil, "moved to their new session")
        #expect(voice.hint(for: webNow) == nil, "not reconnecting any more")
        try await Task.sleep(for: .seconds(2))
        gaps.stop()
        let web = try #require(VoiceTestEnv.webStatus()?.scenarios["nativeFirst"])
        let webGaps = (web.gaps ?? []).filter { $0[0] >= dropAt - 1000 }.map { Int($0[1]) }
        print("VOICE dropped sockets: native's longest gap \(gaps.longest), web's gaps over 100 ms \(webGaps), web connections \(web.connections ?? -1), chimes native \(events) web \((web.chimes ?? 0) - webChimes)")
        #expect(gaps.longest < .milliseconds(300), "the web's audio kept coming: \(gaps.longest)")
        #expect(webGaps.allSatisfy { $0 < 300 }, "ours kept reaching the web: \(webGaps)")
        #expect(web.connections == 1, "the web kept its one connection with us")
        #expect(events.isEmpty, "no sounds on our side: \(events)")
        #expect((web.chimes ?? 0) == webChimes, "no sounds on the web's side")
        voice.onEvent = nil
    }
}
#endif
