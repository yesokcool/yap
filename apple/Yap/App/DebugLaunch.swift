import Foundation
import YapCore
import YapVoice
#if os(macOS)
import AppKit
import ScreenCaptureKit
#else
import UIKit
#endif

/// DEBUG-only launch switches for passive screenshots and the simulator,
/// passed as environment variables (with `simctl launch`, prefix them with
/// `SIMCTL_CHILD_`). Nothing here drives the UI with taps; it only decides
/// where the app starts. Release builds ignore all of it.
///
///   YAP_INVITE=http://127.0.0.1:8789/#key=testkey   use this server (in-memory credentials, Keychain untouched)
///   YAP_LOGIN=Name:123456                          sign in as this account
///   YAP_SIGNUP=Name:123456                         sign up as a new account (→ waiting room)
///   YAP_FAKE_KLIPY=1                               a pretend KLIPY for the GIF picker (no network)
///   YAP_SYNTHETIC_AUDIO=1                          voice uses a fake mic/speaker (no hardware, no prompts)
///   YAP_AUTOJOIN=lounge                            join this voice channel once online
///   YAP_SCREEN=channel:general | members | voice | settings | invite | gifs | emoji | staged | board | draw | space
///             (staged = general with a picture waiting in the tray, typed caption included)
///             | personal                           your Personal space
///             | voicepanel:lounge                  the call sheet for a channel without joining (Mac: no mic)
///   YAP_DEMO_INVITE=label                          make a personal invite once online (shows the one-time link)
///   YAP_DEMO_REPLY=text                            start replying to the newest message containing text
///   YAP_DEMO_EDIT=text                             start editing my newest message containing text
///   YAP_DRAFT=text                                 the open channel's composer starts with this text (\n = a new line)
///   YAP_ORIENTATION=landscape                      iPhone: turn the app to landscape once it's up (screenshots)
///   YAP_USE_PASSWORD=1                             the sign-on form starts on "Use a password"
///   YAP_DEMO_PINRESET=Name                         once online, reset Name's PIN for real (a throwaway account on a
///                                                  local server), to show the temporary PIN
///   YAP_LAST_READ=0 | id:ms,id:ms                  read marks to start from (the in-memory store has none, so a
///                                                  scripted launch starts caught up): 0 = nothing read anywhere
///   YAP_ASK=ping | archive                         ask the open channel's question (the ping note, Archive…?)
///   YAP_SCREEN=members                             the People sheet
///   YAP_VAULT_PASSCODE=…                           unlock the encrypted Personal space once online
///   YAP_SEARCH=query                               search from the open channel (Win98: the Find window)
///   YAP_SHOW_PINS=1                                show the open channel's pinned messages
///   YAP_JUMP=text                                  jump to (and flash) the newest message containing text
///   YAP_SCREEN=switcher | catchup | hang | doodle  the quick switcher (YAP_SWITCHER=text types into it),
///                                                  Catch up, Plan a hang, a doodle for the newest message in general
///   YAP_SETTINGS_PAGE=look | profile | voice | voice-advanced | notifications | account | mods | invite | about
///                                                  the Settings page (with YAP_SCREEN=settings; iPhone: on top of the list;
///                                                  voice-advanced: Voice › Advanced, its page on iPhone, unfolded on the Mac)
///   YAP_SHOW_SHELF=1 / YAP_NEW_POLL=1              the open channel's media shelf / New poll
///   YAP_UNREAD_LAST=N                              pretend the last N messages of the open channel are new
///                                                  ("New since" line, and the pill when it's off screen)
///   YAP_SNAPSHOT_HEIGHT=points                     (Mac, with YAP_SNAPSHOT) make the Settings window this tall first
///   YAP_APPEARANCE=charcoal | standard | system
///   YAP_WATCH=Name                                 once Name shares their screen (a local test peer), watch it
///                                                  (joins their voice channel; pair with YAP_SYNTHETIC_AUDIO=1)
///   YAP_SCREEN_FULL=1                              …and show it full screen
///   YAP_PERSON=Name                                open Name's popover under the voice channel (volume, actions)
///   YAP_DEMO_CALL_ENDED=soundTaken | soundFailed | stopped | offline | channelGone
///                                                  once in the call (YAP_AUTOJOIN + YAP_SYNTHETIC_AUDIO), end it as
///                                                  if that had happened off screen: you leave, and Yap says why
///   YAP_DEMO_BURST=N                               once online, send N numbered messages to the open channel at once
///   YAP_DEMO_SHARE=1                               once in the call, share a moving test card (the sharing UI
///                                                  on iPhone without the broadcast extension; nothing is captured)
///   YAP_DEMO_VOICE=reconnecting:Ada,weak:Ben,relay:Cleo,cantconnect:Name,resuming,silent
///                                                  pretend these call notes/warnings (screenshots; with voicepanel:)
///   YAP_DEMO_MICCHECK=recording | playing | done | failed   Settings › Voice › Hear yourself in that state
///                    | run                         …or run it once (with YAP_SYNTHETIC_AUDIO=1: the test chord, played silently)
///   YAP_SNAPSHOT=name                              Mac: after 8 s, save each window to the app's
///                                                  tmp folder as yap-<name>-<n>.png and quit
///                                                  (its own views; no screen recording needed;
///                                                  YAP_SNAPSHOT_DELAY=seconds waits longer, e.g. for web pages)
struct DebugLaunch {
    var invite: ServerEndpoint?
    var login: (name: String, pin: String)?
    var signup: (name: String, pin: String)?
    var fakeKlipy = false
    var syntheticAudio = false
    var autojoin: String?
    var screen: String?
    var appearance: String?
    var snapshot: String?
    var demoInvite: String?
    var demoReply: String?
    var demoEdit: String?
    var ask: String?
    var vaultPasscode: String?
    var search: String?
    var showPins = false
    var jump: String?
    var switcherText: String?
    var settingsPage: String?
    var showShelf = false
    var newPoll = false
    var unreadLast: Int?
    /// Read marks the in-memory store starts with (`YAP_LAST_READ`).
    var lastRead: [String: Int]?
    var demoVoice: String?
    var demoMicCheck: String?
    var draft: String?
    var demoPinReset: String?
    var orientation: String?
    var usePassword = false
    var watch: String?
    var screenFull = false
    var person: String?
    var demoCallEnded: BackgroundCall.Reason?
    var demoBurst: Int?
    var demoShare = false

    var usesMemoryStore: Bool { invite != nil }
    var isScripted: Bool { invite != nil }
    var clientLabel: String? { nil }

    static let current: DebugLaunch = {
        #if DEBUG
        let env = ProcessInfo.processInfo.environment
        var launch = DebugLaunch()
        launch.invite = env["YAP_INVITE"].flatMap(ServerEndpoint.parse(invite:))
        if let login = env["YAP_LOGIN"], let colon = login.lastIndex(of: ":") {
            launch.login = (String(login[..<colon]), String(login[login.index(after: colon)...]))
        }
        if let signup = env["YAP_SIGNUP"], let colon = signup.lastIndex(of: ":") {
            launch.signup = (String(signup[..<colon]), String(signup[signup.index(after: colon)...]))
        }
        launch.fakeKlipy = env["YAP_FAKE_KLIPY"] == "1"
        launch.syntheticAudio = env["YAP_SYNTHETIC_AUDIO"] == "1"
        launch.autojoin = env["YAP_AUTOJOIN"]
        launch.screen = env["YAP_SCREEN"]
        launch.appearance = env["YAP_APPEARANCE"]
        launch.snapshot = env["YAP_SNAPSHOT"]
        launch.demoInvite = env["YAP_DEMO_INVITE"]
        launch.demoReply = env["YAP_DEMO_REPLY"]
        launch.demoEdit = env["YAP_DEMO_EDIT"]
        launch.lastRead = env["YAP_LAST_READ"].map(Self.readMarks)
        launch.ask = env["YAP_ASK"]
        launch.vaultPasscode = env["YAP_VAULT_PASSCODE"]
        launch.search = env["YAP_SEARCH"]
        launch.showPins = env["YAP_SHOW_PINS"] == "1"
        launch.jump = env["YAP_JUMP"]
        launch.switcherText = env["YAP_SWITCHER"]
        launch.settingsPage = env["YAP_SETTINGS_PAGE"]
        launch.showShelf = env["YAP_SHOW_SHELF"] == "1"
        launch.newPoll = env["YAP_NEW_POLL"] == "1"
        launch.unreadLast = env["YAP_UNREAD_LAST"].flatMap(Int.init)
        launch.demoVoice = env["YAP_DEMO_VOICE"]
        launch.demoMicCheck = env["YAP_DEMO_MICCHECK"]
        launch.draft = env["YAP_DRAFT"]?.replacingOccurrences(of: "\\n", with: "\n")
        launch.demoPinReset = env["YAP_DEMO_PINRESET"]
        launch.orientation = env["YAP_ORIENTATION"]
        launch.usePassword = env["YAP_USE_PASSWORD"] == "1"
        launch.watch = env["YAP_WATCH"]
        launch.screenFull = env["YAP_SCREEN_FULL"] == "1"
        launch.person = env["YAP_PERSON"]
        launch.demoCallEnded = env["YAP_DEMO_CALL_ENDED"].flatMap(BackgroundCall.Reason.init(rawValue:))
        launch.demoBurst = env["YAP_DEMO_BURST"].flatMap(Int.init)
        launch.demoShare = env["YAP_DEMO_SHARE"] == "1"
        return launch
        #else
        return DebugLaunch()
        #endif
    }()

    /// The text channel to open at launch (`YAP_SCREEN=channel:<id>`).
    var channel: String? {
        if screen == "gifs" || screen == "emoji" || screen == "staged" || screen == "doodle" { return "general" }
        guard let screen, screen.hasPrefix("channel:") else { return nil }
        return String(screen.dropFirst("channel:".count))
    }

    func wants(_ screen: String) -> Bool { self.screen == screen }

    /// "0" = a device that has read before but nothing here (every channel
    /// unread, under a key no channel has); "general:0,games:1791…" = these marks.
    nonisolated static func readMarks(_ text: String) -> [String: Int] {
        if let all = Int(text) { return ["": all] }
        var marks: [String: Int] = [:]
        for pair in text.split(separator: ",") {
            let parts = pair.split(separator: ":", maxSplits: 1)
            if parts.count == 2, let ms = Int(parts[1]) { marks[String(parts[0])] = ms }
        }
        return marks
    }

    static func page(named name: String) -> SettingsPage? {
        name == "look" ? .appearance : name == "voice-advanced" ? .voice : SettingsPage(rawValue: name)
    }

    /// `YAP_SCREEN=staged`: the app icon as a picture waiting to be sent.
    @MainActor
    static func demoPicture() -> Data? {
        #if os(iOS)
        UIImage(named: "YapIcon")?.pngData()
        #else
        NSImage(named: "YapIcon")?.tiffRepresentation
        #endif
    }

    /// Settings opens at launch (`settings`, or `invite` for Settings › Invite).
    var wantsSettings: Bool { wants("settings") || wants("invite") }

    #if DEBUG
    /// `YAP_DEMO_VOICE`: notes by person name, and the call's warnings.
    @MainActor
    func applyVoiceDemo(to model: AppModel) {
        guard let demoVoice else { return }
        var demo = VoiceController.Demo()
        for item in demoVoice.split(separator: ",").map(String.init) {
            let parts = item.split(separator: ":", maxSplits: 1).map(String.init)
            switch parts[0] {
            case "resuming": demo.resuming = true
            case "silent": demo.silent = true
            default:
                guard parts.count == 2, let uid = model.client.members.first(where: { $0.name == parts[1] })?.id else { continue }
                let hint: ConnectionHint? = switch parts[0] {
                case "reconnecting": .reconnecting
                case "weak": .weak(concealed: 0.06, rttMs: 340)
                case "relay": .needsRelay
                case "cantconnect": .cantConnect
                default: nil
                }
                demo.hints[uid] = hint
            }
        }
        model.voice.demo = demo
    }

    /// `YAP_DEMO_MICCHECK`: Hear yourself in one of its states.
    @MainActor
    func applyMicCheckDemo(to model: AppModel) {
        switch demoMicCheck {
        case "recording": model.micCheck.show(.recording, level: 0.06)
        case "playing": model.micCheck.show(.playing)
        case "done": model.micCheck.show(.done)
        case "failed": model.micCheck.show(.failed("Couldn't open the microphone."))
        case "run": model.hearYourself()
        default: break
        }
    }
    #endif

    /// `YAP_SCREEN=voicepanel:<id>`: show the call sheet for this channel without joining.
    var previewVoiceChannel: String? {
        guard let screen, screen.hasPrefix("voicepanel:") else { return nil }
        return String(screen.dropFirst("voicepanel:".count))
    }

    @MainActor
    func apply(to model: AppModel) {
        #if DEBUG
        if let appearance { UserDefaults.standard.set(appearance, forKey: Appearance.storageKey) }
        guard let invite else { return }
        model.client.useInvite(invite)
        if fakeKlipy { FakeKlipy.install() }
        if let login { model.client.logIn(name: login.name, pin: login.pin) }
        if let signup { model.client.signUp(name: signup.name, pin: signup.pin, color: "#8e4ec6") }
        if let channel { model.open(.channel(channel)) }
        if wants("board") { model.open(.web(.board)) }
        if wants("draw") { model.open(.web(.draw)) }
        if wants("space") { model.open(.web(.space)) }
        // Settings' list (iPhone), or a page of it: YAP_SETTINGS_PAGE=look | profile | voice |
        // notifications | account | mods | invite | about (YAP_SCREEN=invite = the Invite page).
        let page = wants("invite") ? .invite : wantsSettings ? settingsPage.flatMap(Self.page(named:)) : nil
        if let page {
            model.settingsPage = page
            #if os(iOS)
            model.showSettings(page)
            if settingsPage == "voice-advanced" { model.path.append(.voiceAdvanced) }
            #else
            model.voiceAdvancedOpen = settingsPage == "voice-advanced"
            #endif
        } else if wantsSettings {
            model.open(.settings)
        }
        #if os(macOS)
        if let snapshot { Self.snapshotWindows(named: snapshot, openSettings: wantsSettings) }
        #else
        if orientation == "landscape" {
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(1))
                for case let scene as UIWindowScene in UIApplication.shared.connectedScenes {
                    scene.requestGeometryUpdate(.iOS(interfaceOrientations: .landscapeRight))
                }
            }
        }
        #endif
        #endif
    }

    #if os(macOS) && DEBUG
    /// Unbuffered (stdout to a file only flushes at exit).
    static func log(_ text: String) {
        FileHandle.standardError.write(Data((text + "\n").utf8))
    }

    /// ScreenCaptureKit can hang on some sheet windows: give up after `timeout`.
    @MainActor
    static func capture(_ filter: SCContentFilter, _ config: SCStreamConfiguration, timeout: Double) async -> CGImage? {
        await withCheckedContinuation { (continuation: CheckedContinuation<CGImage?, Never>) in
            var finished = false
            func finish(_ image: CGImage?) {
                guard !finished else { return }
                finished = true
                continuation.resume(returning: image)
            }
            Task { @MainActor in finish(try? await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)) }
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(timeout))
                finish(nil)
            }
        }
    }

    @MainActor
    static func snapshotWindows(named name: String, openSettings: Bool) {
        Task { @MainActor in
            let delay = Double(ProcessInfo.processInfo.environment["YAP_SNAPSHOT_DELAY"] ?? "") ?? 8
            try? await Task.sleep(for: .seconds(delay))
            // YAP_SNAPSHOT_HEIGHT=<points>: the Settings window that tall first, to show a whole page.
            if let height = Double(ProcessInfo.processInfo.environment["YAP_SNAPSHOT_HEIGHT"] ?? "") {
                for window in NSApp.windows where window.isVisible && window.identifier?.rawValue.contains("Settings") == true {
                    window.setContentSize(NSSize(width: window.contentLayoutRect.width, height: height))
                }
                try? await Task.sleep(for: .seconds(1.5))
            }
            // The app's own windows via ScreenCaptureKit's current-process
            // content (no screen-recording permission), which includes
            // materials and glass; AppKit's cacheDisplay as the fallback.
            log("snapshot: looking for windows")
            let own = try? await SCShareableContent.currentProcess
            log("snapshot: \(own?.windows.count ?? -1) capturable windows")
            for (index, window) in NSApp.windows.filter(\.isVisible).enumerated() {
                log("snapshot: window \(index) \(window.title) \(window.frame)")
                let url = URL(fileURLWithPath: NSTemporaryDirectory()).appending(path: "yap-\(name)-\(index).png")
                if let scWindow = own?.windows.first(where: { $0.windowID == CGWindowID(window.windowNumber) }) {
                    let filter = SCContentFilter(desktopIndependentWindow: scWindow)
                    let config = SCStreamConfiguration()
                    config.width = Int(filter.contentRect.width * CGFloat(filter.pointPixelScale))
                    config.height = Int(filter.contentRect.height * CGFloat(filter.pointPixelScale))
                    config.showsCursor = false
                    config.ignoreShadowsSingleWindow = true
                    log("snapshot: capturing \(index) via ScreenCaptureKit")
                    let captured = await capture(filter, config, timeout: 4)
                    log("snapshot: captured \(index): \(captured != nil)")
                    if let image = captured,
                       let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) {
                        try? png.write(to: url)
                        print("snapshot: \(url.path) (\(window.title))")
                        continue
                    }
                }
                log("snapshot: \(index) falls back to cacheDisplay")
                guard let view = window.contentView?.superview ?? window.contentView,
                      let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
                view.cacheDisplay(in: view.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?.write(to: url)
                print("snapshot: \(url.path) (\(window.title)) [cacheDisplay]")
            }
            NSApp.terminate(nil)
        }
    }
    #endif
}
