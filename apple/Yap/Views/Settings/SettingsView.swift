import PhotosUI
import SwiftUI
import YapCore
import YapVoice

/// Settings. Mac: a Windows 98 property sheet (tabs). iPhone: a Control
/// Panel-style list of the same pages; each opens on its own screen.
struct SettingsView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        #if os(macOS)
        VStack(alignment: .leading, spacing: 0) {
            Tabs98(tabs: SettingsPage.allCases.map { ($0, $0.title) }, selection: $model.settingsPage)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    SettingsPageContent98(page: model.settingsPage)
                }
                .padding(12)
            }
            .bevel(.raised)
        }
        .padding(8)
        .background(Theme.face)
        #else
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(SettingsPage.allCases) { page in
                    NavigationLink(value: Destination.settingsPage(page)) {
                        HStack(spacing: 10) {
                            Image(systemName: page.systemImage)
                                .font(Theme.chat)
                                .foregroundStyle(Theme.link)
                                .frame(width: 24)
                            Text(page.title)
                                .font(Theme.chat)
                                .foregroundStyle(Theme.text)
                            Spacer(minLength: 4)
                            Image(systemName: "chevron.right")
                                .font(Theme.small)
                                .foregroundStyle(Theme.hint)
                        }
                        .padding(.horizontal, 10)
                        .frame(minHeight: 44)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            }
            .well()
            .padding(10)
        }
        .background(Theme.face)
        .win98NavigationBar("Yap Properties")
        #endif
    }
}

/// One Settings page's group boxes: a Mac tab, an iPhone screen.
struct SettingsPageContent98: View {
    var page: SettingsPage

    var body: some View {
        switch page {
        case .appearance: AppearanceGroup()
        case .profile:
            ProfileGroup()
            AwayGroup98()
        case .voice:
            MicOutputGroup98()
            VoiceGroup().id("talk")
            SoundsGroup98()
            SoundboardGroup98()
            CallReportGroup98()
            VoiceAdvanced98()
        case .notifications: NotificationsGroup98()
        case .account:
            AccountGroup()
            PersonalSpaceGroup()
        case .mods: ModsGroup()
        case .invite: InviteGroup()
        case .about: AboutGroup()
        }
    }
}

#if os(iOS)
/// iPhone: one Settings page on its own screen.
struct SettingsPageView98: View {
    var page: SettingsPage

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    SettingsPageContent98(page: page)
                    Color.clear.frame(height: 0).id("end")
                }
                .padding(10)
            }
            .task {
                // Screenshots of a page's lower groups (YAP_SETTINGS_SCROLL=bottom, or
                // talk: Voice › How you talk at the top).
                guard let scroll = DebugLaunch.current.settingsScroll else { return }
                try? await Task.sleep(for: .milliseconds(600))
                if scroll == "bottom" { proxy.scrollTo("end", anchor: .bottom) } else { proxy.scrollTo(scroll, anchor: .top) }
            }
        }
        .background(Theme.face)
        .win98NavigationBar(page.title)
    }
}
#endif

/// Picture, name, color.
struct ProfileGroup: View {
    @Environment(AppModel.self) private var model
    @State private var name = ""
    @FocusState private var editingName: Bool
    @State private var photo: PhotosPickerItem?
    @State private var uploading = false

    var body: some View {
        let client = model.client
        GroupBox98(title: "Profile") {
            HStack(spacing: 10) {
                Avatar(user: client.me, size: 44)
                VStack(alignment: .leading, spacing: 4) {
                    let pickerTitle = uploading ? "Uploading…" : "Upload…"
                    PhotosPicker(selection: $photo, matching: .images) {
                        Text(pickerTitle)
                    }
                    .buttonStyle(.win98)
                    .disabled(uploading)
                    if client.me?.avatar != nil {
                        Button("Remove") { client.removeAvatar() }
                            .buttonStyle(.win98)
                    }
                }
            }
            Text("Name:").font(Theme.chrome).foregroundStyle(Theme.text)
            HStack(spacing: 6) {
                Field98 {
                    TextField("", text: $name)
                        .focused($editingName)
                        .onSubmit(saveName)
                        .submitLabel(.done)
                }
                Button("Apply", action: saveName)
                    .buttonStyle(.win98)
                    .disabled(name.isBlank || name == client.me?.name)
            }
            Text("Color:").font(Theme.chrome).foregroundStyle(Theme.text)
            ColorSwatches(selection: Binding(
                get: { client.me?.color ?? client.savedColor ?? Theme.personColors[0] },
                set: { client.updateProfile(color: $0) }
            ))
        }
        .onAppear { name = client.me?.name ?? client.savedName ?? "" }
        .onChange(of: client.me?.name) { _, new in if let new { name = new } }
        // Also saved when the field loses focus or the page closes (not only Apply / Return).
        .onChange(of: editingName) { _, editing in if !editing { saveName() } }
        .onDisappear(perform: saveName)
        .onChange(of: photo) { _, item in
            guard let item else { return }
            Task { await upload(item) }
        }
    }

    private func saveName() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != model.client.me?.name else { return }
        model.client.updateProfile(name: trimmed)
    }

    /// Square-crop to 256 px JPEG (well under 512 KB), then upload.
    private func upload(_ item: PhotosPickerItem) async {
        uploading = true
        defer {
            uploading = false
            photo = nil
        }
        guard let data = try? await item.loadTransferable(type: Data.self), let jpeg = AvatarImage.squareJPEG(from: data) else {
            model.client.notice = YapClient.Notice(text: "Couldn't read that picture.")
            return
        }
        do {
            try await model.client.uploadAvatar(jpeg, mime: "image/jpeg")
        } catch {
            model.client.notice = YapClient.Notice(text: error.localizedDescription)
        }
    }
}

/// Password and sign out.
struct AccountGroup: View {
    @Environment(AppModel.self) private var model
    @State private var changingPassword = false
    @State private var confirmSignOut = false

    var body: some View {
        GroupBox98(title: "Account") {
            Text(AccountText.howToSignIn)
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            // Wraps at big text sizes instead of cutting the labels short.
            FlowLayout(spacing: 6) {
                Button("\(PasswordText.change(onPin: model.client.onPin))…") { changingPassword = true }
                    .buttonStyle(.win98)
                Button("Sign out of this device…") { confirmSignOut = true }
                    .buttonStyle(.win98)
            }
            DevicesGroup98()
        }
        .sheet(isPresented: $changingPassword) { ChangePasswordView() }
        .question(isPresented: $confirmSignOut, Question(title: AccountText.signOutTitle, text: AccountText.signOutMessage,
                                                         action: "Sign out", danger: true, perform: { [client = model.client] _ in client.signOut() }))
    }
}

/// Voice › Mic & output: Hear yourself (and on iPhone, high-quality
/// headphone audio). No device pickers: the call's output button and the
/// Mac's system devices choose.
struct MicOutputGroup98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var voice = model.voice
        GroupBox98(title: SettingsText.micOutput) {
            MicCheck98()
            MicVolumeRow98()
            #if os(iOS)
            Check98(title: VoiceOptions.headphonesTitle, isOn: Binding(get: { voice.settings.highQualityBluetooth }, set: {
                voice.settings.highQualityBluetooth = $0
                VoiceOptions.changed(model)
            }))
            Text(VoiceOptions.headphonesNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            #endif
            if MicModes.offered {
                MicModeRow98()
            }
            Check98(title: TalkWords.dip, isOn: $voice.settings.dipSharedMedia)
            Text(TalkWords.dipNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Voice › Mic & output › Mic mode (iPhone, iPad): what iOS reports, and
/// Change… opening Apple's own picker. Mic modes only apply while a call
/// runs Apple's voice processing, so Change… waits for a call; outside one
/// the row says what you chose last.
struct MicModeRow98: View {
    @Environment(AppModel.self) private var model
    @State private var mode = MicModes.preferred

    var body: some View {
        let inCall = model.voice.isInVoice
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(MicModeWords.row(mode))
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                Spacer(minLength: 4)
                Button(MicModeWords.change) { MicModes.showPicker() }
                    .buttonStyle(.win98)
                    .disabled(!inCall)
            }
            Text(MicModeWords.note)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
        // The picker is Apple's: look again every second while this shows.
        .task(id: inCall) {
            while !Task.isCancelled {
                let now = inCall ? MicModes.reported : MicModes.preferred
                if now != mode { mode = now }
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }
}

/// Voice › Call report: copy what Yap noticed about the last call (or the
/// one you're in) as text.
struct CallReportGroup98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let has = model.voice.notes.hasReport
        GroupBox98(title: "Call report") {
            HStack(spacing: 8) {
                Button(CallReportWords.copyLast) { model.copyCallReport() }
                    .buttonStyle(.win98)
                    .disabled(!has)
                if !has {
                    Text(CallReportWords.none)
                        .font(Theme.small)
                        .foregroundStyle(Theme.hint)
                }
            }
            Text(CallReportWords.note)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Voice › How you talk: input mode and sensitivity.
struct VoiceGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var voice = model.voice
        GroupBox98(title: SettingsText.howYouTalk) {
            Text("Input mode:").font(Theme.chrome).foregroundStyle(Theme.text)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(VoiceEngine.InputMode.allCases) { mode in
                    Radio98(title: mode.title, isOn: voice.settings.mode == mode) { voice.settings.mode = mode }
                }
            }
            if voice.settings.mode == .voiceActivity {
                HStack {
                    Text("\(Text("\(SettingsText.sensitivity):").foregroundStyle(Theme.text)) \(Text("— speak; the bar should cross the line").foregroundStyle(Theme.hint).font(Theme.small))")
                        .font(Theme.chrome)
                    Spacer()
                    Text("\(Int(voice.settings.vadThreshold)) dB").font(Theme.chrome.monospacedDigit()).foregroundStyle(Theme.text)
                }
                Slider(value: $voice.settings.vadThreshold, in: VoiceSettings.vadRange, step: 1)
                if let level = VoiceOptions.meterLevel(model) {
                    LevelMeter(level: level, threshold: voice.settings.vadThreshold, tooLoud: VoiceOptions.meterTooLoud(model))
                } else {
                    Text(VoiceOptions.meterOffNote)
                        .font(Theme.small)
                        .foregroundStyle(Theme.hint)
                }
            }
            Text(SettingsText.pushToTalkNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            Check98(title: TalkWords.pushToTalkClick, isOn: $voice.settings.pushToTalkClick)
            Text(TalkWords.pushToTalkClickNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            Check98(title: TalkWords.reminder, isOn: $voice.settings.mutedReminder)
            Text(TalkWords.reminderNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Voice › Advanced. iPhone: a row that opens its own page. Mac: a
/// tree-view [+] that unfolds it here, folded again each time Settings opens.
struct VoiceAdvanced98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        #if os(iOS)
        NavigationLink(value: Destination.voiceAdvanced) {
            HStack(spacing: 10) {
                Text(SettingsText.advanced)
                    .font(Theme.chat)
                    .foregroundStyle(Theme.text)
                Spacer(minLength: 4)
                Image(systemName: "chevron.right")
                    .font(Theme.small)
                    .foregroundStyle(Theme.hint)
            }
            .padding(.horizontal, 10)
            .frame(minHeight: 44)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .well()
        #else
        let open = model.voiceAdvancedOpen
        VStack(alignment: .leading, spacing: 8) {
            Button { model.voiceAdvancedOpen.toggle() } label: {
                HStack(spacing: 6) {
                    // A tree view's expander: a little box with + or −.
                    Text(open ? "−" : "+")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundStyle(Theme.text)
                        .frame(width: 11, height: 11)
                        .background(Theme.window)
                        .overlay(Rectangle().stroke(Theme.grayText, lineWidth: 1))
                    Text(SettingsText.advanced).font(Theme.chrome).foregroundStyle(Theme.text)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(SettingsText.advanced)
            .accessibilityValue(open ? "Expanded" : "Collapsed")
            if open {
                VoiceAdvancedGroup98()
            }
        }
        #endif
    }
}

/// Voice › Advanced's controls: quality, Hide my IP address.
struct VoiceAdvancedGroup98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var voice = model.voice
        GroupBox98(title: SettingsText.advanced) {
            HStack {
                Text("\(SettingsText.quality):").font(Theme.chrome).foregroundStyle(Theme.text)
                Spacer()
                Dropdown98(title: VoiceGroup.label(voice.settings.bitrate), accessibility: SettingsText.quality) {
                    Picker(SettingsText.quality, selection: $voice.settings.bitrate) {
                        ForEach(VoiceSettings.bitrateChoices, id: \.self) { bitrate in
                            Text(VoiceGroup.label(bitrate)).tag(bitrate)
                        }
                    }
                }
            }
            if let warning = VoiceSettings.qualityWarning(voice.settings.bitrate) {
                WarningLine98(text: warning)
            }
            Text(SettingsText.qualityNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            Check98(title: VoiceOptions.relayTitle, isOn: Binding(get: { voice.settings.relayOnly }, set: {
                voice.settings.relayOnly = $0
                VoiceOptions.changed(model)
            }))
            .disabled(voice.relayAvailable == false)
            Text(VoiceOptions.relayNote(available: voice.relayAvailable))
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
        .task { await model.voice.checkRelay() }
    }
}

#if os(iOS)
/// iPhone: Voice › Advanced on its own screen.
struct VoiceAdvancedPage98: View {
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                VoiceAdvancedGroup98()
            }
            .padding(10)
        }
        .background(Theme.face)
        .win98NavigationBar(SettingsText.advanced)
    }
}
#endif

/// A Win98 drop-down: a white well with the value and a little arrow
/// button; the choices are the system menu's.
struct Dropdown98<Items: View>: View {
    var title: String
    var accessibility: String
    @ViewBuilder var items: Items

    var body: some View {
        Menu {
            items
        } label: {
            HStack(spacing: 6) {
                Text(title)
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                    .lineLimit(1)
                    .padding(.leading, 6)
                Image(systemName: "arrowtriangle.down.fill")
                    .font(.system(size: 7))
                    .foregroundStyle(Theme.text)
                    .frame(width: 18, height: 18)
                    .bevel(.raised)
            }
            .padding(2)
            .bevel(.sunken, fill: Theme.window)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .fixedSize()
        #if os(iOS)
        // Lines up with the buttons beside it, which grow to a 44 pt target.
        .frame(minHeight: 44)
        #endif
        .accessibilityLabel(accessibility)
        .accessibilityValue(title)
    }
}

/// Mic check: Hear yourself and what it's doing.
struct MicCheck98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(VoiceOptions.hearHeading):").font(Theme.chrome).foregroundStyle(Theme.text)
                .accessibilityAddTraits(.isHeader)
            HStack(spacing: 8) {
                Button(VoiceOptions.hearTitle) { model.hearYourself() }
                    .buttonStyle(.win98)
                    .disabled(!VoiceOptions.canHear(model))
                Text(VoiceOptions.hearStatus(model))
                    .font(Theme.small)
                    .foregroundStyle(Theme.hint)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            if model.voice.settings.mode != .voiceActivity, model.micCheck.phase == .recording {
                LevelMeter(level: model.micCheck.level, threshold: model.voice.settings.vadThreshold, tooLoud: model.micCheck.tooLoud)
            }
            Text(VoiceOptions.hearNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

extension VoiceGroup {
    static func label(_ bitrate: Int) -> String {
        bitrate == 128_000 ? "128 kbps (default)" : "\(bitrate / 1000) kbps"
    }
}

/// Charcoal / Windows Standard / system (one look, light or dark).
struct AppearanceGroup: View {
    @AppStorage(Appearance.storageKey) private var appearance: Appearance = .charcoal

    var body: some View {
        GroupBox98(title: "Look") {
            Text("Theme:").font(Theme.chrome).foregroundStyle(Theme.text)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Appearance.allCases) { option in
                    Radio98(title: option.title, isOn: appearance == option) { appearance = option }
                }
            }
        }
    }
}

/// Server, privacy, version.
struct AboutGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        GroupBox98(title: "About Yap") {
            if let endpoint = model.client.endpoint {
                row("Server", endpoint.baseURL.host() ?? endpoint.baseURL.absoluteString)
            }
            if let endpoint = model.client.endpoint {
                Link(destination: endpoint.privacyURL) { Text("Privacy…") }
                    .buttonStyle(.win98)
            }
            if let version = model.serverVersion {
                row("Server version", version)
            }
            row("App version", AppIdentity.version)
            if let flavor = AppIdentity.buildFlavor {
                row("Build", flavor)
            }
        }
    }

    private func row(_ title: String, _ value: String) -> some View {
        HStack {
            Text("\(title):").foregroundStyle(Theme.text)
            Spacer()
            Text(value).foregroundStyle(Theme.hint)
        }
        .font(Theme.chrome)
    }
}

enum AvatarImage {
    static func squareJPEG(from data: Data, side: CGFloat = 256) -> Data? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true,
                  kCGImageSourceCreateThumbnailWithTransform: true,
                  kCGImageSourceThumbnailMaxPixelSize: side * 2,
              ] as CFDictionary) else { return nil }
        let edge = min(image.width, image.height)
        let crop = CGRect(x: (image.width - edge) / 2, y: (image.height - edge) / 2, width: edge, height: edge)
        guard let square = image.cropping(to: crop),
              let context = CGContext(data: nil, width: Int(side), height: Int(side), bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        context.interpolationQuality = .high
        context.draw(square, in: CGRect(x: 0, y: 0, width: side, height: side))
        guard let scaled = context.makeImage() else { return nil }
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out, "public.jpeg" as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, scaled, [kCGImageDestinationLossyCompressionQuality: 0.85] as CFDictionary)
        guard CGImageDestinationFinalize(dest) else { return nil }
        return out as Data
    }
}

/// Mic level vs. the voice-activity threshold: a sunken bar, green when
/// over; "Too loud (clipping)" under it when the peaks keep hitting the top.
struct LevelMeter: View {
    var level: Double
    var threshold: Double
    var tooLoud = false

    var body: some View {
        let db = level > 0 ? 20 * log10(level) : -100
        let fraction = { (v: Double) in max(0, min(1, (v + 80) / 80)) }
        VStack(alignment: .leading, spacing: 3) {
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Theme.window
                    (tooLoud ? Theme.danger : db > threshold ? Theme.ok : Theme.grayText)
                        .frame(width: geo.size.width * fraction(db))
                    Theme.text.frame(width: 1)
                        .offset(x: geo.size.width * fraction(threshold))
                }
            }
            .frame(height: 10)
            .overlay(Bevel(style: .status))
            .accessibilityHidden(true)
            if tooLoud {
                WarningLine98(text: TalkWords.tooLoud, detail: TalkWords.tooLoudDetail)
            }
        }
    }
}

/// Change password (Set a password while still on an old PIN): the
/// current password, or the PIN, and a new password of 8 characters or more.
/// The server's refusals (`bad_pin`, `weak_pin`, a wrong current one) show
/// in its own words.
struct ChangePasswordView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var current = ""
    @State private var new = ""
    @State private var show = false
    /// Read once: the title mustn't flip to "Change password" as it saves.
    @State private var onPin = false
    @State private var opened = false

    var body: some View {
        let client = model.client
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    Text("\(PasswordText.current(onPin: onPin)):").font(Theme.chrome).foregroundStyle(Theme.text)
                    PasswordField98(text: $current, shown: show)
                    Text("\(PasswordText.new):").font(Theme.chrome).foregroundStyle(Theme.text)
                    PasswordField98(text: $new, isNew: true, shown: show)
                    PasswordRule98()
                    Check98(title: PasswordText.show, isOn: $show)
                    if let failure {
                        ErrorLine(text: failure)
                    }
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet(PasswordText.change(onPin: onPin)) {
                CloseSheetButton(hasChanges: !new.isEmpty || !current.isEmpty) { dismiss() }
            } done: {
                DoneButton(action: save)
                    .disabled(!Secret.isPassword(new) || !Secret.canSignIn(current) || client.pinChange == .saving)
            }
            .onAppear {
                guard !opened else { return }
                opened = true
                onPin = client.onPin
                client.pinChange = .idle
            }
            .onChange(of: client.pinChange) { _, state in
                if state == .saved {
                    client.notice = YapClient.Notice(text: PasswordText.saved(onPin: onPin))
                    dismiss()
                }
            }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(!new.isEmpty)
        #if os(macOS)
        .frame(minWidth: 340, minHeight: 300)
        #endif
    }

    private var failure: String? {
        if case .failed(let message) = model.client.pinChange { return message }
        return nil
    }

    private func save() {
        model.client.changePassword(to: new, current: current)
    }
}
