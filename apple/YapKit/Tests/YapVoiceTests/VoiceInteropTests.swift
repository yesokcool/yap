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
///   YAP_TEST_URL, YAP_TEST_KEY, YAP_TEST_ADMIN, YAP_PEER_STATUS (the web peer's JSON);
///   YAP_RED=0 (redundancy off, to compare), YAP_IMPAIR (the proxy's profile, for the log),
///   YAP_MEASURE_SECONDS (how long each RED measurement runs, default 6),
///   YAP_INTEROP_SCENARIOS=voice (only the two voice scenarios).
enum VoiceTestEnv {
    static let env = ProcessInfo.processInfo.environment
    static var configured: Bool { env["YAP_PEER_STATUS"] != nil && env["YAP_TEST_URL"] != nil }
    static var redundancy: Bool { env["YAP_RED"] != "0" }
    static var impair: String { env["YAP_IMPAIR"].flatMap { $0.isEmpty ? nil : $0 } ?? "none" }
    static var measureSeconds: Double { env["YAP_MEASURE_SECONDS"].flatMap(Double.init) ?? 6 }
    static var voiceOnly: Bool { env["YAP_INTEROP_SCENARIOS"] == "voice" }
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
            // What RED shows up in (the voice's inbound and outbound stream).
            var packetsReceived: Double?
            var packetsLost: Double?
            var packetsDiscarded: Double?
            var bytesSent: Double?
            var packetsSent: Double?
            var concealedSamples: Double?
            var silentConcealedSamples: Double?
            var totalSamplesReceived: Double?
            var fecPacketsReceived: Double?
            var fecPacketsDiscarded: Double?
            var jitterBufferDelay: Double?
            var jitterBufferEmittedCount: Double?
            var mime: String?
            var outMime: String?
            var targetBitrate: Double?
            var localFirstCodec: String?
            var remoteFirstCodec: String?
        }
        var phase: String
        var at: Double?
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

/// One direction of a call over a measurement window (PROTOCOL.md › Voice › Redundancy).
struct CallDirection: CustomStringConvertible {
    var kbps: Double
    var bytesPerPacket: Double
    /// At the receiver: packets lost (% of expected) and audio patched over (% of samples).
    var lostPct: Double
    var patchedPct: Double
    /// Packets the receiver's jitter buffer threw away per packet received:
    /// ~1 with RED (a copy of every frame that already came), ~0 without.
    var copiesPerPacket: Double
    /// libwebrtc's "secondary" packets used (FEC, RED where it counts them).
    var secondaryUsed: Double
    var jitterBufferMs: Double

    var description: String {
        String(format: "%.0f kbps, %.0f B/packet, lost %.2f %%, patched %.2f %%, copies/packet %.2f, secondary used %.0f, jitter buffer %.0f ms",
               kbps, bytesPerPacket, lostPct, patchedPct, copiesPerPacket, secondaryUsed, jitterBufferMs)
    }

    /// RED is going this way: about twice the bytes of a 128 kbps Opus packet
    /// (~320 B), and the copies arriving.
    var carriesRed: Bool { bytesPerPacket > 480 && copiesPerPacket > 0.8 }
    var plainOpus: Bool { bytesPerPacket < 480 && copiesPerPacket < 0.2 }
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
        voice.engine.redundancy = VoiceTestEnv.redundancy
        voice.settings.mode = .openMic
        let name = "Sim \(VoiceTestEnv.run) \(Int.random(in: 100...999))"
        client.signUp(name: name, pin: "correct-horse-582039", color: "#0090ff")
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
        if VoiceTestEnv.voiceOnly { return }
        try await watchTheWebsScreen()
        try await shareOurScreenWithTheWeb()
    }

    /// Both directions of the call with the web over `YAP_MEASURE_SECONDS`:
    /// what each side sends, and what arrives (RED shows as twice the bytes
    /// per packet and a discarded copy per packet; libwebrtc's stats name
    /// the codec inside, Opus, either way). Checks libwebrtc's rule: each
    /// side sends RED exactly when the *other* side's description lists it first.
    @discardableResult
    func measureRed(_ label: String, scenario: String) async throws -> (toWeb: CallDirection, toNative: CallDirection) {
        func web() -> (at: Double, latest: VoiceTestEnv.WebStatus.Latest)? {
            guard let status = VoiceTestEnv.webStatus(), let latest = status.scenarios[scenario]?.latest, let at = status.at else { return nil }
            return (at, latest)
        }
        let n1 = try #require(await voice.engine.peerStats().first)
        let w1 = try #require(web())
        try await Task.sleep(for: .seconds(VoiceTestEnv.measureSeconds))
        let n2 = try #require(await voice.engine.peerStats().first)
        let w2 = try #require(web())
        let seconds = VoiceTestEnv.measureSeconds
        let webSeconds = max(0.25, (w2.at - w1.at) / 1000)
        func d(_ a: Double?, _ b: Double?) -> Double { (b ?? 0) - (a ?? 0) }
        func pct(_ part: Double, _ whole: Double) -> Double { whole > 0 ? 100 * part / whole : 0 }

        let webIn = (received: d(w1.latest.packetsReceived, w2.latest.packetsReceived), lost: d(w1.latest.packetsLost, w2.latest.packetsLost))
        let toWeb = CallDirection(
            kbps: (n2.bytesSent - n1.bytesSent) * 8 / 1000 / seconds,
            bytesPerPacket: (n2.bytesSent - n1.bytesSent) / Double(max(1, n2.packetsSent - n1.packetsSent)),
            lostPct: pct(webIn.lost, webIn.received + webIn.lost),
            patchedPct: pct(d(w1.latest.concealedSamples, w2.latest.concealedSamples) - d(w1.latest.silentConcealedSamples, w2.latest.silentConcealedSamples),
                            d(w1.latest.totalSamplesReceived, w2.latest.totalSamplesReceived)),
            copiesPerPacket: d(w1.latest.packetsDiscarded, w2.latest.packetsDiscarded) / max(1, webIn.received),
            secondaryUsed: d(w1.latest.fecPacketsReceived, w2.latest.fecPacketsReceived) - d(w1.latest.fecPacketsDiscarded, w2.latest.fecPacketsDiscarded),
            jitterBufferMs: 1000 * d(w1.latest.jitterBufferDelay, w2.latest.jitterBufferDelay) / max(1, d(w1.latest.jitterBufferEmittedCount, w2.latest.jitterBufferEmittedCount)))
        let nativeIn = (received: Double(n2.voicePacketsReceived - n1.voicePacketsReceived), lost: Double(n2.voicePacketsLost - n1.voicePacketsLost))
        let toNative = CallDirection(
            kbps: d(w1.latest.bytesSent, w2.latest.bytesSent) * 8 / 1000 / webSeconds,
            bytesPerPacket: d(w1.latest.bytesSent, w2.latest.bytesSent) / max(1, d(w1.latest.packetsSent, w2.latest.packetsSent)),
            lostPct: pct(nativeIn.lost, nativeIn.received + nativeIn.lost),
            patchedPct: pct((n2.concealedSamples - n2.silentConcealedSamples) - (n1.concealedSamples - n1.silentConcealedSamples), n2.samplesReceived - n1.samplesReceived),
            copiesPerPacket: Double(n2.voicePacketsDiscarded - n1.voicePacketsDiscarded) / max(1, nativeIn.received),
            secondaryUsed: Double((n2.fecPacketsReceived - n1.fecPacketsReceived) - (n2.fecPacketsDiscarded - n1.fecPacketsDiscarded)),
            jitterBufferMs: 1000 * (n2.jitterBufferDelay - n1.jitterBufferDelay) / max(1, n2.jitterBufferEmitted - n1.jitterBufferEmitted))
        let webSendsRed = w2.latest.remoteFirstCodec?.lowercased() == "red"
        print("RED \(label) [impair \(VoiceTestEnv.impair), redundancy \(VoiceTestEnv.redundancy ? "on" : "off"), \(Int(seconds)) s]: native's description first \(n2.asksRed ? "red" : "opus"), web's first \(w2.latest.localFirstCodec ?? "?"); native → web (\(n2.sendsRed ? "red+opus" : "opus")): \(toWeb); web → native (\(webSendsRed ? "red+opus" : "opus")): \(toNative); stats codec native in \(n2.inboundCodec ?? "?") out \(n2.outboundCodec ?? "?"), web in \(w2.latest.mime ?? "?") out \(w2.latest.outMime ?? "?"), web target \(Int(w2.latest.targetBitrate ?? -1))")
        #expect(n2.asksRed == VoiceTestEnv.redundancy, "our description lists RED first exactly when redundancy is on")
        #expect(n2.sendsRed == (w2.latest.localFirstCodec?.lowercased() == "red"), "we send RED when the web's description lists it first")
        #expect(n2.sendsRed ? toWeb.carriesRed : toWeb.plainOpus, "native → web: \(toWeb)")
        #expect(webSendsRed == n2.asksRed, "the web follows our description")
        #expect(webSendsRed ? toNative.carriesRed : toNative.plainOpus, "web → native: \(toNative)")
        return (toWeb, toNative)
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
        // "Lower shared media while friends talk": while the web speaks, the
        // share's sound plays at half my volume for it; off, back to it.
        #expect(abs((voice.engine.screenSoundVolume(of: webSid) ?? 0) - 1) < 0.01, "my volume: 100 %")
        voice.settings.dipSharedMedia = true
        try await waitUntil("the web speaking", timeout: .seconds(10)) { voice.engine.speaking.contains(webSid) }
        try await waitUntil("the share's sound at half", timeout: .seconds(3)) {
            abs((voice.engine.screenSoundVolume(of: webSid) ?? 0) - 0.5) < 0.01
        }
        print("TALK dip: share sound at \(voice.engine.screenSoundVolume(of: webSid) ?? -1) while the web speaks, saved volume \(voice.screenVolume(for: voice.watch?.uid ?? ""))")
        #expect(voice.screenVolume(for: voice.watch?.uid ?? "") == 1, "the saved volume never changes")
        voice.settings.dipSharedMedia = false
        try await waitUntil("back to my volume", timeout: .seconds(2)) { abs((voice.engine.screenSoundVolume(of: webSid) ?? 0) - 1) < 0.01 }
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
        try await measureRed("nativeOffersToWeb", scenario: "webFirst")

        // What the web reports receiving from us (remote-inbound-rtp): our own connection.
        let own = try #require(await voice.engine.peerStats().first)
        print("OWN the web receives from us: fraction lost \(own.remoteFractionLost ?? -1), lost \(own.remotePacketsLost) of \(own.packetsSent), rtt \(own.remoteRttMs ?? -1) ms, reports \(own.remoteReports), hint \(voice.ownConnectionWeak)")
        #expect(own.remoteReports > 0 && own.remoteFractionLost != nil, "the web's reports about our voice arrive")
        if VoiceTestEnv.impair == "none" { #expect(!voice.ownConnectionWeak, "nothing lost on localhost") }

        let webSide = try #require(VoiceTestEnv.webStatus()?.scenarios["webFirst"])
        print("VOICE web side (webFirst): connected=\(webSide.connected ?? false) bytes=\(webSide.maxBytesReceived ?? 0) packets=\(webSide.maxPacketsReceived ?? 0) level=\(webSide.maxLevel ?? 0) \(webSide.mime ?? "?") fmtp=\(webSide.fmtp ?? "")")
        #expect(webSide.connected == true)
        #expect(webSide.mime == "audio/opus")
        #expect((webSide.maxPacketsReceived ?? 0) > 100)
        #expect((webSide.maxLevel ?? 0) > 0.01, "the web hears the native synthetic mic")
        #expect(webSide.remoteDescriptionHasTunedOpus == true, "native's SDP carries the Opus tuning")

        // Mute: the web sees the flag and the level drops.
        var nudges: [(at: ContinuousClock.Instant, blip: Bool, first: Bool)] = []
        voice.onEvent = { if case .talkingWhileMuted(let blip, let first) = $0 { nudges.append((.now, blip, first)) } }
        voice.toggleMute()
        try await waitUntil("web sees mute flag and silence", timeout: .seconds(8)) {
            guard let latest = VoiceTestEnv.webStatus()?.scenarios["webFirst"]?.latest else { return false }
            return latest.mutedFlag && latest.level < 0.001
        }
        // Voice activity measures the mic before the mute gate (so it can reopen).
        let muted = try #require(await voice.engine.peerStats().first)
        print("VOICE local level while muted: \(muted.localAudioLevel)")
        // …so talking while muted is seen: "You're muted" at most every 5 s, the blip only the first time.
        try await waitUntil("a second nudge", timeout: .seconds(13)) { nudges.count >= 2 }
        let gaps = zip(nudges.dropFirst(), nudges).map { ($0.at - $1.at) / .seconds(1) }
        print("TALK muted: level \(Int(VoiceGate.dB(voice.engine.localLevel))) dBFS, nudges \(nudges.count), gaps \(gaps.map { String(format: "%.2f", $0) })")
        #expect(VoiceGate.dB(voice.engine.localLevel) > -40, "the level before the mute")
        #expect(nudges.first?.first == true && nudges.dropFirst().allSatisfy { !$0.first }, "explained once")
        #expect(nudges.first?.blip == true && nudges.dropFirst().allSatisfy { !$0.blip }, "one blip per mute")
        #expect(gaps.allSatisfy { $0 >= 4.95 }, "at most every 5 s: \(gaps)")
        #expect(voice.mutedNudgeShown || nudges.count >= 2)
        voice.onEvent = nil
        voice.toggleMute()
        #expect(!voice.mutedNudgeShown, "unmuting takes \"You're muted\" away")

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
        try await measureRed("webOffersToNative", scenario: "nativeFirst")

        try await callSurvivesDroppedSockets(webSid: webSid)

        // Silence closes the gate again.
        device.silent.withLock { $0 = true }
        try await waitUntil("gate closes in silence", timeout: .seconds(5)) { !voice.engine.vadOpen }
        device.silent.withLock { $0 = false }

        voice.leave()
        // The call report of this call (PROTOCOL.md › Voice › Call report): both drops, kept.
        let report = try #require(voice.callReportText())
        print("CALLREPORT-BEGIN\n\(report)CALLREPORT-END")
        let webName = client.user(VoiceTestEnv.webStatus()?.uid ?? "")?.name ?? "Webby"
        #expect(report.hasPrefix("Yap call report\n"))
        #expect(report.contains("Joined Lounge; nobody else here yet"))
        #expect(report.contains("\(webName) joined"))
        #expect(report.contains("Your connection to the server dropped; the call keeps going meanwhile"))
        #expect(report.contains("Back in the call: it was kept, nothing to reconnect"))
        #expect(report.contains("\(webName) reconnected; the call kept going"))
        #expect(report.contains("reconnects kept: 1"))
        #expect(report.contains("From you: they lost "), "what the web reported receiving from us")
        #expect(report.contains(VoiceTestEnv.redundancy ? "red+opus in" : "opus in"))
        #expect(report.contains("Ended: You left."))
        #expect(report.range(of: #"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"#, options: .regularExpression) == nil, "no IP addresses")
        try await waitUntil("web sharing next", timeout: .seconds(20)) {
            VoiceTestEnv.webStatus()?.phase == (VoiceTestEnv.voiceOnly ? "done" : "web-sharing")
        }
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
        // (Behind the impairment proxy "weak connection" may well show; "reconnecting…" mustn't.)
        #expect(VoiceTestEnv.impair == "none" ? voice.hint(for: webNow) == nil : voice.hint(for: webNow) != .reconnecting, "not reconnecting any more")
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
