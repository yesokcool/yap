import Foundation
import Observation
import SwiftUI
import YapCore
import YapVoice

/// Where the main pane can go: a text channel, an in-app web page, or the
/// screen share I'm watching.
enum Destination: Hashable {
    case channel(String)
    case web(WebPage)
    /// Someone's screen, big (`voice.watch` says whose).
    case screen
    /// iPhone: Settings is a row at the end of the channel list (its list of pages)…
    case settings
    /// …and one of its pages…
    case settingsPage(SettingsPage)
    /// …and Voice › Advanced (a disclosure on the Mac).
    case voiceAdvanced
}

/// In-app web pages: the server's board (optional; it's served at `/board`),
/// and the web app for the places that are web-only (Draw, Space). Voice
/// keeps going meanwhile.
enum WebPage: Hashable {
    case board
    case draw
    case space

    var title: String {
        switch self {
        case .board: "Board"
        case .draw: "Draw"
        case .space: "Space"
        }
    }

    var systemImage: String {
        switch self {
        case .board: "square.grid.3x2"
        case .draw: "scribble.variable"
        case .space: "map"
        }
    }

    /// The web app's name for it (`#place=`, and the `view` people see), nil for the board.
    var place: String? {
        switch self {
        case .board: nil
        case .draw: "draw"
        case .space: "space"
        }
    }

    /// The plain address (Share, and the fallback when no sign-in pass comes).
    func url(for endpoint: ServerEndpoint) -> URL {
        if let place { endpoint.webURL(place: place) ?? endpoint.baseURL } else { endpoint.boardURL }
    }
}

extension AppModel {
    /// The sidebar footer's link: "v0.39.0 · what's new" (the server's
    /// version, like the web), "What's new" until it's known.
    var whatsNewTitle: String {
        serverVersion.map { "v\($0) · what's new" } ?? "What's new"
    }

    /// A page's title. The board goes by the name of the server's board link.
    func title(of page: WebPage) -> String {
        if page == .board, let name = places.first(where: { $0.view == "board" })?.name { return name }
        return page.title
    }

    /// "Open in Safari": web-app places open signed in (a one-time pass).
    func openInBrowser(_ page: WebPage, with openURL: OpenURLAction) {
        if let place = page.place {
            client.openWeb(place, with: openURL)
        } else if let endpoint = client.endpoint {
            openURL(page.url(for: endpoint))
        }
    }
}

/// Settings' pages: the Mac's tabs and iPhone's list, in the web's order
/// (plus About).
enum SettingsPage: String, Hashable, CaseIterable, Identifiable {
    case appearance, profile, voice, notifications, account, mods, invite, about

    var id: Self { self }

    /// The web's tab names.
    var title: String {
        switch self {
        case .appearance: "Look"
        case .profile: "Profile"
        case .voice: "Voice"
        case .notifications: "Notifications"
        case .account: "Account"
        case .mods: "Owner & mods"
        case .invite: "Invite"
        case .about: "About"
        }
    }

    var systemImage: String {
        switch self {
        case .appearance: "paintpalette"
        case .profile: "person.crop.circle"
        case .voice: "waveform"
        case .notifications: "bell"
        case .account: "key"
        case .mods: "shield.lefthalf.filled"
        case .invite: "person.badge.plus"
        case .about: "info.circle"
        }
    }
}

/// The app's one object graph: the protocol client, voice, and the bits of
/// server config the screens need.
@MainActor
@Observable
final class AppModel {
    let client: YapClient
    let voice: VoiceController
    let images = ImageStore()
    let notifier = Notifier()
    /// Chimes, blips and the soundboard.
    let sounds = SoundPlayer()
    /// Settings › Voice › Hear yourself.
    let micCheck: MicCheck
    private(set) var config: ServerConfig?
    /// The server's Yap version (the sidebar footer's "v0.39.0 · what's new").
    private(set) var serverVersion: String?
    private(set) var minecraft: MinecraftStatus?

    /// iOS: the chat tab's navigation path. Mac: the sidebar selection.
    var path: [Destination] = []
    var selection: Destination?
    var showingVoicePanel = false
    /// Mac: which property-sheet tab Settings opens on.
    var settingsPage: SettingsPage = .profile
    /// Mac: `showSettings` asked for the Settings window (the root view opens it).
    var settingsRequested = false
    /// Mac: Settings › Voice › Advanced is unfolded (folded again each time Settings opens).
    var voiceAdvancedOpen = false
    /// Bumped when Settings closes, so the next opening starts fresh (short lists).
    var settingsSession = 0
    /// The People sheet (everyone A–Z, the waiting room). There's no member
    /// sidebar and no online/offline (PROTOCOL.md › Presence).
    var showingPeople = false
    /// The encrypted Personal space's passcode sheet.
    var vaultSheet: VaultMode?
    /// Change PIN (from the "easy to guess" nudge).
    var changingPin = false
    /// The "easy to guess" nudge was dismissed for this run.
    var weakPinDismissed = false
    /// A message to scroll to once its channel is on screen.
    var jump: JumpTarget?
    /// The screen I'm watching fills the window (Mac: the window goes full
    /// screen and the channels pane steps aside; iPhone: no bars).
    var screenFullScreen = false
    /// Picture-in-picture for the screen I'm watching ("Pop out").
    let screenPiP = ScreenPictureInPicture()
    /// Mac: where the main pane was before the screen opened (it goes back there after).
    var beforeScreen: Destination?
    /// Search asked for from elsewhere (the quick switcher's last row): the
    /// open channel's search picks it up.
    var searchRequest: SearchRequest?
    /// The quick switcher (⌘K).
    var showingSwitcher = false
    /// Catch up: the unread channels, with counts.
    var showingCatchUp = false
    /// Plan a hang (from Places or a hang notice).
    var planningHang = false
    /// The message a doodle reaction is being drawn for (one sheet at the root).
    var doodleTarget: Message?

    /// The channel the call sheet shows: the call we're in (or, for DEBUG
    /// screenshots, `YAP_SCREEN=voicepanel:<id>`).
    var callSheetChannel: String {
        voice.wantedChannelId ?? DebugLaunch.current.previewVoiceChannel ?? ""
    }

    /// iPhone: a call ending while Yap is off screen says so ("You left Lounge").
    @ObservationIgnored private(set) var callWatch: CallWatch?
    @ObservationIgnored private var minecraftTask: Task<Void, Never>?
    @ObservationIgnored private var configTask: Task<Void, Never>?

    init() {
        let launch = DebugLaunch.current
        let credentials: any CredentialStore = launch.usesMemoryStore ? MemoryCredentialStore() : DeviceCredentialStore(service: AppIdentity.keychainService)
        if let marks = launch.lastRead, let memory = credentials as? MemoryCredentialStore { memory.lastRead = marks }
        client = YapClient(credentials: credentials, client: launch.clientLabel ?? AppIdentity.clientLabel)
        #if os(iOS)
        if launch.syntheticAudio {
            voice = VoiceController(client: client, engine: VoiceEngine(audioDevice: SyntheticAudioDevice()), requestsMicPermission: false)
        } else {
            voice = VoiceController(client: client)
        }
        #else
        voice = VoiceController(client: client)
        #endif
        micCheck = MicCheck(source: launch.syntheticAudio ? .synthetic : .microphone)
        client.onMention = { [weak self] message in self?.mentioned(message) }
        client.emojiTone = EmojiToneSetting.current
        NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak client] _ in
            MainActor.assumeIsolated { client?.emojiTone = EmojiToneSetting.current }
        }
        client.onPing = { [weak self] user, text in self?.pinged(user, text) }
        client.onSound = { [weak self] sound, _, uid in self?.soundPlayed(sound, by: uid) }
        client.onHangNow = { [weak self] hang in self?.hangStarted(hang) }
        client.onAdmitted = { [weak self] in self?.admitted() }
        client.savedToPersonalBefore = {
            defer { UserDefaults.standard.set(true, forKey: "yap.savedToPersonal") }
            return UserDefaults.standard.bool(forKey: "yap.savedToPersonal")
        }
        voice.onEvent = { [weak self] event in self?.voiceEvent(event) }
        voice.onAudioSession = { [weak self] event in self?.callWatch?.audio(event) }
        voice.onEndedByServer = { [weak self] in self?.callWatch?.endedByServer() }
        voice.onInVoiceChanged = { [weak self] in self?.callWatch?.callChanged() }
        notifier.onRejoin = { [weak self] channelId in self?.rejoin(channelId) }
        screenPiP.renderer = voice.screenRenderer
        screenPiP.onRestore = { [weak self] in self?.showScreen() }
        screenPiP.onClosed = { [weak self] in
            // Closed the popped-out window while the viewer isn't on screen: done watching.
            guard let self, !self.isShowingScreen else { return }
            self.voice.stopWatching()
        }
        #if os(iOS)
        let terminating = UIApplication.willTerminateNotification
        #else
        let terminating = NSApplication.willTerminateNotification
        #endif
        NotificationCenter.default.addObserver(forName: terminating, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.willTerminate() }
        }
        callWatch = CallWatch(model: self)
        #if os(iOS)
        // Screen sharing: the broadcast extension finds Yap at this socket while it's in a call.
        voice.broadcastSocketPath = AppIdentity.appGroup.flatMap(BroadcastScreenFeeder.socketPath(appGroup:))
        #endif
        launch.apply(to: self)
    }

    func start() {
        client.start()
        refreshConfig()
    }

    // MARK: Config, Minecraft

    func refreshConfig() {
        guard let endpoint = client.endpoint else { return }
        configTask?.cancel()
        configTask = Task {
            if let config = try? await ServerAPI(endpoint: endpoint).config() {
                self.config = config
                if config.minecraft == nil { stopMinecraft() } else { startMinecraft() }
            }
            if let version = try? await ServerAPI(endpoint: endpoint).serverVersion() { serverVersion = version }
        }
    }

    /// Poll the server's Minecraft status about once a minute while the app is
    /// up. Optional: only servers with one set up (`MINECRAFT_SERVER`) send
    /// `minecraft` in their config.
    private func startMinecraft() {
        guard config?.minecraft != nil, minecraftTask == nil else { return }
        minecraftTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshMinecraft()
                try? await Task.sleep(for: .seconds(60))
            }
        }
    }

    /// No Minecraft on this server: no polling, no row, no status-bar panel.
    private func stopMinecraft() {
        minecraftTask?.cancel()
        minecraftTask = nil
        minecraft = nil
    }

    func refreshMinecraft() async {
        guard config?.minecraft != nil, let endpoint = client.endpoint, client.isAppActive else { return }
        if let status = try? await ServerAPI(endpoint: endpoint).minecraft() {
            minecraft = status
        }
    }

    /// Sidebar Places: the server's live list (built-in views, https links).
    var places: [PlaceLink] {
        client.links.filter { $0.view != nil || $0.url.lowercased().hasPrefix("https://") }
    }

    var klipy: KlipyClient {
        KlipyClient(appKey: config?.gifs?.appKey, uid: client.uid, session: KlipyClient.mediaSession)
    }

    // MARK: Lifecycle

    /// Settings › Voice › Hear yourself: 4 s through what a call does, played back.
    func hearYourself() {
        guard VoiceOptions.canHear(self) else { return }
        let settings = voice.settings
        Task {
            await micCheck.run(bitrate: voice.callBitrate, mode: settings.mode, threshold: settings.vadThreshold,
                               highQualityBluetooth: settings.highQualityBluetooth)
        }
    }

    /// Quitting: a goodbye to the call, so nobody keeps it waiting for us.
    func willTerminate() {
        callWatch?.willTerminate()
        voice.goodbye()
    }

    /// "You left Lounge… Tap to rejoin.": back into that call (once we're connected).
    func rejoin(_ channelId: String) {
        Task {
            for _ in 0..<150 where !client.isConnected { try? await Task.sleep(for: .milliseconds(200)) }
            guard client.channelsById[channelId]?.kind == .voice else { return }
            await voice.join(channelId)
        }
    }

    func scenePhaseChanged(_ phase: ScenePhase) {
        client.isAppActive = phase == .active
        callWatch?.scene(active: phase == .active)
        if phase == .active {
            client.reconnectNow()
            if config == nil { refreshConfig() } else { Task { await refreshMinecraft() } }
        }
    }

    @ObservationIgnored private var autojoined = false
    @ObservationIgnored private var demoInvited = false
    @ObservationIgnored private var demoPinReset = false
    @ObservationIgnored private var demoWatched = false
    @ObservationIgnored private var demoCallEnded = false
    @ObservationIgnored private var demoShared = false

    func phaseChanged(_ phase: YapClient.Phase) {
        callWatch?.connection(online: phase == .online)
        if phase == .online {
            if config == nil { refreshConfig() }
            warmSounds()
            notifier.requestAuthorizationOnce()
            #if DEBUG
            if let channel = DebugLaunch.current.autojoin, !autojoined {
                autojoined = true
                Task { await voice.join(channel) }
            }
            if let label = DebugLaunch.current.demoInvite, !demoInvited {
                demoInvited = true
                client.createInvite(label: label)
            }
            if let name = DebugLaunch.current.demoPinReset, !demoPinReset {
                demoPinReset = true
                Task {
                    try? await Task.sleep(for: .seconds(1.5))
                    if let uid = client.members.first(where: { $0.name == name })?.id { client.resetPin(for: uid) }
                }
            }
            if DebugLaunch.current.previewVoiceChannel != nil { showingVoicePanel = true }
            if let name = DebugLaunch.current.watch, !demoWatched {
                demoWatched = true
                Task {
                    // Wait (up to a minute) for their share to show up, then watch it.
                    for _ in 0..<300 {
                        if let uid = client.members.first(where: { $0.name == name })?.id,
                           let session = client.sessions(of: uid).first(where: { $0.voice?.screen == true }) {
                            watchScreen(session)
                            if DebugLaunch.current.screenFull {
                                try? await Task.sleep(for: .seconds(3))
                                setScreenFullScreen(true)
                            }
                            return
                        }
                        try? await Task.sleep(for: .milliseconds(200))
                    }
                }
            }
            DebugLaunch.current.applyVoiceDemo(to: self)
            DebugLaunch.current.applyMicCheckDemo(to: self)
            if DebugLaunch.current.wants("personal"), let personal = client.personalChannel { open(.channel(personal.id)) }
            if DebugLaunch.current.wants("members") { showingPeople = true }
            if DebugLaunch.current.wants("switcher") { showingSwitcher = true }
            if DebugLaunch.current.wants("hang") { planningHang = true }
            if DebugLaunch.current.wants("catchup") {
                showingCatchUp = true
            }
            if DebugLaunch.current.wants("doodle") {
                Task {
                    for _ in 0..<50 where client.histories["general"]?.loading != false { try? await Task.sleep(for: .milliseconds(100)) }
                    doodleTarget = client.histories["general"]?.messages.last
                }
            }
            if DebugLaunch.current.demoShare, !demoShared {
                demoShared = true
                Task {
                    for _ in 0..<100 where voice.channelId == nil { try? await Task.sleep(for: .milliseconds(100)) }
                    try? await voice.startSharing(SyntheticScreenFeeder())
                }
            }
            if let reason = DebugLaunch.current.demoCallEnded, !demoCallEnded {
                demoCallEnded = true
                Task {
                    for _ in 0..<100 where voice.channelId == nil { try? await Task.sleep(for: .milliseconds(100)) }
                    try? await Task.sleep(for: .seconds(1))
                    callWatch?.demoEnd(reason)
                }
            }
            if let passcode = DebugLaunch.current.vaultPasscode, client.vaultLocked {
                Task { try? await client.unlockVault(passcode: passcode) }
            }
            #endif
        }
    }

    // MARK: Mentions + pings

    private func mentioned(_ message: Message) {
        let author = client.user(message.uid)?.name ?? "Someone"
        let channel = client.channelsById[message.channelId]?.name ?? "Yap"
        let lookingAtIt = client.isAppActive && client.activeChannelId == message.channelId
        // Quiet hours apply to our own notifications and their sounds too.
        guard !lookingAtIt, !client.quietHolds(mention: true) else { return }
        notifier.chime()
        if !client.isAppActive {
            notifier.post(title: "\(author) mentioned you", subtitle: channel, body: MessageFormatter.preview(message), thread: message.channelId)
        }
    }

    /// Just let in from the waiting room: a chime (the first channel says
    /// "You're in. Say hi 👋"), and a notification if Yap is in the background.
    private func admitted() {
        notifier.chime()
        if !client.isAppActive {
            notifier.post(title: "You're in!", subtitle: nil, body: "Say hi 👋", thread: "yap-welcome")
        }
    }

    private func pinged(_ user: User?, _ text: String) {
        guard !client.quietHolds(mention: false) else { return }
        notifier.chime()
        if !client.isAppActive {
            notifier.post(title: "📣 \(user?.name ?? "Someone") pinged everyone", subtitle: nil, body: text.isEmpty ? "Hop on Yap!" : text, thread: "yap-ping")
        }
    }

    /// A hang is starting: the notice (the app shows `client.hangStarting`
    /// with Join voice); a notification if Yap is in the background.
    private func hangStarted(_ hang: Hang) {
        guard !client.isAppActive, !client.quietHolds(mention: false) else { return }
        let place = hang.channelId.flatMap { client.channelsById[$0]?.name }
        notifier.post(title: "🗓 \(hang.title) is starting", subtitle: place, body: place.map { "Hop in \($0)." } ?? "Hop on Yap.", thread: "yap-hang")
    }

    // MARK: Navigation

    func open(_ destination: Destination) {
        #if os(macOS)
        selection = destination
        #else
        path = [destination]
        #endif
    }
}
