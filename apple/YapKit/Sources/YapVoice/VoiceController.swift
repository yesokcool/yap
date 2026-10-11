import AVFoundation
import Foundation
import Observation
import os
import YapCore
import YapScreenLink
@preconcurrency import WebRTC

/// Per-device voice preferences (the web keeps the same ones in localStorage).
public struct VoiceSettings: Codable, Sendable, Equatable {
    public var mode: VoiceEngine.InputMode = .voiceActivity
    /// Opus target, bps.
    public var bitrate: Int = 128_000
    /// Voice-activity threshold, dBFS RMS (like the web and desktop).
    public var vadThreshold: Double = Self.defaultVadThreshold
    public var muted = false
    public var deafened = false
    /// uid → gain (1 = normal, 0…2).
    public var volumes: [String: Double] = [:]
    /// "Hide my IP address in calls": voice only through the server's TURN
    /// relay (when it has one), so the people you talk to never see your address.
    public var relayOnly = false
    /// iPhone: Bluetooth headphones play in high quality (A2DP) while the
    /// phone's own mic listens, instead of the headset's narrowband call mode.
    public var highQualityBluetooth = false
    /// uid → the volume of their screen share's sound (0…1), separate from their voice.
    public var screenVolumes: [String: Double] = [:]
    /// Smooth / Sharp / Max for my next share (the Mac).
    public var shareQuality: ScreenQuality = .smooth
    /// The scale `vadThreshold` is on: 3 = RMS dBFS (the web's); 2 = WebRTC's
    /// peak-based counters, ~9 dB hotter; before that, a 100 ms peak level.
    var vadScale = 3

    public init() {}

    public static let defaultVadThreshold: Double = -50
    /// The sensitivity slider's range (the web's).
    public static let vadRange: ClosedRange<Double> = -80 ... -10

    public static let bitrateChoices = [64_000, 96_000, 128_000, 256_000, 510_000]
    /// What a voice channel can be set to for everyone (nil = each person's own).
    public static let channelBitrateChoices: [Int?] = [nil, 64_000, 96_000, 128_000, 256_000, 510_000]

    /// "64 kbps"; nil → "Each person's own".
    public static func label(forChannelBitrate bitrate: Int?) -> String {
        bitrate.map { "\($0 / 1000) kbps" } ?? "Each person's own"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        mode = (try? c.decode(VoiceEngine.InputMode.self, forKey: .mode)) ?? .voiceActivity
        bitrate = (try? c.decode(Int.self, forKey: .bitrate)) ?? 128_000
        let threshold = (try? c.decode(Double.self, forKey: .vadThreshold)) ?? Self.defaultVadThreshold
        // Old defaults become the RMS default. A value picked on the
        // peak-based scale moves down by the same 9 dB the level now does,
        // so it opens for the same voice as before.
        let scale = (try? c.decode(Int.self, forKey: .vadScale)) ?? 1
        switch scale {
        case ..<2: vadThreshold = threshold == -45 ? Self.defaultVadThreshold : threshold
        case 2 where threshold != Self.defaultVadThreshold:
            vadThreshold = min(Self.vadRange.upperBound, max(Self.vadRange.lowerBound, threshold + VoiceGate.peakToRMSDb))
        default: vadThreshold = threshold
        }
        relayOnly = (try? c.decode(Bool.self, forKey: .relayOnly)) ?? false
        highQualityBluetooth = (try? c.decode(Bool.self, forKey: .highQualityBluetooth)) ?? false
        muted = (try? c.decode(Bool.self, forKey: .muted)) ?? false
        deafened = (try? c.decode(Bool.self, forKey: .deafened)) ?? false
        volumes = (try? c.decode([String: Double].self, forKey: .volumes)) ?? [:]
        screenVolumes = (try? c.decode([String: Double].self, forKey: .screenVolumes)) ?? [:]
        shareQuality = (try? c.decode(ScreenQuality.self, forKey: .shareQuality)) ?? .smooth
    }
}

/// What happened in my call, for sounds (the app plays chimes and blips).
public enum VoiceEvent: Sendable, Equatable {
    /// My join landed / I left (on purpose, or the channel went away).
    case joined, left
    /// Someone joined / left the channel I'm in (OBS listeners don't count).
    case peerJoined(uid: String)
    case peerLeft(uid: String)
    /// I muted/unmuted or deafened/undeafened (true = now quiet).
    case quieted(Bool)
}

/// iPhone: what happened to the call's audio session (the app's background
/// call watch listens: `BackgroundCall`).
public enum AudioSessionEvent: Sendable, Equatable {
    /// A phone call, an alarm, Siri or another app took the sound.
    case interruptionBegan
    case interruptionEnded
    /// The sound couldn't be started again (the session wouldn't
    /// activate, the audio unit wouldn't start, a media-server reset we
    /// couldn't recover from).
    case failed
}

/// A quiet word next to someone in my call, only when something's off
/// (PROTOCOL.md › Voice › Connection hints; nothing goes over the wire).
public enum ConnectionHint: Sendable, Equatable {
    /// Their socket dropped and we're keeping the call ("reconnecting…").
    case reconnecting
    /// ICE failed and the server has no TURN relay ("needs the voice relay").
    case needsRelay
    /// ICE failed although there is a relay ("can't connect").
    case cantConnect
    /// More than 3 % of their audio patched over, or a slow round trip.
    case weak(concealed: Double, rttMs: Double?)
}

/// Glue between the socket (`YapClient`) and the mesh (`VoiceEngine`): the
/// join → own-echo → offer flow, keeping calls across reconnects
/// (`CallKeeper`), mute/deafen state broadcast, connection hints, the
/// audio session, and screen sharing (watching one share at a time; sharing
/// from the Mac).
@MainActor
@Observable
public final class VoiceController: VoiceCoordinator {
    public let engine: VoiceEngine
    /// The channel we're joining or in; survives reconnects.
    public private(set) var wantedChannelId: String? {
        didSet {
            guard (oldValue == nil) != (wantedChannelId == nil) else { return }
            updateBroadcastListener()
            onInVoiceChanged?()
        }
    }
    /// The channel the server confirmed (our own `voice` echo).
    public private(set) var channelId: String?
    public private(set) var micDenied = false
    /// Whether this server has a TURN relay (needed for `relayOnly`), once
    /// `checkRelay()` has asked; nil until then.
    public private(set) var relayAvailable: Bool?
    /// The channel of the last join that landed (a reconnect's rejoin isn't a new join: no sound).
    @ObservationIgnored private var joinedOnce: String?
    /// The joined channel's shared quality (`Channel.bitrate`), nil = each
    /// person's own setting.
    public private(set) var channelBitrate: Int?
    /// What this call sends and asks for: the channel's quality, else mine.
    public var callBitrate: Int { channelBitrate ?? settings.bitrate }
    public var settings: VoiceSettings {
        didSet {
            save()
            applySettings(old: oldValue)
        }
    }

    /// Joins and leaves in my call (chimes, blips).
    @ObservationIgnored public var onEvent: ((VoiceEvent) -> Void)?
    /// iPhone: interruptions and audio that won't come back.
    @ObservationIgnored public var onAudioSession: ((AudioSessionEvent) -> Void)?
    /// The call ended without us asking (the channel was archived).
    @ObservationIgnored public var onEndedByServer: (() -> Void)?
    /// We joined a call or left it (`isInVoice` changed).
    @ObservationIgnored public var onInVoiceChanged: (() -> Void)?

    // MARK: Screen sharing state

    /// The share I'm watching (one at a time, like the web): connecting,
    /// playing, paused, stopped. Nil when I'm not watching.
    public private(set) var watch: ScreenWatch?
    /// Draws the screen I watch; picture-in-picture plays from its layer too.
    public let screenRenderer = ScreenRenderer()
    /// I'm sharing my screen, at this quality (nil = not sharing).
    public private(set) var sharing: ScreenQuality?
    /// Every app shares: the Mac with ScreenCaptureKit, iPhone and iPad
    /// through the ReplayKit broadcast extension (`broadcastSocketPath`).
    public static var canShareScreen: Bool { true }
    /// iPhone and iPad: where Yap's broadcast extension finds the app (the
    /// App Group's socket). The app listens there while it's in a call.
    @ObservationIgnored public var broadcastSocketPath: String? {
        didSet { updateBroadcastListener() }
    }
    @ObservationIgnored private var broadcastServer: ScreenLinkServer?
    @ObservationIgnored private var feeder: (any ScreenFeeder)?
    @ObservationIgnored private var renderedTrack: RTCVideoTrack?
    @ObservationIgnored private var watchTask: Task<Void, Never>?
    /// Seconds, monotonic (a hand-cranked clock in tests).
    @ObservationIgnored var now: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }

    let keeper: CallKeeper
    @ObservationIgnored private unowned let client: YapClient
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let settingsKey: String
    @ObservationIgnored private let requestsMicPermission: Bool
    #if os(iOS)
    @ObservationIgnored private var sessionWatch: AudioSessionWatch?
    #endif

    /// `requestsMicPermission: false` for tests on a synthetic audio device.
    public init(client: YapClient, engine: VoiceEngine = VoiceEngine(), defaults: UserDefaults = .standard,
                settingsKey: String = "yap.voice", requestsMicPermission: Bool = true) {
        self.client = client
        self.engine = engine
        self.defaults = defaults
        self.settingsKey = settingsKey
        self.requestsMicPermission = requestsMicPermission
        keeper = CallKeeper(mesh: engine, directory: client)
        if let data = defaults.data(forKey: settingsKey), let saved = try? JSONDecoder().decode(VoiceSettings.self, from: data) {
            settings = saved
        } else {
            settings = VoiceSettings()
        }
        engine.onSignal = { [weak client] to, data in client?.send(.signal(to: to, data: data)) }
        engine.uidOf = { [weak client] sid in client?.sessions[sid]?.uid }
        engine.resumable = { [weak client] sid in client?.sessions[sid]?.voice?.resume == true }
        engine.fetchIce = { [weak client] in
            guard let endpoint = client?.endpoint else { return [] }
            let servers = await ServerAPI(endpoint: endpoint).iceServers()
            return servers == IceServerConfig.fallback ? [] : servers // couldn't ask: keep what we have
        }
        engine.onPeerState = { [weak self] sid, state in self?.keeper.peerStateChanged(sid, state) }
        engine.onScreenViewers = { [weak self] joined in self?.screenViewersChanged(joined: joined) }
        engine.onRekey = { [weak self] old, new in
            // The sharer I watch came back under a new session: follow it.
            if self?.watch?.sid == old { self?.watch?.sid = new }
        }
        keeper.onChime = { [weak self] event in self?.onEvent?(event) }
        keeper.channelId = { [weak self] in self?.channelId }
        applySettings(old: nil)
        client.voice = self
        #if os(iOS)
        if requestsMicPermission {
            let watch = AudioSessionWatch()
            watch.isInCall = { [weak self] in self?.isInVoice == true }
            watch.highQualityBluetooth = { [weak self] in self?.settings.highQualityBluetooth == true }
            watch.onEvent = { [weak self] event in
                guard let self, self.isInVoice else { return }
                self.onAudioSession?(event)
            }
            RTCAudioSession.sharedInstance().add(watch)
            sessionWatch = watch
        }
        #endif
    }

    public var isInVoice: Bool { wantedChannelId != nil }
    /// My session in the call: the current one, or while the socket is down
    /// and we're keeping the call, the one it was.
    public var ownSid: String? { client.mySid ?? keeper.resume?.sid }
    /// Joined and every peer connected (or alone in the channel).
    public var isConnected: Bool { channelId != nil && channelId == wantedChannelId && engine.allConnected }
    /// Our socket dropped mid-call and we're keeping the call while it comes
    /// back ("Reconnecting… voice is still on").
    public var isResuming: Bool {
        if demo?.resuming == true { return true }
        return keeper.resume != nil
    }

    /// Screenshots only (`YAP_DEMO_VOICE`): these notes and warnings, without a
    /// call. Only the app's debug launch sets it; not behind `#if DEBUG` because
    /// SwiftPM builds this package without DEBUG under the app's Dev configuration.
    public struct Demo: Sendable {
        /// uid → the note next to them.
        public var hints: [String: ConnectionHint] = [:]
        public var resuming = false
        public var silent = false
        public init() {}
    }

    public var demo: Demo?

    /// The quiet word next to someone in my call, if something's off.
    public func hint(for sid: String) -> ConnectionHint? {
        if let demo, let uid = client.sessions[sid]?.uid, let hint = demo.hints[uid] { return hint }
        if keeper.isGhost(sid) { return .reconnecting }
        guard channelId != nil, sid != client.mySid, let state = engine.peerStates[sid] else { return nil }
        if engine.failed.contains(sid), state != .connected { return engine.hasRelay ? .cantConnect : .needsRelay }
        if let weak = engine.weak[sid] { return .weak(concealed: weak.concealed, rttMs: weak.rttMs) }
        return nil
    }

    /// "Your mic is sending silence": nothing at all from it for 5 s while
    /// I'm in a call and not muted.
    public var micSendsSilence: Bool {
        if demo?.silent == true { return true }
        return engine.micSilent && isInVoice && !micDenied && !settings.muted && !settings.deafened
    }

    // MARK: Actions

    public func join(_ channelId: String) async {
        guard client.isConnected, wantedChannelId != channelId else { return }
        wantedChannelId = channelId
        await sendJoin(channelId)
    }

    /// Mic, ICE servers, then `voice.join` (continuing our call if we're keeping it).
    private func sendJoin(_ channelId: String) async {
        guard let endpoint = client.endpoint else { return }
        let micOK = await Self.ensureMicPermission(ask: requestsMicPermission)
        micDenied = !micOK
        guard wantedChannelId == channelId else { return }
        let ice = await ServerAPI(endpoint: endpoint).iceServers()
        guard wantedChannelId == channelId, client.isConnected else { return }
        relayAvailable = IceServerConfig.hasRelay(ice)
        engine.relayOnly = settings.relayOnly && IceServerConfig.hasRelay(ice)
        #if os(iOS)
        if requestsMicPermission { AudioSessionSetup.configure(highQualityBluetooth: settings.highQualityBluetooth) }
        #endif
        engine.mySid = client.mySid
        channelBitrate = client.channelsById[channelId]?.bitrate
        engine.bitrate = callBitrate
        engine.prepare(iceServers: ice)
        engine.setMuted(settings.muted || !micOK)
        engine.setDeafened(settings.deafened)
        let proof = keeper.joinProof(for: channelId)
        client.send(.voiceJoin(channelId: channelId, muted: settings.muted || !micOK, deafened: settings.deafened,
                               resume: true, prev: proof?.prev, proof: proof?.proof, screen: sharing != nil))
    }

    public func leave() {
        guard wantedChannelId != nil || channelId != nil else { return }
        // Leaving voice stops sharing and watching (PROTOCOL.md › Screen sharing).
        stopSharing(announce: false)
        stopWatching()
        if channelId != nil { onEvent?(.left) }
        joinedOnce = nil
        wantedChannelId = nil
        channelId = nil
        // A real goodbye, so nobody keeps our call waiting for us to come back.
        client.send(.voiceLeave)
        keeper.leave()
        engine.stop()
        resetChannelBitrate()
    }

    /// The app is quitting: say goodbye if we're in a call (others would
    /// otherwise keep our connection a while in case we're only reconnecting).
    public func goodbye() {
        guard channelId != nil else { return }
        client.send(.voiceLeave)
    }

    /// Everyone in `channelId` uses this quality (nil = each person's own).
    public func setChannelBitrate(_ bitrate: Int?, for channelId: String) {
        client.setBitrate(bitrate, for: channelId)
    }

    private func resetChannelBitrate() {
        channelBitrate = nil
        engine.bitrate = settings.bitrate
    }

    public func toggleMute() {
        if settings.deafened {
            settings.deafened = false
            settings.muted = false
        } else {
            settings.muted.toggle()
        }
        onEvent?(.quieted(settings.muted || settings.deafened))
    }

    public func toggleDeafen() {
        settings.deafened.toggle()
        onEvent?(.quieted(settings.deafened))
    }

    public func setPushToTalk(_ down: Bool) {
        engine.setPushToTalk(down)
    }

    public func setVolume(_ volume: Double, for uid: String) {
        settings.volumes[uid] = volume
    }

    /// Their screen share's sound, 0…1 (remembered per person on this device).
    public func setScreenVolume(_ volume: Double, for uid: String) {
        settings.screenVolumes[uid] = min(1, max(0, volume))
    }

    public func screenVolume(for uid: String) -> Double {
        settings.screenVolumes[uid] ?? 1
    }

    // MARK: Watching a screen (PROTOCOL.md › Screen sharing)

    /// Watch someone's screen: joins their voice channel first if I'm not
    /// in it, asks them for it (`{screen: "want"}`, again on every
    /// reconnect) and shows it until they stop or I do. Watching someone
    /// else stops watching the last one.
    public func watchScreen(_ sid: String) async {
        guard sid != client.mySid, let session = client.sessions[sid], let voice = session.voice, voice.screen else { return }
        if watch?.sid == sid { return }
        stopWatching()
        watch = ScreenWatch(sid: sid, uid: session.uid)
        screenRenderer.clear()
        startWatchLoop()
        if wantedChannelId != voice.channelId { await join(voice.channelId) }
        guard watch?.sid == sid else { return }
        engine.watchScreen(sid, true)
    }

    /// "Stop watching" (or the viewer closed): tell the sharer, drop the picture.
    public func stopWatching() {
        watchTask?.cancel()
        watchTask = nil
        renderedTrack?.remove(screenRenderer)
        renderedTrack = nil
        guard let watch else { return }
        engine.watchScreen(watch.sid, false)
        self.watch = nil
        screenRenderer.clear()
    }

    /// The share's video track, while it comes (the viewer shows `screenRenderer`).
    public var watchedTrack: RTCVideoTrack? { watch.flatMap { engine.remoteScreens[$0.sid] } }
    /// The share comes with sound (the volume slider matters).
    public var watchedHasSound: Bool { watch.map { engine.remoteScreenSound.contains($0.sid) } ?? false }

    private func startWatchLoop() {
        watchTask?.cancel()
        watchTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.tickWatch()
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
    }

    /// Frames in, the sharer still sharing, time passing: connecting →
    /// playing ⇄ paused (3 s without a frame) → stopped → closed 2 s later.
    func tickWatch() {
        guard var next = watch else { return }
        let time = now()
        let track = engine.remoteScreens[next.sid]
        if track !== renderedTrack {
            renderedTrack?.remove(screenRenderer)
            track?.add(screenRenderer)
            renderedTrack = track
        }
        let counters = screenRenderer.counters
        if let lastAt = counters.lastFrameAt { next.frames(counters.frames, lastAt: lastAt) }
        // (While our own socket is down nobody's list is current: wait for it.)
        if client.phase == .online, client.sessions[next.sid]?.voice?.screen != true { next.sharerStopped(at: time) }
        next.tick(at: time)
        if next.phase == .closed {
            stopWatching()
        } else if next != watch {
            watch = next
        }
    }

    // MARK: Sharing from iPhone and iPad (the broadcast extension)

    /// Listen for the extension while in a call (a broadcast started from
    /// Control Center lands here too); not otherwise, so the extension can
    /// tell you to join a voice channel first.
    private func updateBroadcastListener() {
        let wanted = wantedChannelId != nil && broadcastSocketPath != nil
        if wanted, broadcastServer == nil, let path = broadcastSocketPath {
            let server = ScreenLinkServer(path: path)
            server.onConnection = { [weak self] connection in
                Task { @MainActor in await self?.broadcastArrived(connection) }
            }
            do {
                try server.start()
                broadcastServer = server
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "screen").error("can't listen for the broadcast: \(String(describing: error), privacy: .public)")
            }
        } else if !wanted, let server = broadcastServer {
            server.stop()
            broadcastServer = nil
        }
    }

    /// The extension connected: share what it sends (a new broadcast takes over from an old one).
    func broadcastArrived(_ connection: ScreenLinkConnection) async {
        // Just joined, or the socket is coming back: give the call a moment.
        for _ in 0..<50 where channelId == nil && wantedChannelId != nil { try? await Task.sleep(for: .milliseconds(100)) }
        guard channelId != nil else {
            connection.close()
            return
        }
        if sharing != nil || feeder != nil { stopSharing(announce: false) }
        do {
            try await startSharing(BroadcastScreenFeeder(connection: connection))
        } catch {
            connection.close()
        }
    }

    // MARK: Sharing my screen (the Mac; iPhone and iPad through the extension)

    /// Share my screen in the call: `feeder` asks what to share (the
    /// system picker) and supplies the frames. Only people who ask get the
    /// video; nothing is encoded while nobody watches.
    public func startSharing(_ feeder: any ScreenFeeder, quality: ScreenQuality? = nil) async throws {
        guard channelId != nil, sharing == nil, self.feeder == nil else { return }
        let quality = quality ?? settings.shareQuality
        let source = engine.makeScreenSource()
        self.feeder = feeder
        do {
            try await feeder.start(into: source, quality: quality)
        } catch {
            self.feeder = nil
            engine.stopScreen()
            throw error
        }
        guard channelId != nil, self.feeder === feeder else {
            feeder.stop()
            return
        }
        feeder.onEnded = { [weak self] in self?.stopSharing() }
        engine.startScreen(quality: quality)
        sharing = quality
        feeder.setDelivering(!engine.screenViewers.isEmpty)
        sendState()
    }

    /// Smooth / Sharp / Max while sharing (and for the next share).
    public func setShareQuality(_ quality: ScreenQuality) {
        settings.shareQuality = quality
        guard sharing != nil else { return }
        sharing = quality
        engine.setScreenQuality(quality)
        feeder?.setQuality(quality)
    }

    /// Another window or screen, without stopping (where there's a picker).
    public var canChangeSharedContent: Bool { feeder?.canChangeContent ?? false }
    public func changeSharedContent() { feeder?.changeContent() }

    public func stopSharing() { stopSharing(announce: true) }

    private func stopSharing(announce: Bool) {
        guard sharing != nil || feeder != nil else { return }
        feeder?.stop()
        feeder = nil
        engine.stopScreen()
        sharing = nil
        if announce { sendState() }
    }

    /// Who's watching my share: their accounts, in the order they asked
    /// (each person once). Local only.
    public var screenWatchers: [String] {
        var seen = Set<String>()
        return engine.screenViewers.compactMap { client.sessions[$0]?.uid ?? engine.peerUid($0) }.filter { seen.insert($0).inserted }
    }

    private func screenViewersChanged(joined: Bool) {
        guard let feeder else { return }
        feeder.setDelivering(!engine.screenViewers.isEmpty)
        if joined { feeder.refresh() }
    }

    /// Asks the server whether it has a voice relay (Settings shows "Hide my
    /// IP address" as unavailable without one).
    public func checkRelay() async {
        guard let endpoint = client.endpoint else { return }
        relayAvailable = IceServerConfig.hasRelay(await ServerAPI(endpoint: endpoint).iceServers())
    }

    // MARK: VoiceCoordinator

    public func clientDidBecomeReady() {
        engine.mySid = client.mySid
        // Kept our call? Match who's back, keep waiting for who isn't.
        keeper.ready()
        guard let wanted = wantedChannelId else { return }
        if client.channelsById[wanted]?.kind == .voice {
            Task { await sendJoin(wanted) }
        } else {
            wantedChannelId = nil
            joinedOnce = nil
            keeper.leave()
            engine.stop()
            resetChannelBitrate()
        }
    }

    public func clientConnectionDropped() {
        let current = channelId
        channelId = nil
        // Audio is peer to peer: a call can outlive the socket.
        keeper.socketDropped(mySid: engine.mySid, key: client.resumeKey, channelId: current)
    }

    public func clientDidSignOut() {
        stopSharing(announce: false)
        stopWatching()
        joinedOnce = nil
        wantedChannelId = nil
        channelId = nil
        keeper.leave()
        engine.stop()
        resetChannelBitrate()
    }

    /// The channel we're in changed its shared quality: renegotiate every call.
    public func clientChannelChanged(_ channel: Channel) {
        guard channel.id == wantedChannelId, channel.bitrate != channelBitrate else { return }
        let before = callBitrate
        channelBitrate = channel.bitrate
        engine.bitrate = callBitrate
        if callBitrate != before { engine.applySettings(renegotiate: true) }
    }

    public func clientVoiceChanged(_ change: VoiceChange) {
        let sid = change.sid
        guard sid == client.mySid else {
            // Someone else: joins, leaves, lost sockets, resumes (and their chimes).
            keeper.voiceChanged(change, mine: channelId)
            return
        }
        if let newChannel = change.channelId, newChannel != channelId {
            // Our join landed: offer to everyone already there — or, if the
            // server took our resume, carry on with the connections we kept.
            let rejoin = channelId == nil && change.previous == nil && wantedChannelId == newChannel && joinedOnce == newChannel
            channelId = newChannel
            joinedOnce = newChannel
            wantedChannelId = newChannel
            engine.mySid = sid
            let others = client.voiceSessions(in: newChannel).map(\.sid).filter { $0 != sid }
            let handedOver = keeper.joined(prev: change.prev, others: others)
            if !rejoin && !handedOver { onEvent?(.joined) }
            // Still sharing after a join that didn't carry it over: say so again.
            if sharing != nil, client.sessions[sid]?.voice?.screen != true { sendState() }
        } else if change.channelId == nil, let current = channelId {
            keeper.leave()
            engine.hangUpAll()
            channelId = nil
            // Not a switch we asked for (the channel was archived): we're out.
            if wantedChannelId == current {
                stopSharing(announce: false)
                stopWatching()
                wantedChannelId = nil
                joinedOnce = nil
                engine.stop()
                resetChannelBitrate()
                onEvent?(.left)
                onEndedByServer?()
            }
        }
    }

    public func clientSessionLeft(sid: String) {
        keeper.sessionLeft(sid)
    }

    public func clientReceivedSignal(_ data: SignalData, from sid: String) {
        guard let mine = channelId, client.sessions[sid]?.voice?.channelId == mine else { return }
        engine.handleSignal(data, from: sid)
    }

    // MARK: Settings

    private func applySettings(old: VoiceSettings?) {
        engine.mode = settings.mode
        engine.vadThreshold = settings.vadThreshold
        engine.volumes = settings.volumes
        engine.screenVolumes = settings.screenVolumes
        if old != nil, old?.screenVolumes != settings.screenVolumes { engine.applyScreenVolumes() }
        // A channel-wide quality overrides mine; changing mine then changes nothing.
        let bitrateChanged = old != nil && old?.bitrate != settings.bitrate && channelBitrate == nil
        engine.bitrate = callBitrate
        engine.setMuted(settings.muted || micDenied)
        engine.setDeafened(settings.deafened)
        if old == nil || old?.mode != settings.mode { engine.setPushToTalk(false) }
        engine.applySettings(renegotiate: bitrateChanged)
        if channelId != nil, old?.muted != settings.muted || old?.deafened != settings.deafened {
            sendState()
        }
    }

    /// Mute, deafen and whether I'm sharing, for everyone to see.
    private func sendState() {
        guard channelId != nil else { return }
        client.send(.voiceState(muted: settings.muted || micDenied, deafened: settings.deafened, screen: sharing != nil))
    }

    private func save() {
        if let data = try? JSONEncoder().encode(settings) { defaults.set(data, forKey: settingsKey) }
    }

    // MARK: Microphone permission

    public static func ensureMicPermission(ask: Bool) async -> Bool {
        guard ask else { return true }
        #if os(iOS)
        switch AVAudioApplication.shared.recordPermission {
        case .granted: return true
        case .denied: return false
        default: return await AVAudioApplication.requestRecordPermission()
        }
        #else
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return true
        case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
        default: return false
        }
        #endif
    }
}

#if os(iOS)
/// `.playAndRecord` + `.voiceChat` (Apple's echo cancellation and AGC),
/// Bluetooth headsets allowed, speaker by default when nothing's plugged in
/// (a phone in hand, not at the ear). WebRTC's audio unit activates the
/// session with this when the call starts; the `audio` background mode keeps
/// it (and the socket) alive when the app is in the background.
///
/// With `highQualityBluetooth`, Bluetooth headphones stay in A2DP (stereo,
/// full band) and the phone's own mic is used: `.voiceChat` would switch any
/// headset to its narrowband call profile (HFP), so the mode is `.default`
/// (WebRTC's voice-processing unit still cancels echo).
enum AudioSessionSetup {
    static func configure(highQualityBluetooth: Bool) {
        let config = RTCAudioSessionConfiguration.webRTC()
        config.category = AVAudioSession.Category.playAndRecord.rawValue
        if highQualityBluetooth {
            config.mode = AVAudioSession.Mode.default.rawValue
            config.categoryOptions = [.allowBluetoothA2DP, .defaultToSpeaker]
        } else {
            config.mode = AVAudioSession.Mode.voiceChat.rawValue
            config.categoryOptions = [.allowBluetoothHFP, .defaultToSpeaker]
        }
        RTCAudioSessionConfiguration.setWebRTC(config)
    }
}
#endif

#if os(iOS)
/// What iOS does to the call's audio: a phone call or Siri interrupting,
/// AirPods connecting, the media server restarting. WebRTC's own session
/// object reacts first (reactivates the session after an interruption and,
/// with the `WebRTC-Audio-iOS-Holding` field trial, restarts its audio
/// unit); this logs it and, if the media server was reset, puts our
/// configuration back. A mic that still delivers nothing shows as "Your mic
/// is sending silence" (`VoiceEngine.micSilent`).
final class AudioSessionWatch: NSObject, RTCAudioSessionDelegate, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) var isInCall: (@MainActor () -> Bool)?
    nonisolated(unsafe) var highQualityBluetooth: (@MainActor () -> Bool)?
    /// Interruptions and failures, on the main thread.
    nonisolated(unsafe) var onEvent: (@MainActor (AudioSessionEvent) -> Void)?
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    private func tell(_ event: AudioSessionEvent) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onEvent?(event) }
        }
    }

    func audioSessionDidBeginInterruption(_ session: RTCAudioSession) {
        log.info("audio interrupted")
        tell(.interruptionBegan)
    }

    func audioSessionDidEndInterruption(_ session: RTCAudioSession, shouldResumeSession: Bool) {
        log.info("audio interruption ended (resume: \(shouldResumeSession))")
        tell(.interruptionEnded)
    }

    func audioSession(_ audioSession: RTCAudioSession, failedToSetActive active: Bool, error: any Error) {
        log.error("audio session didn't \(active ? "activate" : "deactivate", privacy: .public): \(error.localizedDescription, privacy: .public)")
        if active { tell(.failed) }
    }

    func audioSession(_ audioSession: RTCAudioSession, audioUnitStartFailedWithError error: any Error) {
        log.error("the audio unit didn't start: \(error.localizedDescription, privacy: .public)")
        tell(.failed)
    }

    func audioSessionDidChangeRoute(_ session: RTCAudioSession, reason: AVAudioSession.RouteChangeReason, previousRoute: AVAudioSessionRouteDescription) {
        let outputs = session.currentRoute.outputs.map(\.portType.rawValue).joined(separator: ",")
        let inputs = session.currentRoute.inputs.map(\.portType.rawValue).joined(separator: ",")
        log.info("audio route changed (\(reason.rawValue)): in \(inputs, privacy: .public) out \(outputs, privacy: .public)")
    }

    func audioSessionMediaServerReset(_ session: RTCAudioSession) {
        log.error("the media server was reset")
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.isInCall?() == true else { return }
                AudioSessionSetup.configure(highQualityBluetooth: self.highQualityBluetooth?() == true)
                let rtc = RTCAudioSession.sharedInstance()
                rtc.lockForConfiguration()
                defer { rtc.unlockForConfiguration() }
                do {
                    try rtc.setConfiguration(RTCAudioSessionConfiguration.webRTC(), active: true)
                } catch {
                    self.log.error("couldn't restore the audio session: \(error.localizedDescription, privacy: .public)")
                    self.onEvent?(.failed)
                }
            }
        }
    }
}
#endif
