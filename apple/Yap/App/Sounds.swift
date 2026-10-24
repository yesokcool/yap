import AVFoundation
import Foundation
import Observation
import YapCore
import YapVoice

/// Per-device sound preferences (Settings › Voice), like the web's
/// `settings.sounds` and the soundboard's volume rule.
enum SoundSettings {
    static let soundsKey = "yap.sounds"
    static let volumeKey = "yap.soundboardVolume"
    static let mutedKey = "yap.soundboardMuted"

    /// Yap's little blips and everyone's join chimes.
    static var sounds: Bool {
        UserDefaults.standard.object(forKey: soundsKey) == nil ? true : UserDefaults.standard.bool(forKey: soundsKey)
    }

    /// 0…1, default 50 %: never above a clip's measured gain.
    static var volume: Double {
        UserDefaults.standard.object(forKey: volumeKey) == nil ? Soundboard.defaultVolume : UserDefaults.standard.double(forKey: volumeKey)
    }

    static var muted: Bool { UserDefaults.standard.bool(forKey: mutedKey) }
}

/// Everything Yap plays itself, synthesized or tiny: the join/leave blips,
/// people's join chimes (PROTOCOL.md › Join chimes) and soundboard clips
/// (› Soundboard). One AVAudioEngine: players → a mixer → a limiter at
/// −12 dB with a fast attack → out. At most two clips at once (a third
/// stops the oldest); nothing while deafened; the soundboard has its own
/// volume (default 50 %, never above the clip's measured gain) and a mute
/// switch; plays from people I've turned to 0 % are ignored. The engine
/// stops a few seconds after the last sound, so it costs nothing idle.
@MainActor
@Observable
final class SoundPlayer {
    /// The clip being previewed or played, for a "playing" state in pickers.
    private(set) var playing: Set<String> = []

    @ObservationIgnored private let engine = AVAudioEngine()
    @ObservationIgnored private let mixer = AVAudioMixerNode()
    @ObservationIgnored private let limiter: AVAudioUnitEffect
    @ObservationIgnored private var clipPlayers: [(id: String, node: AVAudioPlayerNode)] = []
    @ObservationIgnored private var tonePlayers: [AVAudioPlayerNode] = []
    @ObservationIgnored private var tones: [String: AVAudioPCMBuffer] = [:]
    @ObservationIgnored private var clips: [String: AVAudioPCMBuffer] = [:]
    @ObservationIgnored private var idleStop: Task<Void, Never>?
    @ObservationIgnored private var wired = false
    /// 48 kHz mono float: the tones' format; clips convert in the mixer.
    @ObservationIgnored private let toneFormat = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 1)!

    init() {
        let description = AudioComponentDescription(componentType: kAudioUnitType_Effect, componentSubType: kAudioUnitSubType_DynamicsProcessor,
                                                    componentManufacturer: kAudioUnitManufacturer_Apple, componentFlags: 0, componentFlagsMask: 0)
        limiter = AVAudioUnitEffect(audioComponentDescription: description)
    }

    // MARK: What to play

    /// A built-in tone (a chime or one of Yap's blips).
    func play(_ tone: ChimeTone, gain: Double = 1) {
        guard !tone.notes.isEmpty else { return }
        let buffer = tones[tone.id] ?? makeBuffer(tone.render(sampleRate: 48_000), format: toneFormat)
        guard let buffer else { return }
        tones[tone.id] = buffer
        start()
        let node = AVAudioPlayerNode()
        attach(node, format: buffer.format)
        tonePlayers.append(node)
        node.volume = Float(max(0, min(1, gain)))
        let key = ObjectIdentifier(node)
        node.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) { @Sendable [weak self] _ in
            Task { @MainActor in self?.finished(tone: key) }
        }
        node.play()
    }

    /// A soundboard clip or an uploaded chime, at `gain` (0…1, already the
    /// clip's measured gain × my volume). Fetched once, then cached.
    func play(clip sound: Sound, gain: Double, from endpoint: ServerEndpoint) {
        guard gain > 0 else { return }
        Task { [weak self] in
            guard let self, let buffer = await self.clipBuffer(sound, endpoint: endpoint) else { return }
            self.startClip(buffer, id: sound.id, gain: gain)
        }
    }

    private func startClip(_ buffer: AVAudioPCMBuffer, id: String, gain: Double) {
        start()
        // Two at once at most: a third stops the oldest.
        while clipPlayers.count >= Soundboard.maxConcurrent {
            let oldest = clipPlayers.removeFirst()
            oldest.node.stop()
            detach(oldest.node)
            playing.remove(oldest.id)
        }
        let node = AVAudioPlayerNode()
        attach(node, format: buffer.format)
        clipPlayers.append((id, node))
        playing.insert(id)
        node.volume = Float(max(0, min(1, gain)))
        let key = ObjectIdentifier(node)
        node.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) { @Sendable [weak self] _ in
            Task { @MainActor in self?.finished(clip: key) }
        }
        node.play()
    }

    /// A clip being trimmed before upload, at my soundboard volume.
    func preview(samples: [Float], sampleRate: Double, gain: Double) {
        guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
              let buffer = makeBuffer(samples, format: format) else { return }
        stopClips()
        start()
        let node = AVAudioPlayerNode()
        attach(node, format: format)
        clipPlayers.append(("preview", node))
        playing.insert("preview")
        node.volume = Float(max(0, min(1, gain)))
        let key = ObjectIdentifier(node)
        node.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) { @Sendable [weak self] _ in
            Task { @MainActor in self?.finished(clip: key) }
        }
        node.play()
    }

    /// Stop every clip (the soundboard's mute switch, deafening).
    func stopClips() {
        for player in clipPlayers {
            player.node.stop()
            detach(player.node)
        }
        clipPlayers = []
        playing = []
    }

    /// Prefetch a clip so the first play is instant.
    func warm(_ sound: Sound, endpoint: ServerEndpoint) {
        Task { _ = await clipBuffer(sound, endpoint: endpoint) }
    }

    // MARK: Engine

    private func start() {
        idleStop?.cancel()
        if !wired {
            wired = true
            engine.attach(mixer)
            engine.attach(limiter)
            engine.connect(mixer, to: limiter, format: nil)
            engine.connect(limiter, to: engine.mainMixerNode, format: nil)
            configureLimiter()
            NotificationCenter.default.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
                // A route change (headphones, a call starting) stops the engine; the next sound restarts it.
                MainActor.assumeIsolated { self?.engineReset() }
            }
        }
        guard !engine.isRunning else { return }
        #if os(iOS)
        // Outside calls the app's session is ambient (never stops someone's
        // music); in a call WebRTC owns it and it's already active.
        if AVAudioSession.sharedInstance().category == .ambient { try? AVAudioSession.sharedInstance().setActive(true) }
        #endif
        engine.prepare()
        try? engine.start()
    }

    /// Threshold −12 dB, almost no headroom (a limiter, not a compressor), fast attack.
    private func configureLimiter() {
        let unit = limiter.audioUnit
        AudioUnitSetParameter(unit, kDynamicsProcessorParam_Threshold, kAudioUnitScope_Global, 0, AudioUnitParameterValue(Soundboard.limiterThreshold), 0)
        AudioUnitSetParameter(unit, kDynamicsProcessorParam_HeadRoom, kAudioUnitScope_Global, 0, 0.1, 0)
        AudioUnitSetParameter(unit, kDynamicsProcessorParam_AttackTime, kAudioUnitScope_Global, 0, 0.001, 0)
        AudioUnitSetParameter(unit, kDynamicsProcessorParam_ReleaseTime, kAudioUnitScope_Global, 0, 0.08, 0)
    }

    private func engineReset() {
        for player in clipPlayers { detach(player.node) }
        for node in tonePlayers { detach(node) }
        clipPlayers = []
        tonePlayers = []
        playing = []
    }

    private func attach(_ node: AVAudioPlayerNode, format: AVAudioFormat) {
        engine.attach(node)
        engine.connect(node, to: mixer, format: format)
    }

    private func detach(_ node: AVAudioPlayerNode) {
        guard node.engine != nil else { return }
        engine.detach(node)
    }

    private func finished(tone key: ObjectIdentifier) {
        if let i = tonePlayers.firstIndex(where: { ObjectIdentifier($0) == key }) {
            detach(tonePlayers.remove(at: i))
        }
        scheduleIdleStop()
    }

    private func finished(clip key: ObjectIdentifier) {
        if let i = clipPlayers.firstIndex(where: { ObjectIdentifier($0.node) == key }) {
            playing.remove(clipPlayers[i].id)
            detach(clipPlayers.remove(at: i).node)
        }
        scheduleIdleStop()
    }

    private func scheduleIdleStop() {
        guard clipPlayers.isEmpty, tonePlayers.isEmpty else { return }
        idleStop?.cancel()
        idleStop = Task { [weak self] in
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled, let self, self.clipPlayers.isEmpty, self.tonePlayers.isEmpty else { return }
            self.engine.stop()
        }
    }

    // MARK: Buffers

    private func makeBuffer(_ samples: [Float], format: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard !samples.isEmpty, let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)),
              let channel = buffer.floatChannelData?[0] else { return nil }
        samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        return buffer
    }

    private func clipBuffer(_ sound: Sound, endpoint: ServerEndpoint) async -> AVAudioPCMBuffer? {
        if let hit = clips[sound.media] { return hit }
        guard let (data, response) = try? await URLSession.shared.data(from: endpoint.mediaURL(sound.media)),
              ((response as? HTTPURLResponse)?.statusCode ?? 200) < 400,
              let wav = WAV.decode(data),
              let format = AVAudioFormat(standardFormatWithSampleRate: Double(wav.sampleRate), channels: 1),
              let buffer = makeBuffer(wav.samples, format: format) else { return nil }
        clips[sound.media] = buffer
        return buffer
    }
}

/// When to play what (the rules live here; `SoundPlayer` just plays).
extension AppModel {
    /// Someone played a soundboard clip in my voice channel.
    func soundPlayed(_ sound: Sound, by uid: String) {
        guard let endpoint = client.endpoint,
              Soundboard.shouldPlay(muted: SoundSettings.muted, deafened: voice.settings.deafened,
                                    personVolume: uid == client.uid ? nil : voice.settings.volumes[uid]) else { return }
        sounds.play(clip: sound, gain: Soundboard.playbackGain(clipGain: sound.gain, volume: SoundSettings.volume), from: endpoint)
    }

    /// Someone's join chime (or the generic blip), through the same cap.
    func playChime(of uid: String) {
        guard SoundSettings.sounds, !voice.settings.deafened, (voice.settings.volumes[uid] ?? 1) > 0 else { return }
        playChime(ChimeChoice.from(client.user(uid)?.chime))
    }

    /// A chime choice as a preview or for real, like the web's `playChime`:
    /// built-ins at their table volume, clips at their measured gain × my
    /// soundboard volume (the soundboard's mute switch doesn't silence
    /// chimes; Sounds off and deafening do), all through the limiter.
    func playChime(_ choice: ChimeChoice) {
        switch choice {
        case .builtIn(let tone):
            sounds.play(tone)
        case .clip(let id):
            if let sound = client.sounds[id], let endpoint = client.endpoint {
                sounds.play(clip: sound, gain: Soundboard.playbackGain(clipGain: sound.gain, volume: SoundSettings.volume), from: endpoint)
            } else {
                sounds.play(ChimeTone.all[0])
            }
        }
    }

    /// Yap's own blips (join/leave, mute) when Sounds is on.
    func blip(_ tone: ChimeTone) {
        guard SoundSettings.sounds else { return }
        sounds.play(tone)
    }

    /// Voice events → sounds (like the web's `sounds.js`).
    func voiceEvent(_ event: VoiceEvent) {
        switch event {
        case .joined: blip(.join)
        case .left: blip(.leave)
        case .peerJoined(let uid): playChime(of: uid)
        case .peerLeft: if !voice.settings.deafened { blip(.peerLeave) }
        case .quieted(let quiet):
            if quiet, voice.settings.deafened { sounds.stopClips() }
            blip(quiet ? .mute : .unmute)
        case .talkingWhileMuted(let blip, let firstTime):
            // Its own setting ("Remind me when I talk while muted"), not Sounds.
            if blip { sounds.play(.mutedNudge) }
            if firstTime { client.notice = YapClient.Notice(text: TalkWords.explainNudge) }
        case .pushToTalk(let live):
            // Its own setting ("Push-to-talk click"), for my ears only.
            sounds.play(live ? .pushToTalkOn : .pushToTalkOff)
        }
    }
}

/// Adding a soundboard clip or my own join chime: pick a file (a voice
/// memo, an mp3…), trim it to ≤ 5 s (≤ 2 s for a chime), preview, upload.
/// `SoundUpload98` draws the sheet; this holds the work.
@MainActor
@Observable
final class SoundUpload {
    enum Kind: String, Identifiable {
        case board, chime
        var id: String { rawValue }
    }

    let kind: Kind
    var clip: SoundClip?
    /// Where the kept part starts, seconds.
    var start: Double = 0
    var name = ""
    var emoji = ""
    var error: String?
    var working = false

    init(kind: Kind) { self.kind = kind }

    var maxSeconds: Double { kind == .chime ? Soundboard.maxChimeSeconds : Soundboard.maxBoardSeconds }
    /// How long the kept part is.
    var length: Double { clip.map { min(maxSeconds, max(0, $0.seconds - start)) } ?? 0 }
    var latestStart: Double { clip.map { max(0, $0.seconds - 0.25) } ?? 0 }
    var canUpload: Bool {
        clip != nil && !working && length > 0.05 && (kind == .chime || !name.isBlank)
    }

    /// A file from the importer (security-scoped on iOS).
    func load(_ url: URL) {
        error = nil
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            let loaded = try SoundClip.load(url)
            clip = loaded
            // Like the web: start just before the first sound (leading silence trimmed).
            start = loaded.firstSound
            if name.isEmpty, kind == .board { name = String(loaded.name.prefix(Soundboard.maxNameLength)) }
        } catch {
            clip = nil
            self.error = error.localizedDescription
        }
    }

    func preview(on player: SoundPlayer) {
        guard let clip else { return }
        let cut = WAV.trim(clip.samples, sampleRate: SoundClip.sampleRate, start: start, seconds: maxSeconds)
        player.preview(samples: cut, sampleRate: SoundClip.sampleRate, gain: SoundSettings.volume)
    }

    /// True when it's up (the sheet closes).
    func upload(_ client: YapClient) async -> Bool {
        guard let clip else { return false }
        working = true
        defer { working = false }
        do {
            let (wav, _) = try clip.wav(start: start, seconds: maxSeconds)
            let emojiChar = emoji.trimmingCharacters(in: .whitespaces).first.map(String.init)
            try await client.uploadSound(wav, kind: kind.rawValue, name: String(name.trimmingCharacters(in: .whitespaces).prefix(Soundboard.maxNameLength)),
                                         emoji: kind == .board ? emojiChar : nil)
            client.notice = YapClient.Notice(text: kind == .chime ? "Your join chime is set." : "Added to the soundboard.")
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }
}
