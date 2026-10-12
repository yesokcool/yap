import Foundation
import Observation
import os
import YapCore
@preconcurrency import WebRTC

/// Voice = WebRTC mesh. Every pair of people in a channel has a direct
/// peer-to-peer connection carrying Opus; the server only relays signals.
/// This is the native twin of web/js/voice.js: who offers, perfect
/// negotiation, Opus tuning, the transmit gate, speaking levels, keeping
/// connections across reconnects, ICE restarts, connection hints and screen
/// sharing (video only to the people who ask, PROTOCOL.md › Screen sharing).
@MainActor
@Observable
public final class VoiceEngine {
    public enum InputMode: String, Codable, CaseIterable, Sendable, Identifiable {
        /// Transmit while your level is above the sensitivity threshold.
        case voiceActivity
        /// Transmit while the talk button is held.
        case pushToTalk
        /// Always transmit (the system's echo cancellation does the work).
        case openMic

        public var id: String { rawValue }
        public var title: String {
            switch self {
            case .voiceActivity: "Voice activity"
            case .pushToTalk: "Push to talk"
            case .openMic: "Always on"
            }
        }

        var gateMode: VoiceGate.Mode {
            switch self {
            case .voiceActivity: .voiceActivity
            case .pushToTalk: .pushToTalk
            case .openMic: .alwaysOn
            }
        }
    }

    public enum PeerState: String, Sendable {
        case new, connecting, connected, disconnected, failed, closed

        init(_ state: RTCPeerConnectionState) {
            switch state {
            case .new: self = .new
            case .connecting: self = .connecting
            case .connected: self = .connected
            case .disconnected: self = .disconnected
            case .failed: self = .failed
            case .closed: self = .closed
            @unknown default: self = .connecting
            }
        }
    }

    public struct Summary: Sendable, Equatable {
        public var rttMs: Int?
    }

    /// What happened to one connection (the call report's timeline).
    public enum PeerEvent: Sendable, Equatable {
        /// The path died and we restarted ICE (the nth time since it was last up).
        case iceRestart(Int)
        /// We threw the connection away and offered a new one.
        case rebuilt
        /// They started a new connection (a new DTLS fingerprint), so we did too.
        case freshStart
        /// "weak connection" shown / gone.
        case weak(Bool)
    }

    // MARK: Observable state

    /// Sessions (sids) currently speaking, including our own when transmitting.
    public private(set) var speaking: Set<String> = []
    public private(set) var peerStates: [String: PeerState] = [:]
    /// Our mic level: linear RMS (0…1 of full scale) over the last ~25 ms,
    /// while in a voice channel (alone too: see `LevelMonitor`).
    public private(set) var localLevel: Double = 0
    public private(set) var summary: Summary?
    /// People whose audio reaches us badly (PROTOCOL.md › Voice › Connection hints).
    public private(set) var weak: [String: LinkQuality.Weak] = [:]
    /// Connections whose ICE failed since they were last up (no route: maybe the relay is needed).
    public private(set) var failed: Set<String> = []
    /// Nothing but digital silence (or nothing at all) from the mic for 5 s.
    public private(set) var micSilent = false
    /// The mic's level stopped coming in: voice activity sends everything
    /// meanwhile (fail open) rather than nothing.
    public private(set) var micStalled = false

    public var mySid: String?
    public var mode: InputMode = .voiceActivity { didSet { applyGate() } }
    /// Opus target, bps (64k–510k; default 128k).
    public var bitrate: Int = 128_000
    /// Redundancy (RED) on the voice: each packet also carries the 20 ms
    /// before it, so a lost one is rebuilt word for word (PROTOCOL.md ›
    /// Voice › Redundancy). Off only to measure what it does.
    public var redundancy = true
    /// Voice-activity threshold, dBFS RMS (the web's and desktop's scale).
    public var vadThreshold: Double = -50
    /// Only through the TURN relay (hides your IP from the people you talk
    /// to). Set before joining; the controller only sets it when the server
    /// has a relay.
    public var relayOnly = false
    /// The server's `/api/ice` has a TURN relay ("can't connect" vs "needs the voice relay").
    public private(set) var hasRelay = false
    public private(set) var muted = false
    public private(set) var deafened = false
    /// Push to talk is down for the gate: held, or double-tapped to stay live.
    public private(set) var pushToTalkDown = false
    /// Push to talk was double-tapped and stays live until the next press
    /// ("Live — tap to stop").
    public private(set) var pushToTalkLatched = false
    /// "Your connection is dropping some of your audio": most people in the
    /// call lose more than 3 % of what we send (PROTOCOL.md › Voice ›
    /// Connection hints › Your own connection).
    public private(set) var ownLinkWeak = false
    /// What they lose of ours (the median person), 0…1, while that shows.
    public private(set) var ownLinkLost = 0.0
    /// The mic's peaks keep reaching −1 dBFS ("Too loud (clipping)").
    public private(set) var tooLoud = false
    /// "Lower shared media while friends talk": a screen share's sound eases
    /// to half of my volume while someone else in the call speaks.
    public var dipSharedMedia = false
    /// The transmit gate lets audio through (voice activity incl. its hold,
    /// push to talk incl. its tail, or always on).
    public private(set) var vadOpen = false
    /// Our socket is up. While it isn't, every connection keeps going and its
    /// signals wait (a call outlives the socket).
    public var online = true

    // MARK: Screen sharing state

    /// We're sharing our screen, at this quality (nil = not sharing).
    public private(set) var screenQuality: ScreenQuality?
    /// Sessions that asked for our screen and haven't said stop or gone
    /// away, in the order they asked (the sharer's "Watching: …").
    public private(set) var screenViewers: [String] = []
    /// Sharers we asked for their screen.
    public private(set) var watchingScreens: Set<String> = []
    /// sid → their screen's video, while it comes.
    public private(set) var remoteScreens: [String: RTCVideoTrack] = [:]
    /// Sharers whose screen comes with sound.
    public private(set) var remoteScreenSound: Set<String> = []

    // MARK: Internals

    @ObservationIgnored let factory: RTCPeerConnectionFactory
    @ObservationIgnored private(set) var audioTrack: RTCAudioTrack?
    @ObservationIgnored private(set) var iceServers: [RTCIceServer] = []
    @ObservationIgnored private var peers: [String: Peer] = [:]
    @ObservationIgnored private var levelTask: Task<Void, Never>?
    @ObservationIgnored private var statsTask: Task<Void, Never>?
    @ObservationIgnored private var gate = VoiceGate()
    /// Would voice activity open right now? Fed the same level as the gate in
    /// every mode, so talking shows while muted or on always on.
    @ObservationIgnored private var talkDetector = VoiceGate()
    /// Push to talk's press, release and double-tap.
    @ObservationIgnored private var latch = PushToTalkLatch()
    /// Push to talk is live right now (for the click when it changes).
    @ObservationIgnored private var pushToTalkLive = false
    @ObservationIgnored private var ownQuality = OwnLinkQuality()
    @ObservationIgnored private var clipWatch = ClipWatch()
    @ObservationIgnored private var dip = MediaDip()
    /// The dip's factor on a screen share's sound right now (1 = none).
    @ObservationIgnored private(set) var mediaDip = 1.0
    /// Alone in the channel: the loopback that keeps the mic measured.
    @ObservationIgnored private var monitor: LevelMonitor?
    /// iPhone: an audio stream that lasts the whole call, so the sound never
    /// stops while the people in it change (see `LevelMonitor`).
    @ObservationIgnored private var anchor: LevelMonitor?
    /// The mic's energy counters at the last poll (and which connection they came from).
    @ObservationIgnored private var energyBaseline: MicEnergy?
    /// When the mic last delivered audio, and since when we expect it to.
    @ObservationIgnored private var micProgressAt: TimeInterval?
    @ObservationIgnored private var iceFetchedAt: TimeInterval = -.infinity
    @ObservationIgnored public var onSignal: ((String, SignalData) -> Void)?
    /// uid for a sid (for per-person volume, and to check a resume).
    @ObservationIgnored public var uidOf: ((String) -> String?)?
    /// Does that session keep calls across reconnects (can it follow a rebuilt connection)?
    @ObservationIgnored public var resumable: ((String) -> Bool)?
    /// Fresh ICE servers for an ICE restart (relay credentials can expire).
    @ObservationIgnored public var fetchIce: (@MainActor () async -> [IceServerConfig])?
    /// A connection's state changed (nil = it's gone).
    @ObservationIgnored public var onPeerState: ((String, PeerState?) -> Void)?
    /// ICE restarts, rebuilds, "weak connection" (the call report).
    @ObservationIgnored public var onPeerEvent: ((String, PeerEvent) -> Void)?
    /// Every connection's stats, every 2 s (the call report).
    @ObservationIgnored public var onStats: (([PeerStats]) -> Void)?
    /// uid → gain (1 = normal, up to 2).
    @ObservationIgnored public var volumes: [String: Double] = [:]
    /// uid → their screen's sound (0…1), separate from their voice.
    @ObservationIgnored public var screenVolumes: [String: Double] = [:]
    /// Someone started (true) or stopped (false) watching our screen.
    @ObservationIgnored public var onScreenViewers: ((_ joined: Bool) -> Void)?
    /// We're muted (not deafened) and talking: the level measured before the
    /// mute would open the gate, or push to talk is pressed. Called on every
    /// level reading while it lasts and on each press; the controller keeps
    /// it to one nudge every 5 s.
    @ObservationIgnored public var onTalkWhileMuted: (() -> Void)?
    /// Push to talk went live (true) or closed after its tail (false): the click.
    @ObservationIgnored public var onPushToTalk: ((_ live: Bool) -> Void)?
    /// A connection moved to the far side's new session (old, new).
    @ObservationIgnored public var onRekey: ((String, String) -> Void)?
    /// What our screen share sends (frames come in through a `ScreenFeeder`).
    @ObservationIgnored private var screenSource: RTCVideoSource?
    @ObservationIgnored private(set) var screenTrack: RTCVideoTrack?
    @ObservationIgnored private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    /// How often the mic level, voice activity and speaking rings update.
    static let levelInterval: Duration = .milliseconds(25)
    static let remoteSpeakingLevel = 0.02
    static let remoteHold: TimeInterval = 0.25
    /// No level for this long while the mic should be running: fail open.
    static let stallAfter: TimeInterval = 1.5
    /// ICE servers are fetched again at most this often.
    static let iceRefetch: TimeInterval = 30

    private static let sslInitialized: Bool = {
        #if os(iOS)
        // Before the first factory. NWPathMonitor: WebRTC notices Wi-Fi ↔
        // cellular switches at once (with continual gathering, ICE follows).
        // iOS-Holding: after an interruption (a phone call, Siri) the audio
        // unit is restarted instead of trusting it to resume by itself.
        FieldTrials.apply([
            kRTCFieldTrialUseNWPathMonitor: kRTCFieldTrialEnabledValue,
            "WebRTC-Audio-iOS-Holding": kRTCFieldTrialEnabledValue,
        ])
        #endif
        RTCInitializeSSL()
        return true
    }()

    /// The default audio device: on iOS the system voice-processing unit
    /// (Apple's echo cancellation), on the Mac Core Audio.
    public init() {
        _ = Self.sslInitialized
        factory = RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory())
    }

    #if os(iOS)
    /// A custom audio device (tests use `SyntheticAudioDevice`: no mic needed).
    public init(audioDevice: any RTCAudioDevice) {
        _ = Self.sslInitialized
        factory = RTCPeerConnectionFactory(encoderFactory: RTCDefaultVideoEncoderFactory(), decoderFactory: RTCDefaultVideoDecoderFactory(), audioDevice: audioDevice)
    }
    #endif

    public var peerCount: Int { peers.count }
    public var peerSids: [String] { Array(peers.keys) }
    public var allConnected: Bool { peerStates.values.allSatisfy { $0 == .connected } }

    /// Get the mic track ready. Call right before joining (with fresh ICE servers).
    public func prepare(iceServers: [IceServerConfig]) {
        setIceServers(iceServers)
        if audioTrack == nil {
            let source = factory.audioSource(with: RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil))
            audioTrack = factory.audioTrack(with: source, trackId: "yap-mic")
            gate = VoiceGate(mode: mode.gateMode, threshold: vadThreshold)
            talkDetector = VoiceGate(mode: .voiceActivity, threshold: vadThreshold)
            clipWatch.reset()
            micProgressAt = nil
        }
        holdAudio()
        applyGate()
        startLoops()
        updateMonitor()
    }

    /// iPhone: the anchor starts with the call (in the foreground, where iOS
    /// lets the sound start) and stays until we leave. Without it the sound
    /// stopped for a moment whenever the people in the call changed, and in
    /// the background iOS paused Yap right there.
    private func holdAudio() {
        guard Self.anchorsAudio, anchor == nil, let created = LevelMonitor(factory: factory, track: nil) else { return }
        anchor = created
        Task { await created.start() }
    }

    /// The anchor is an iPhone thing (a Mac app is never paused for going quiet).
    static var anchorsAudio: Bool {
        #if os(iOS)
        true
        #else
        false
        #endif
    }

    /// The anchor's audio stream is open (tests).
    var holdsAudio: Bool { anchor != nil }

    private func setIceServers(_ servers: [IceServerConfig]) {
        iceServers = servers.map { RTCIceServer(urlStrings: $0.urls, username: $0.username, credential: $0.credential) }
        hasRelay = IceServerConfig.hasRelay(servers)
        iceFetchedAt = ProcessInfo.processInfo.systemUptime
    }

    /// A new connection's configuration.
    func configuration() -> RTCConfiguration {
        let config = RTCConfiguration()
        config.iceServers = iceServers
        config.sdpSemantics = .unifiedPlan
        config.bundlePolicy = .maxBundle
        config.rtcpMuxPolicy = .require
        // Phones hop between Wi-Fi and cellular; keep gathering so ICE can follow.
        config.continualGatheringPolicy = .gatherContinually
        // "Hide my IP address in calls": only the TURN relay's candidates.
        config.iceTransportPolicy = relayOnly ? .relay : .all
        return config
    }

    /// ICE servers for an ICE restart: fetched again (relay credentials can
    /// expire), at most every 30 s.
    func freshIceServers() async -> [RTCIceServer] {
        let now = ProcessInfo.processInfo.systemUptime
        if let fetchIce, now - iceFetchedAt > Self.iceRefetch {
            iceFetchedAt = now
            let servers = await fetchIce()
            if !servers.isEmpty { setIceServers(servers) }
        }
        return iceServers
    }

    // MARK: Peers

    /// We just joined: offer to everyone already in the channel.
    public func enter(_ sids: [String]) {
        for sid in sids where sid != mySid && peers[sid] == nil {
            createPeer(sid, initiator: true)
        }
        startLoops()
    }

    public func handleSignal(_ data: SignalData, from sid: String) {
        if case .screen(let want) = data {
            screenRequest(want, from: sid)
            return
        }
        var peer = peers[sid]
        if let existing = peer, case .sdp(let description) = data, description.type == "offer",
           let had = existing.pc.remoteDescription.flatMap({ SDPTuner.fingerprint(in: $0.sdp) }),
           let now = SDPTuner.fingerprint(in: description.sdp), had != now {
            // A different DTLS fingerprint = the far side started a brand-new
            // connection (it couldn't keep ours): start over too.
            log.info("fresh start from \(sid, privacy: .public)")
            replace(sid)
            peer = nil
            onPeerEvent?(sid, .freshStart)
        }
        if peer == nil {
            // Only an offer may open a new connection.
            guard case .sdp(let description) = data, description.type == "offer" else { return }
            peer = createPeer(sid, initiator: false)
        }
        peer?.handle(data)
    }

    public func drop(_ sid: String) {
        guard replace(sid) else { return }
        updateMonitor()
        onPeerState?(sid, nil)
    }

    /// Close a connection without telling anyone it's gone (it's about to be replaced).
    @discardableResult
    private func replace(_ sid: String) -> Bool {
        guard let peer = peers.removeValue(forKey: sid) else { return false }
        peer.close()
        peerStates.removeValue(forKey: sid)
        speaking.remove(sid)
        weak.removeValue(forKey: sid)
        failed.remove(sid)
        remoteScreens.removeValue(forKey: sid)
        remoteScreenSound.remove(sid)
        refreshViewers()
        return true
    }

    /// Leave every connection but keep the mic (socket reconnecting).
    public func hangUpAll() {
        for sid in Array(peers.keys) { drop(sid) }
        summary = nil
        online = true
    }

    /// Fully off: no peers, mic released, no share, nothing watched.
    public func stop() {
        stopScreen()
        watchingScreens = []
        audioTrack?.isEnabled = false
        audioTrack = nil
        hangUpAll()
        updateMonitor()
        // Last: the sound may stop now (we've left the call).
        anchor?.close()
        anchor = nil
        levelTask?.cancel()
        levelTask = nil
        statsTask?.cancel()
        statsTask = nil
        energyBaseline = nil
        micProgressAt = nil
        speaking = []
        localLevel = 0
        vadOpen = false
        pushToTalkDown = false
        latch.reset()
        pushToTalkLatched = false
        pushToTalkLive = false
        micSilent = false
        micStalled = false
        ownQuality.reset()
        ownLinkWeak = false
        ownLinkLost = 0
        clipWatch.reset()
        tooLoud = false
        dip = MediaDip()
        mediaDip = 1
    }

    @discardableResult
    private func createPeer(_ sid: String, initiator: Bool) -> Peer? {
        guard let peer = Peer(engine: self, sid: sid, initiator: initiator) else {
            log.error("could not create a peer connection")
            return nil
        }
        peers[sid] = peer
        peerStates[sid] = .new
        updateMonitor()
        return peer
    }

    // MARK: Keeping calls across reconnects

    /// Move a connection to the far side's new session id (they continue the
    /// same call). Same RTCPeerConnection: no new ICE, no new DTLS, no gap.
    /// Returns false if there was nothing to move.
    @discardableResult
    public func rekey(from old: String, to new: String) -> Bool {
        guard old != new, let peer = peers[old] else { return false }
        if peers[new] != nil {
            // Already connected to the new session (it offered first): the old one is spare.
            drop(old)
            return false
        }
        peers.removeValue(forKey: old)
        peer.sid = new
        peers[new] = peer
        if let state = peerStates.removeValue(forKey: old) { peerStates[new] = state }
        speaking.remove(old)
        if let w = weak.removeValue(forKey: old) { weak[new] = w }
        if failed.remove(old) != nil { failed.insert(new) }
        if let screen = remoteScreens.removeValue(forKey: old) { remoteScreens[new] = screen }
        if remoteScreenSound.remove(old) != nil { remoteScreenSound.insert(new) }
        if watchingScreens.remove(old) != nil { watchingScreens.insert(new) }
        screenViewers = screenViewers.map { $0 == old ? new : $0 }
        peer.held = false
        if online { peer.resync() }
        onPeerState?(old, nil)
        onPeerState?(new, peer.state)
        onRekey?(old, new)
        return true
    }

    /// Their socket is down (or back): their signals wait meanwhile.
    public func setHeld(_ sid: String, _ held: Bool) {
        guard let peer = peers[sid], peer.held != held else { return }
        peer.held = held
        if !held { peer.resync() }
    }

    /// Our socket is back: send what waited for this connection (and an
    /// unanswered offer again).
    public func resync(_ sid: String) {
        peers[sid]?.resync()
    }

    /// Throw a connection away and offer a brand-new one (the far side sees
    /// a new DTLS fingerprint and follows).
    public func restart(_ sid: String) {
        guard replace(sid) else { return }
        createPeer(sid, initiator: true)
        onPeerState?(sid, .new)
        onPeerEvent?(sid, .rebuilt)
    }

    func peerEvent(_ peer: Peer, _ event: PeerEvent) {
        guard peers[peer.sid] === peer else { return }
        onPeerEvent?(peer.sid, event)
    }

    public func peerUid(_ sid: String) -> String? { peers[sid]?.uid }

    /// The loopback monitor runs while we have a mic and nobody to call.
    private func updateMonitor() {
        if let track = audioTrack, peers.isEmpty {
            guard monitor == nil, let created = LevelMonitor(factory: factory, track: track) else { return }
            monitor = created
            energyBaseline = nil
            micProgressAt = nil
            Task { await created.start() }
        } else if let monitor {
            monitor.close()
            self.monitor = nil
            energyBaseline = nil
            micProgressAt = nil
        }
    }

    func sendSignal(to sid: String, _ data: SignalData) {
        onSignal?(sid, data)
    }

    func peerStateChanged(_ peer: Peer) {
        guard peers[peer.sid] === peer else { return }
        peerStates[peer.sid] = peer.state
        if peer.failed, peer.state != .connected { failed.insert(peer.sid) } else { failed.remove(peer.sid) }
        if peer.state != .connected, weak.removeValue(forKey: peer.sid) != nil { onPeerEvent?(peer.sid, .weak(false)) }
        onPeerState?(peer.sid, peer.state)
    }

    func volume(forSid sid: String) -> Double {
        guard let uid = uidOf?(sid) else { return 1 }
        return volumes[uid] ?? 1
    }

    func screenVolume(forSid sid: String) -> Double {
        guard let uid = uidOf?(sid) ?? peers[sid]?.uid else { return 1 }
        return screenVolumes[uid] ?? 1
    }

    // MARK: Screen sharing (PROTOCOL.md › Screen sharing)

    /// A source to feed our screen into (a `ScreenFeeder` pushes frames).
    /// Marked as a screen cast, so WebRTC tunes for text and slides.
    public func makeScreenSource() -> RTCVideoSource {
        let source = factory.videoSource(forScreenCast: true)
        screenSource = source
        screenTrack = nil
        return source
    }

    /// Start sending our screen to everyone who asked (and whoever asks later).
    public func startScreen(quality: ScreenQuality) {
        guard let source = screenSource else { return }
        let track = screenTrack ?? factory.videoTrack(with: source, trackId: "yap-screen")
        screenTrack = track
        screenQuality = quality
        for peer in peers.values where peer.wantsScreen { peer.addScreen(track) }
        refreshViewers()
    }

    /// Smooth / Sharp / Max while sharing: each viewer's sender follows at once.
    public func setScreenQuality(_ quality: ScreenQuality) {
        guard screenQuality != nil, screenQuality != quality else { return }
        screenQuality = quality
        for peer in peers.values { peer.applySenderParameters() }
    }

    /// Stop sharing: every viewer's video goes (each connection renegotiates).
    public func stopScreen() {
        guard screenQuality != nil || screenSource != nil else { return }
        screenQuality = nil
        for peer in peers.values { peer.removeScreen() }
        screenTrack = nil
        screenSource = nil
        refreshViewers()
    }

    /// Ask a sharer for their screen (`{screen: "want"}`), or stop. Asked
    /// again whenever the connection (re)connects.
    public func watchScreen(_ sid: String, _ on: Bool) {
        if on {
            watchingScreens.insert(sid)
            if let peer = peers[sid], peer.state == .connected { peer.send(.screen(true)) }
        } else {
            guard watchingScreens.remove(sid) != nil else { return }
            peers[sid]?.send(.screen(false))
        }
    }

    /// A viewer's want / stop.
    private func screenRequest(_ want: Bool, from sid: String) {
        guard let peer = peers[sid] else { return }
        peer.wantsScreen = want
        if let track = screenTrack, screenQuality != nil {
            if want { peer.addScreen(track) } else { peer.removeScreen() }
        }
        refreshViewers()
    }

    /// Who's watching: only while we share, only connections that are still here.
    private func refreshViewers() {
        let now = screenQuality == nil ? [] : peers.values.filter(\.wantsScreen).sorted { $0.wantedAt < $1.wantedAt }.map(\.sid)
        guard now != screenViewers else { return }
        let joined = Set(now).subtracting(screenViewers).isEmpty == false
        screenViewers = now
        onScreenViewers?(joined)
    }

    /// A connection's incoming screen (or its sound) came or went.
    func remoteMediaChanged(_ peer: Peer) {
        guard peers[peer.sid] === peer else { return }
        if let track = peer.remoteScreen {
            if remoteScreens[peer.sid] !== track { remoteScreens[peer.sid] = track }
        } else if remoteScreens[peer.sid] != nil {
            remoteScreens.removeValue(forKey: peer.sid)
        }
        if peer.remoteScreenSound != remoteScreenSound.contains(peer.sid) {
            if peer.remoteScreenSound { remoteScreenSound.insert(peer.sid) } else { remoteScreenSound.remove(peer.sid) }
        }
        peer.applyOutput()
    }

    /// Re-apply the share's sound volumes (the viewer's slider moved).
    public func applyScreenVolumes() {
        for peer in peers.values { peer.applyOutput() }
    }

    /// The volume their screen's sound plays at right now (tests).
    func screenSoundVolume(of sid: String) -> Double? { peers[sid]?.screenSoundVolume }

    /// Is our screen going out on this connection (tests)?
    public func sendsScreen(to sid: String) -> Bool { peers[sid]?.sendsScreen ?? false }

    /// A connection's current local SDP (tests).
    func peerLocalDescription(_ sid: String) -> String? { peers[sid]?.pc.localDescription?.sdp }

    /// A connection's RTCPeerConnection (tests).
    func peerConnection(_ sid: String) -> RTCPeerConnection? { peers[sid]?.pc }

    // MARK: Transmit gate (mute / deafen / voice activity / push-to-talk)

    public func setMuted(_ muted: Bool) {
        self.muted = muted
        applyGate()
    }

    public func setDeafened(_ deafened: Bool) {
        self.deafened = deafened
        for peer in peers.values { peer.applyOutput() }
        applyGate()
    }

    /// The push-to-talk button (or key) went down or up. Twice within 400 ms
    /// stays live until the next press (PROTOCOL.md › Voice › Transmit gate).
    public func setPushToTalk(_ down: Bool) {
        guard latch.held != down else { return }
        if down { latch.press(at: Self.now) } else { latch.release() }
        if down, mode == .pushToTalk, muted, !deafened { onTalkWhileMuted?() }
        applyPushToTalk()
    }

    /// Nothing held, nothing latched (a new input mode).
    public func resetPushToTalk() {
        latch.reset()
        applyPushToTalk()
    }

    private func applyPushToTalk() {
        if latch.latched != pushToTalkLatched { pushToTalkLatched = latch.latched }
        let down = latch.isDown
        if down != pushToTalkDown {
            pushToTalkDown = down
            gate.setPushToTalk(down, at: Self.now)
        }
        applyGate()
    }

    /// Re-apply bitrate (renegotiates so the far side sees our new fmtp) and volumes.
    public func applySettings(renegotiate: Bool) {
        for peer in peers.values {
            peer.applySenderParameters()
            peer.applyOutput()
            if renegotiate { Task { await peer.renegotiate() } }
        }
    }

    public var transmitting: Bool {
        guard audioTrack != nil, !muted, !deafened else { return false }
        return vadOpen
    }

    private func applyGate() {
        gate.threshold = vadThreshold
        var open = gate.setMode(mode.gateMode, at: Self.now)
        // Fail open: no level to decide on, so send rather than cut you off.
        if mode == .voiceActivity, micStalled { open = true }
        if open != vadOpen { vadOpen = open }
        // Mute and deafen are certain, whatever the gate says.
        let on = transmitting
        if let audioTrack, audioTrack.isEnabled != on { audioTrack.isEnabled = on }
        // Push to talk's click: live, and closed again after its tail (never
        // for a switch to another mode).
        let live = mode == .pushToTalk && on
        if live != pushToTalkLive {
            pushToTalkLive = live
            if mode == .pushToTalk { onPushToTalk?(live) }
        }
        updateSelfSpeaking()
    }

    private func updateSelfSpeaking() {
        guard let mySid else { return }
        let isSpeaking = transmitting && (mode == .voiceActivity ? !micStalled : VoiceGate.dB(localLevel) > vadThreshold)
        if isSpeaking != speaking.contains(mySid) {
            if isSpeaking { speaking.insert(mySid) } else { speaking.remove(mySid) }
        }
    }

    /// What the gate opens at right now (your sensitivity, or higher in a noisy room), dBFS.
    public var openAt: Double { gate.openAt }

    // MARK: Levels + stats

    private static var now: TimeInterval { ProcessInfo.processInfo.systemUptime }

    private func startLoops() {
        // Each loop ends with the engine (not only on stop()).
        if levelTask == nil {
            levelTask = Task { [weak self] in
                while !Task.isCancelled {
                    guard let engine = self else { return }
                    await engine.pollLevels()
                    try? await Task.sleep(for: Self.levelInterval)
                }
            }
        }
        if statsTask == nil {
            statsTask = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(2))
                    guard let engine = self else { return }
                    await engine.pollStats()
                }
            }
        }
    }

    /// WebRTC's media-source counters for the mic: Σ level² × seconds, and
    /// seconds. Their deltas give the level over any interval.
    struct MicEnergy {
        var source: ObjectIdentifier
        var energy: Double
        var duration: Double
        /// The loudest sample of the last ~110 ms (WebRTC's `audioLevel`), 0…1.
        var peak: Double?
    }

    /// From a connected call (any: the counters are the mic's, not the
    /// call's), else from the monitor while alone.
    private func micEnergy() async -> MicEnergy? {
        let pc: RTCPeerConnection
        let sender: RTCRtpSender
        if let peer = peers.values.first(where: { $0.state == .connected }), let audio = peer.audioSender {
            (pc, sender) = (peer.pc, audio)
        } else if let monitor, monitor.isConnected, let audio = monitor.audioSender {
            (pc, sender) = (monitor.sender, audio)
        } else {
            return nil
        }
        let report = await pc.statistics(for: sender)
        for stat in report.statistics.values where stat.type == "media-source" && stat.values["kind"] as? String == "audio" {
            guard let energy = (stat.values["totalAudioEnergy"] as? NSNumber)?.doubleValue,
                  let duration = (stat.values["totalSamplesDuration"] as? NSNumber)?.doubleValue else { continue }
            return MicEnergy(source: ObjectIdentifier(pc), energy: energy, duration: duration,
                             peak: (stat.values["audioLevel"] as? NSNumber)?.doubleValue)
        }
        return nil
    }

    private func pollLevels() async {
        let date = Date()
        for peer in peers.values {
            let level = peer.remoteAudioLevel(now: date)
            if level > Self.remoteSpeakingLevel { peer.speakingUntil = date.addingTimeInterval(Self.remoteHold) }
            let isSpeaking = date < peer.speakingUntil && !deafened
            if isSpeaking != speaking.contains(peer.sid) {
                if isSpeaking { speaking.insert(peer.sid) } else { speaking.remove(peer.sid) }
            }
            // A boost past 100 % follows how loud they are right now.
            if volume(forSid: peer.sid) > 1 { peer.applyOutput() }
        }
        // Our own level since the last poll, from the mic's counters (measured
        // before the transmit gate, so voice activity can reopen it), on the
        // web's RMS scale.
        let sample = await micEnergy()
        let now = Self.now
        if let sample {
            if let last = energyBaseline, last.source == sample.source,
               let peakRMS = VoiceGate.rms(fromEnergy: last.energy, duration: last.duration, toEnergy: sample.energy, duration: sample.duration) {
                let db = peakRMS > 0 ? VoiceGate.dB(peakRMS) + VoiceGate.peakToRMSDb : VoiceGate.silenceDb
                let level = VoiceGate.linear(db)
                if level != localLevel { localLevel = level }
                gate.threshold = vadThreshold
                gate.update(db: db, duration: sample.duration - last.duration, at: now)
                talkDetector.threshold = vadThreshold
                talkDetector.update(db: db, duration: sample.duration - last.duration, at: now)
                if let peak = sample.peak {
                    let loud = clipWatch.update(peakDb: VoiceGate.dB(peak), at: now)
                    if loud != tooLoud { tooLoud = loud }
                }
                micProgressAt = now
            } else if energyBaseline?.source != sample.source {
                micProgressAt = micProgressAt ?? now
            }
            energyBaseline = sample
        } else {
            energyBaseline = nil
            micProgressAt = nil
            if localLevel != 0 { localLevel = 0 }
        }
        // Expecting the mic (something is sending it) but nothing new from it.
        let quietFor = micProgressAt.map { now - $0 } ?? 0
        let stalled = audioTrack != nil && quietFor > Self.stallAfter
        if stalled != micStalled {
            micStalled = stalled
            log.info("mic level \(stalled ? "stalled: failing open" : "back", privacy: .public)")
        }
        let silent = audioTrack != nil && (gate.isSilent || quietFor > VoiceGate.silenceAfter)
        if silent != micSilent { micSilent = silent }
        if sample == nil, tooLoud {
            clipWatch.reset()
            tooLoud = false
        }
        applyGate()
        // Talking while muted: the gate would open on the level before the
        // mute (whatever the mode), or push to talk is down.
        let talking = mode == .pushToTalk ? latch.isDown : (!micStalled && talkDetector.decide(at: now))
        if talking, muted, !deafened, audioTrack != nil { onTalkWhileMuted?() }
        updateDip(at: now)
    }

    /// "Lower shared media while friends talk": anyone else I can hear speaking
    /// eases a screen share's sound to half (never the saved volume).
    private func updateDip(at now: TimeInterval) {
        let others = dipSharedMedia && peers.values.contains { speaking.contains($0.sid) && volume(forSid: $0.sid) > 0 }
        let factor = dip.update(on: dipSharedMedia, othersSpeaking: others, at: now)
        guard abs(factor - mediaDip) > 0.004 || (factor == 1 && mediaDip != 1) else { return }
        mediaDip = factor
        for peer in peers.values where peer.remoteScreenSound { peer.applyOutput() }
    }

    private func pollStats() async {
        var rtt: [Double] = []
        var all: [PeerStats] = []
        var losses: [Double] = []
        let now = Date()
        for peer in peers.values {
            let stats = await peer.stats()
            log.debug("peer \(peer.sid, privacy: .public): in \(Int(stats.bytesReceived)) B, inbound level \(stats.inboundAudioLevel), sources \(peer.sourceCount)")
            if peer.state == .connected, let r = stats.rttMs { rtt.append(r) }
            guard peers[peer.sid] === peer else { continue }
            all.append(stats)
            if let loss = peer.ownLoss(stats, at: now) { losses.append(loss) }
            let wasWeak = weak[peer.sid] != nil
            if peer.judgeQuality(stats, at: now) {
                if let w = peer.quality.weak { weak[peer.sid] = w } else { weak.removeValue(forKey: peer.sid) }
            } else if let w = peer.quality.weak, weak[peer.sid] != w {
                weak[peer.sid] = w
            }
            if (weak[peer.sid] != nil) != wasWeak { onPeerEvent?(peer.sid, .weak(!wasWeak)) }
        }
        // Our own connection: what most people lose of ours.
        if peers.isEmpty { ownQuality.reset() } else { ownQuality.update(losses: losses) }
        if ownQuality.weak != ownLinkWeak { ownLinkWeak = ownQuality.weak }
        if ownQuality.lost != ownLinkLost { ownLinkLost = ownQuality.lost }
        if !all.isEmpty { onStats?(all) }
        guard !peers.isEmpty else {
            summary = nil
            return
        }
        summary = Summary(rttMs: rtt.isEmpty ? nil : Int((rtt.reduce(0, +) / Double(rtt.count)).rounded()))
    }

    /// Per-peer stats (tests and the debug panel).
    public func peerStats() async -> [PeerStats] {
        var out: [PeerStats] = []
        for peer in peers.values { out.append(await peer.stats()) }
        return out
    }

    public func connectionState(of sid: String) -> PeerState? { peerStates[sid] }

    /// ICE restarts since the connection was last up (tests, debugging).
    public func iceRestarts(of sid: String) -> Int { peers[sid]?.restarts ?? 0 }
}

/// libwebrtc's field trials. The Objective-C API marks the global setter
/// deprecated in favour of passing them to the factory, which this SDK's
/// factory initializers can't do yet; it's looked up at run time so the
/// build stays free of warnings.
enum FieldTrials {
    typealias Setter = @convention(c) (UnsafeRawPointer) -> Void

    /// `RTCInitFieldTrialDictionary`, if this WebRTC build still has it (RTLD_DEFAULT = -2).
    static var setter: Setter? {
        dlsym(UnsafeMutableRawPointer(bitPattern: -2), "RTCInitFieldTrialDictionary").map { unsafeBitCast($0, to: Setter.self) }
    }

    static func apply(_ trials: [String: String]) {
        guard let setter else {
            Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice").error("no field trials in this WebRTC build")
            return
        }
        let dictionary = trials as NSDictionary
        withExtendedLifetime(dictionary) {
            setter(Unmanaged.passUnretained(dictionary).toOpaque())
        }
    }
}
