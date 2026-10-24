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
    /// relay, so the people you talk to never see your address. Without a
    /// relay that means no voice at all, never a direct connection (the call
    /// says so: `VoiceController.relayMissing`).
    public var relayOnly = false
    /// iPhone: Bluetooth headphones play in high quality (A2DP) while the
    /// phone's own mic listens, instead of the headset's narrowband call mode.
    public var highQualityBluetooth = false
    /// uid → the volume of their screen share's sound (0…1), separate from their voice.
    public var screenVolumes: [String: Double] = [:]
    /// Smooth / Sharp / Max for my next share (the Mac).
    public var shareQuality: ScreenQuality = .smooth
    /// "Remind me when I talk while muted": "You're muted", and a soft blip
    /// the first time since muting (PROTOCOL.md › Voice › Transmit gate).
    public var mutedReminder = true
    /// "Push-to-talk click": a quiet click, for my ears only, when push to
    /// talk opens and when it closes.
    public var pushToTalkClick = true
    /// "Lower shared media while friends talk": a screen share's sound eases
    /// to half while someone else in the call speaks (off by default).
    public var dipSharedMedia = false
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
        mutedReminder = (try? c.decode(Bool.self, forKey: .mutedReminder)) ?? true
        pushToTalkClick = (try? c.decode(Bool.self, forKey: .pushToTalkClick)) ?? true
        dipSharedMedia = (try? c.decode(Bool.self, forKey: .dipSharedMedia)) ?? false
    }

    /// The quality warning next to 256 and 510 kbps (PROTOCOL.md › Voice):
    /// past ~230 kbps a frame and its copy don't fit in a packet, so there's
    /// no RED, and each friend costs several times the upload.
    public static func qualityWarning(_ bitrate: Int) -> String? {
        bitrate > 128_000 ? "No dropout protection at this quality (each bit of voice no longer fits twice in a packet), and several times the upload for each friend in the call." : nil
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
    /// I'm talking while muted (at most every 5 s): "You're muted", and the
    /// soft blip if `blip` (the first since I muted). `firstTime`: the first
    /// blip ever on this device (say what the sound means).
    case talkingWhileMuted(blip: Bool, firstTime: Bool)
    /// Push to talk went live / closed after its tail: the click (if it's on).
    case pushToTalk(live: Bool)
}

/// iPhone: what happened to the call's audio session (the app's background
/// call watch listens: `BackgroundCall`).
public enum AudioSessionEvent: Sendable, Equatable {
    /// A phone call, an alarm, Siri or another app took the sound.
    case interruptionBegan
    case interruptionEnded
    /// The sound couldn't be started again (the session wouldn't
    /// activate, the audio unit wouldn't start, a media-server reset we
    /// couldn't recover from). `VoiceController` keeps trying.
    case failed
    /// After `failed`: the sound runs again.
    case recovered
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
    /// Hide my IP address is on and there's no relay to go through: nobody
    /// can be reached (it never falls back to a direct connection).
    case hiddenNoRelay
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
    /// The call report's notes (PROTOCOL.md › Voice › Call report).
    public let notes: CallNotes
    /// How the app names itself in a call report ("Yap for iPhone 1.0 (5)").
    @ObservationIgnored public var appDescription = "Yap"
    /// Why the call is about to end, when it isn't "You left." (the app's off-screen reasons).
    @ObservationIgnored private var endingReason: String?
    @ObservationIgnored private var networkWatch: NetworkWatch?
    @ObservationIgnored private var network: String?
    @ObservationIgnored private var noted = NotedState()
    /// "You're muted": one nudge every 5 s at most, the blip once per mute.
    @ObservationIgnored private var nudge = MutedNudge()
    @ObservationIgnored private var nudgeHide: Task<Void, Never>?
    /// "You're muted" shows by the Mute button (for 2.5 s after each nudge).
    public private(set) var mutedNudgeShown = false
    @ObservationIgnored private unowned let client: YapClient
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let settingsKey: String
    @ObservationIgnored private let requestsMicPermission: Bool
    #if os(iOS)
    @ObservationIgnored private var sessionWatch: AudioSessionWatch?
    /// An interruption holds the sound (nothing to start until it ends).
    @ObservationIgnored private var audioInterrupted = false
    /// Getting the sound back after it couldn't be started again.
    @ObservationIgnored private var soundRecovery: Task<Void, Never>?
    #endif

    /// `requestsMicPermission: false` for tests on a synthetic audio device.
    /// `callReports`: where the last three call reports are kept (the app
    /// passes `CallReportStore.standard`; nil keeps them in memory only).
    public init(client: YapClient, engine: VoiceEngine = VoiceEngine(), defaults: UserDefaults = .standard,
                settingsKey: String = "yap.voice", requestsMicPermission: Bool = true, callReports: CallReportStore? = nil) {
        self.client = client
        self.engine = engine
        notes = CallNotes(store: callReports)
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
        engine.onPeerState = { [weak self] sid, state in
            self?.keeper.peerStateChanged(sid, state)
            self?.notePeerState(sid, state)
        }
        engine.onPeerEvent = { [weak self] sid, event in self?.notePeerEvent(sid, event) }
        engine.onStats = { [weak self] stats in self?.noteStats(stats) }
        engine.onScreenViewers = { [weak self] joined in self?.screenViewersChanged(joined: joined) }
        engine.onTalkWhileMuted = { [weak self] in self?.talkingWhileMuted() }
        engine.onPushToTalk = { [weak self] live in
            guard let self, self.settings.pushToTalkClick else { return }
            self.onEvent?(.pushToTalk(live: live))
        }
        engine.onRekey = { [weak self] old, new in
            // The sharer I watch came back under a new session: follow it.
            if self?.watch?.sid == old { self?.watch?.sid = new }
        }
        keeper.onChime = { [weak self] event in self?.onEvent?(event) }
        keeper.onGaveUp = { [weak self] in self?.notes.note("Not back on the server within 20 s: the call starts over") }
        keeper.channelId = { [weak self] in self?.channelId }
        applySettings(old: nil)
        client.voice = self
        #if os(iOS)
        if requestsMicPermission {
            let watch = AudioSessionWatch()
            watch.isInCall = { [weak self] in self?.isInVoice == true }
            watch.highQualityBluetooth = { [weak self] in self?.settings.highQualityBluetooth == true }
            watch.onEvent = { [weak self] event in self?.audioSessionEvent(event) }
            watch.onNote = { [weak self] text in self?.notes.note(text) }
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
        /// "Your connection is dropping some of your audio".
        public var ownWeak = false
        /// These people (uids) show as speaking.
        public var speaking: Set<String> = []
        public init() {}
    }

    public var demo: Demo?

    /// The quiet word next to someone in my call, if something's off.
    public func hint(for sid: String) -> ConnectionHint? {
        if let demo, let uid = client.sessions[sid]?.uid, let hint = demo.hints[uid] { return hint }
        if keeper.isGhost(sid) { return .reconnecting }
        guard channelId != nil, sid != client.mySid, let state = engine.peerStates[sid] else { return nil }
        if relayMissing, state != .connected { return .hiddenNoRelay }
        if engine.failed.contains(sid), state != .connected { return engine.hasRelay ? .cantConnect : .needsRelay }
        if let weak = engine.weak[sid] { return .weak(concealed: weak.concealed, rttMs: weak.rttMs) }
        return nil
    }

    /// "Your connection is dropping some of your audio": most people in my
    /// call lose more than 3 % of what I send (PROTOCOL.md › Voice ›
    /// Connection hints › Your own connection).
    public var ownConnectionWeak: Bool {
        if demo?.ownWeak == true { return true }
        return engine.ownLinkWeak && isInVoice
    }

    /// In a call with Hide my IP address on and no voice relay (none on this
    /// server, or `/api/ice` didn't answer): nobody can be reached, and the
    /// call says so instead of quietly connecting directly.
    public var relayMissing: Bool {
        wantedChannelId != nil && engine.relayOnly && !engine.hasRelay
    }

    /// Someone in voice is speaking (their ring): the engine's levels (or a screenshot's).
    public func isSpeaking(_ session: Session) -> Bool {
        if let demo, demo.speaking.contains(session.uid) { return true }
        return engine.speaking.contains(session.sid)
    }

    /// Talking while muted (the engine says so on every reading): "You're
    /// muted" at most every 5 s, if the reminder is on, with a soft blip the
    /// first time since muting; the first blip ever on this device, the app
    /// also says what the sound means. Not while another of my devices is
    /// live in this call.
    func talkingWhileMuted() {
        guard settings.mutedReminder, settings.muted, !settings.deafened, !micDenied, isInVoice,
              !MutedNudge.liveElsewhere(client.sessions.values, uid: client.uid, mySid: client.mySid, channelId: channelId),
              let kind = nudge.talking(at: now()) else { return }
        let key = settingsKey + ".mutedNudgeExplained"
        let firstTime = kind == .blip && !defaults.bool(forKey: key)
        if firstTime { defaults.set(true, forKey: key) }
        mutedNudgeShown = true
        nudgeHide?.cancel()
        nudgeHide = Task { [weak self] in
            try? await Task.sleep(for: .seconds(MutedNudge.shownFor))
            guard !Task.isCancelled else { return }
            self?.mutedNudgeShown = false
        }
        onEvent?(.talkingWhileMuted(blip: kind == .blip, firstTime: firstTime))
    }

    /// Screenshots: "You're muted" by the button, without talking.
    public func showMutedNudgeForDemo() {
        mutedNudgeShown = true
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
        applyRelaySetting(ice)
        #if os(iOS)
        if requestsMicPermission { AudioSessionSetup.configure(highQualityBluetooth: settings.highQualityBluetooth) }
        #endif
        engine.mySid = client.mySid
        channelBitrate = client.channelsById[channelId]?.bitrate
        engine.bitrate = callBitrate
        engine.prepare(iceServers: ice)
        engine.setMuted(settings.muted || !micOK)
        engine.setDeafened(settings.deafened)
        nudge.muted() // (a new call: its first nudge blips, muted or not yet)
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
        endNotes(endingReason ?? "You left.")
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
        endNotes("Yap was quit.")
        guard channelId != nil else { return }
        client.send(.voiceLeave)
    }

    /// The next leave() ends the call for this reason (the call report says it).
    public func noteEnding(_ reason: String) {
        endingReason = reason
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

    /// Hide my IP address: relay only whenever it's on. It fails closed: no
    /// relay (or `/api/ice` didn't answer) means nobody can be reached, never
    /// a direct connection that shows my address (`relayMissing` says so).
    func applyRelaySetting(_ ice: [IceServerConfig]) {
        relayAvailable = IceServerConfig.hasRelay(ice)
        engine.relayOnly = settings.relayOnly
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
        notes.note("Back on the server")
        if client.channelsById[wanted]?.kind == .voice {
            Task { await sendJoin(wanted) }
        } else {
            // Archived while our socket was down: we're out, the same as when
            // it happens while we're connected (the share and the screen
            // we watch end too; the app says why).
            endNotes("The channel is gone.")
            stopSharing(announce: false)
            stopWatching()
            wantedChannelId = nil
            joinedOnce = nil
            keeper.leave()
            engine.stop()
            resetChannelBitrate()
            onEvent?(.left)
            onEndedByServer?()
        }
    }

    public func clientConnectionDropped() {
        let current = channelId
        if current != nil { notes.note("Your connection to the server dropped; the call keeps going meanwhile") }
        channelId = nil
        // Audio is peer to peer: a call can outlive the socket.
        keeper.socketDropped(mySid: engine.mySid, key: client.resumeKey, channelId: current)
    }

    public func clientDidSignOut() {
        endNotes("You signed out.")
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
        notes.note("The channel's quality: \(channel.bitrate.map { "\($0 / 1000) kbps for everyone" } ?? "each person's own")")
        engine.bitrate = callBitrate
        if callBitrate != before { engine.applySettings(renegotiate: true) }
    }

    public func clientVoiceChanged(_ change: VoiceChange) {
        let sid = change.sid
        guard sid == client.mySid else {
            // Someone else: joins, leaves, lost sockets, resumes (and their chimes).
            let kept = change.prev.map { engine.peerSids.contains($0) } ?? false
            keeper.voiceChanged(change, mine: channelId)
            noteVoiceChange(change, kept: kept)
            return
        }
        if let newChannel = change.channelId, newChannel != channelId {
            // Our join landed: offer to everyone already there — or, if the
            // server took our resume, carry on with the connections we kept.
            let rejoin = channelId == nil && change.previous == nil && wantedChannelId == newChannel && joinedOnce == newChannel
            let newCall = joinedOnce != newChannel
            channelId = newChannel
            joinedOnce = newChannel
            wantedChannelId = newChannel
            engine.mySid = sid
            let others = client.voiceSessions(in: newChannel).map(\.sid).filter { $0 != sid }
            let handedOver = keeper.joined(prev: change.prev, others: others)
            if !rejoin && !handedOver { onEvent?(.joined) }
            if newCall {
                beginNotes(channelId: newChannel, others: others)
            } else {
                notes.note(handedOver ? "Back in the call: it was kept, nothing to reconnect" : "Back in the call as a newcomer (connecting again)")
            }
            // Still sharing after a join that didn't carry it over: say so again.
            if sharing != nil, client.sessions[sid]?.voice?.screen != true { sendState() }
        } else if change.channelId == nil, let current = channelId {
            keeper.leave()
            engine.hangUpAll()
            channelId = nil
            // Not a switch we asked for (the channel was archived): we're out.
            if wantedChannelId == current {
                endNotes("The channel was archived.")
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
        if let old { noteSettings(old: old) }
        engine.mode = settings.mode
        engine.vadThreshold = settings.vadThreshold
        engine.volumes = settings.volumes
        engine.dipSharedMedia = settings.dipSharedMedia
        engine.screenVolumes = settings.screenVolumes
        if old != nil, old?.screenVolumes != settings.screenVolumes { engine.applyScreenVolumes() }
        // A channel-wide quality overrides mine; changing mine then changes nothing.
        let bitrateChanged = old != nil && old?.bitrate != settings.bitrate && channelBitrate == nil
        engine.bitrate = callBitrate
        engine.setMuted(settings.muted || micDenied)
        engine.setDeafened(settings.deafened)
        if old == nil || old?.mode != settings.mode { engine.resetPushToTalk() }
        if settings.muted, old?.muted == false { nudge.muted() }
        if !settings.muted || settings.deafened, mutedNudgeShown {
            nudgeHide?.cancel()
            mutedNudgeShown = false
        }
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

// MARK: The call report (PROTOCOL.md › Voice › Call report)

/// What the notes last said, so a change is written once.
struct NotedState {
    /// uid → the connection state last noted.
    var states: [String: VoiceEngine.PeerState] = [:]
    var micSilent = false
    var ownWeak = false
    var micMode: String?
    var sensitivityAt: Date?
}

extension VoiceController {
    /// A session's account and name ("Someone" if we never knew).
    func who(_ sid: String) -> (uid: String, name: String)? {
        guard let uid = client.sessions[sid]?.uid ?? engine.peerUid(sid) else { return nil }
        return (uid, client.user(uid)?.name ?? "Someone")
    }

    /// Copy the report: the call we're in, else the last one kept.
    public func callReportText() -> String? {
        notes.text(live: isInVoice)
    }

    func beginNotes(channelId: String, others: [String]) {
        let name = client.channelsById[channelId]?.name ?? channelId
        let report = CallReport(started: Date(), app: appDescription, device: DeviceInfo.description, channel: name,
                                settings: reportSettings(), audio: AudioRouteInfo.describe())
        notes.begin(report)
        noted = NotedState()
        noted.micMode = MicModes.offered ? MicModes.reported : nil
        let people = others.compactMap { client.sessions[$0] }.filter { !$0.isListener }
        var names: [String] = []
        for session in people {
            let name = client.user(session.uid)?.name ?? "Someone"
            notes.person(session.uid, name)
            names.append(name)
        }
        notes.note("Joined \(name)" + (names.isEmpty ? "; nobody else here yet" : " with \(names.joined(separator: ", "))"))
        network = nil
        networkWatch?.cancel()
        networkWatch = NetworkWatch { [weak self] kind in self?.networkChanged(kind) }
    }

    func endNotes(_ reason: String) {
        endingReason = nil
        networkWatch?.cancel()
        networkWatch = nil
        notes.end(reason)
    }

    private func networkChanged(_ kind: String) {
        guard notes.isTaking, kind != network else { return }
        notes.note(network.map { "Network: \($0) → \(kind)" } ?? "Network: \(kind)")
        network = kind
    }

    /// Someone else's `voice` event in our channel: joins, leaves, resumes.
    func noteVoiceChange(_ change: VoiceChange, kept: Bool) {
        guard let mine = channelId, let session = client.sessions[change.sid], !session.isListener else { return }
        let name = client.user(session.uid)?.name ?? "Someone"
        if change.channelId == mine, change.previous != mine {
            if kept { notes.reconnectKept(uid: session.uid, name: name) } else { notes.joined(uid: session.uid, name: name) }
        } else if change.previous == mine, change.channelId != mine {
            notes.left(uid: session.uid, name: name, lost: change.lost)
        }
    }

    func notePeerState(_ sid: String, _ state: VoiceEngine.PeerState?) {
        // Closing is said by the leave; a move to a new session isn't a change.
        guard notes.isTaking, let state, state != .new, let person = who(sid), noted.states[person.uid] != state else { return }
        noted.states[person.uid] = state
        notes.connectionState(uid: person.uid, name: person.name, state)
    }

    func notePeerEvent(_ sid: String, _ event: VoiceEngine.PeerEvent) {
        guard notes.isTaking, let (uid, name) = who(sid).map({ ($0.uid, $0.name) }) else { return }
        switch event {
        case .iceRestart(let n): notes.iceRestart(uid: uid, name: name, count: n)
        case .rebuilt: notes.rebuilt(uid: uid, name: name, words: "connection rebuilt")
        case .freshStart: notes.rebuilt(uid: uid, name: name, words: "they started a new connection, and we followed")
        case .weak(let on): notes.weak(uid: uid, name: name, on)
        }
    }

    func noteStats(_ stats: [PeerStats]) {
        guard notes.isTaking else { return }
        notes.stats(stats.compactMap { s in
            who(s.sid).map { CallNotes.Reading(uid: $0.uid, name: $0.name, connected: engine.peerStates[s.sid] == .connected, stats: s) }
        })
        // Every 2 s: the mic warning and (iPhone) the mic mode.
        if micSendsSilence != noted.micSilent {
            noted.micSilent = micSendsSilence
            notes.note(micSendsSilence ? "\"Your mic is sending silence\" shown" : "The mic is sending sound again")
        }
        if engine.ownLinkWeak != noted.ownWeak {
            noted.ownWeak = engine.ownLinkWeak
            notes.note(engine.ownLinkWeak
                ? "Your connection is dropping some of your audio (they lose about \(Int((engine.ownLinkLost * 100).rounded())) % of yours)"
                : "Your connection is fine again")
        }
        if MicModes.offered {
            let mode = MicModes.reported
            if mode != noted.micMode {
                notes.note("Mic mode: \(mode)")
                noted.micMode = mode
            }
        }
    }

    /// Settings changed mid-call (the timeline says which).
    func noteSettings(old: VoiceSettings) {
        guard notes.isTaking else { return }
        let new = settings
        if old.muted != new.muted { notes.note(new.muted ? "You muted" : "You unmuted") }
        if old.deafened != new.deafened { notes.note(new.deafened ? "You deafened" : "You undeafened") }
        if old.mode != new.mode { notes.note("Settings: input \(Self.inputText(new))") }
        if old.bitrate != new.bitrate { notes.note("Settings: your quality \(new.bitrate / 1000) kbps") }
        if old.vadThreshold != new.vadThreshold {
            // A slider drag: once per few seconds.
            let now = Date()
            if noted.sensitivityAt.map({ now.timeIntervalSince($0) > 3 }) ?? true {
                notes.note("Settings: sensitivity \(Int(new.vadThreshold)) dB")
                noted.sensitivityAt = now
            }
        }
        if old.relayOnly != new.relayOnly { notes.note("Settings: hide my IP address \(new.relayOnly ? "on" : "off") (from the next join)") }
        if old.highQualityBluetooth != new.highQualityBluetooth {
            notes.note("Settings: high-quality headphone audio \(new.highQualityBluetooth ? "on" : "off") (from the next join)")
        }
        if old.dipSharedMedia != new.dipSharedMedia {
            notes.note("Settings: lower shared media while friends talk \(new.dipSharedMedia ? "on" : "off")")
        }
    }

    static func inputText(_ settings: VoiceSettings) -> String {
        switch settings.mode {
        case .voiceActivity: "voice activity, sensitivity \(Int(settings.vadThreshold)) dB"
        case .pushToTalk: "push to talk"
        case .openMic: "always on"
        }
    }

    /// The header's settings lines.
    func reportSettings() -> [String] {
        var lines: [String] = []
        let mine = "\(settings.bitrate / 1000) kbps"
        lines.append(channelBitrate.map { "Quality: \($0 / 1000) kbps, the channel's for everyone (yours: \(mine))" } ?? "Quality: \(mine)")
        lines.append("Input: \(Self.inputText(settings))")
        #if os(iOS)
        let processing = "on (Apple's voice processing)"
        #else
        let processing = "on (WebRTC's)"
        #endif
        lines.append("Noise suppression: \(processing)")
        lines.append("Echo cancellation: \(processing)")
        lines.append("Automatic gain: \(processing)")
        lines.append("Music mode: not in this app")
        lines.append("Hide my IP address: \(!engine.relayOnly ? "off" : engine.hasRelay ? "on" : "on, but this server has no voice relay (nobody can be reached)")")
        lines.append("Redundancy (RED): \(engine.redundancy ? "on" : "off")")
        lines.append("Lower shared media while friends talk: \(settings.dipSharedMedia ? "on" : "off")")
        #if os(iOS)
        lines.append("High-quality headphone audio: \(settings.highQualityBluetooth ? "on" : "off")")
        lines.append("Mic mode: \(MicModes.reported) (what iOS reports)")
        #endif
        return lines
    }
}

#if os(iOS)
// MARK: Getting the sound back (iPhone)

extension VoiceController {
    /// After the sound couldn't be started again: when to try again
    /// (seconds after it failed), then every `soundRetryEvery`.
    static let soundRetries: [TimeInterval] = [1, 2, 4, 8]
    static let soundRetryEvery: TimeInterval = 10

    /// What iOS did to the call's audio: into the call report, and on to
    /// the app (`BackgroundCall`).
    private func audioSessionEvent(_ event: AudioSessionEvent) {
        switch event {
        case .interruptionBegan: audioInterrupted = true
        case .interruptionEnded: audioInterrupted = false
        case .failed, .recovered: break
        }
        guard isInVoice else { return }
        let words = switch event {
        case .interruptionBegan: "Sound interrupted (a phone call, an alarm, Siri or another app)"
        case .interruptionEnded: "Interruption over"
        case .failed: "The sound couldn't be started again"
        case .recovered: "The sound is back"
        }
        notes.note(words)
        if event == .failed { recoverSound() }
        onAudioSession?(event)
    }

    /// The sound couldn't be started again, often only for a moment
    /// (dictation or Siri still letting go of the mic). Nothing in WebRTC
    /// tries again until the next interruption or until Yap comes back on
    /// screen, so we do what it does then (`AudioSessionWatch.restartSound`)
    /// now and then, until the mic's counters move again or the call ends.
    private func recoverSound() {
        guard soundRecovery == nil else { return }
        let lostAt = VoiceEngine.now
        soundRecovery = Task { [weak self] in
            var tries = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                guard let self, self.isInVoice else { break }
                let now = VoiceEngine.now
                // (Half a second: the counters may still show the last moment before it stopped.)
                if let ran = self.engine.micRanAt, ran > lostAt + 0.5 {
                    self.soundRecovery = nil
                    self.audioSessionEvent(.recovered)
                    return
                }
                let due = tries < Self.soundRetries.count
                    ? Self.soundRetries[tries]
                    : Self.soundRetries[Self.soundRetries.count - 1] + Double(tries - Self.soundRetries.count + 1) * Self.soundRetryEvery
                guard now - lostAt >= due, !self.audioInterrupted else { continue }
                tries += 1
                self.notes.note("Starting the sound again (try \(tries))")
                self.sessionWatch?.restartSound()
            }
            self?.soundRecovery = nil
        }
    }
}
#endif

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
///
/// Outside calls the session is `everyday()`: ambient, mixing with other
/// apps. WebRTC doesn't put that back when a call ends (it only lets the
/// session go), so the app does.
public enum AudioSessionSetup {
    /// Ambient, mixing with others: Yap's own sounds (blips, chimes, GIFs)
    /// never stop someone's music and follow the silent switch.
    public static func everyday() {
        try? AVAudioSession.sharedInstance().setCategory(.ambient, options: [.mixWithOthers])
    }

    /// The session is still a call's (play and record) although no call is running.
    public static var leftByACall: Bool {
        AVAudioSession.sharedInstance().category == .playAndRecord
    }

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
/// configuration back. When the sound won't start again, `VoiceController`
/// tries again (`restartSound`); a mic that still delivers nothing shows as
/// "Your mic is sending silence" (`VoiceEngine.micSilent`).
final class AudioSessionWatch: NSObject, RTCAudioSessionDelegate, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) var isInCall: (@MainActor () -> Bool)?
    nonisolated(unsafe) var highQualityBluetooth: (@MainActor () -> Bool)?
    /// Interruptions and failures, on the main thread.
    nonisolated(unsafe) var onEvent: (@MainActor (AudioSessionEvent) -> Void)?
    /// Route changes and resets, in words (the call report), on the main thread.
    nonisolated(unsafe) var onNote: (@MainActor (String) -> Void)?
    /// `restartSound` is going through WebRTC's interruption handling (main thread).
    nonisolated(unsafe) private var restarting = false

    /// Start the sound again the way WebRTC does when an interruption ends
    /// or Yap comes back on screen: activate the session and restart the
    /// voice-processing unit (`WebRTC-Audio-iOS-Holding`). WebRTC has no
    /// public call for that, so its session gets the notice iOS would post.
    @MainActor func restartSound() {
        restarting = true
        defer { restarting = false }
        NotificationCenter.default.post(name: AVAudioSession.interruptionNotification, object: AVAudioSession.sharedInstance(), userInfo: [
            AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue,
            AVAudioSessionInterruptionOptionKey: AVAudioSession.InterruptionOptions.shouldResume.rawValue,
        ])
    }

    private func note(_ text: String) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onNote?(text) }
        }
    }
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
        // (Delivered right away while `restartSound` posts, on the main thread.)
        if Thread.isMainThread, restarting { return }
        log.info("audio interruption ended (resume: \(shouldResumeSession))")
        tell(.interruptionEnded)
    }

    func audioSession(_ audioSession: RTCAudioSession, failedToSetActive active: Bool, error: any Error) {
        log.error("audio session didn't \(active ? "activate" : "deactivate", privacy: .public): \(error.localizedDescription, privacy: .public)")
        guard active else { return }
        note("The audio session wouldn't start: \(Self.describe(error))")
        tell(.failed)
    }

    func audioSession(_ audioSession: RTCAudioSession, audioUnitStartFailedWithError error: any Error) {
        log.error("the audio unit didn't start: \(error.localizedDescription, privacy: .public)")
        note("The audio unit wouldn't start: \(Self.describe(error))")
        tell(.failed)
    }

    /// An audio error for the call report: its code, and the four letters
    /// Core Audio codes stand for ('!pri': another app has priority).
    static func describe(_ error: any Error) -> String {
        let code = (error as NSError).code
        let bytes = withUnsafeBytes(of: UInt32(truncatingIfNeeded: code).bigEndian, Array.init)
        let letters = bytes.allSatisfy { (0x20...0x7e).contains($0) } ? " '\(String(decoding: bytes, as: UTF8.self))'" : ""
        return "error \(code)\(letters)"
    }

    func audioSessionDidChangeRoute(_ session: RTCAudioSession, reason: AVAudioSession.RouteChangeReason, previousRoute: AVAudioSessionRouteDescription) {
        let outputs = session.currentRoute.outputs.map(\.portType.rawValue).joined(separator: ",")
        let inputs = session.currentRoute.inputs.map(\.portType.rawValue).joined(separator: ",")
        log.info("audio route changed (\(reason.rawValue)): in \(inputs, privacy: .public) out \(outputs, privacy: .public)")
        let why = AudioRouteInfo.reason(reason)
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onNote?("Audio route (\(why)): \(AudioRouteInfo.describe() ?? "unknown")") }
        }
    }

    func audioSessionMediaServerReset(_ session: RTCAudioSession) {
        log.error("the media server was reset")
        note("iOS reset its audio (the media server); putting the call's sound back")
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
