import SwiftUI
import YapCore
import YapVoice

// Settings for the v0.36 extras in the Windows 98 skin: the away message,
// sounds and chimes, the soundboard, notifications and quiet hours.

/// AIM's away message: type it, pick when it ends, Set.
struct AwayGroup98: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @State private var choice = AwayEnd.back

    var body: some View {
        let client = model.client
        GroupBox98(title: "Away message") {
            if let away = client.me?.awayNow() {
                Tooltip98(text: away.text)
                let until = AwayPresets.untilLabel(away)
                Text(until.isEmpty ? "Until you're back." : "On \(until).").font(Theme.small).foregroundStyle(Theme.hint)
                Button("I'm back") { client.clearAway() }.buttonStyle(.win98)
            } else {
                Field98 {
                    TextField("", text: $text, prompt: Text("at work til 6"))
                        .onSubmit(set)
                }
                .onChange(of: text) { _, new in if new.count > Away.maxLength { text = String(new.prefix(Away.maxLength)) } }
                FlowLayout(spacing: 4) {
                    ForEach(AwayPresets.suggestions, id: \.self) { suggestion in
                        Button(suggestion) { text = suggestion }.buttonStyle(.win98Compact)
                    }
                }
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(AwayPresets.choices()) { end in
                        Radio98(title: end.title, isOn: choice == end) { choice = end }
                    }
                }
                Button("Set") { set() }
                    .buttonStyle(.win98)
                    .disabled(text.isBlank)
            }
            Text(SettingsText.awayNote98)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func set() {
        model.client.setAway(text, until: choice.until())
        text = ""
    }
}

/// Voice › Sounds: Yap's blips, your join chime, the soundboard's volume and mute.
struct SoundsGroup98: View {
    @Environment(AppModel.self) private var model
    @AppStorage(SoundSettings.soundsKey) private var sounds = true
    @AppStorage(SoundSettings.volumeKey) private var volume = Soundboard.defaultVolume
    @AppStorage(SoundSettings.mutedKey) private var muted = false
    @State private var uploading: SoundUpload.Kind?

    var body: some View {
        let client = model.client
        let mine = client.me?.chime ?? "default"
        GroupBox98(title: SettingsText.sounds) {
            Check98(title: SettingsText.joinSounds, isOn: $sounds)
            Text("\(Text("\(SettingsText.chime)").foregroundStyle(Theme.text)) \(Text("— what everyone hears when you join their voice channel").foregroundStyle(Theme.hint).font(Theme.small))")
                .font(Theme.chrome)
                .fixedSize(horizontal: false, vertical: true)
            FlowLayout(spacing: 6) {
                Dropdown98(title: mine.hasPrefix("s:") ? "Your own clip" : (ChimeTone.all.first { $0.id == mine }?.title ?? ChimeTone.all[0].title),
                           accessibility: SettingsText.chime) {
                    Picker(SettingsText.chime, selection: Binding(get: { mine }, set: { client.setChime($0 == "default" ? nil : $0) })) {
                        ForEach(ChimeTone.all) { tone in Text(tone.title).tag(tone.id) }
                        if mine.hasPrefix("s:") {
                            Text("Your own clip").tag(mine)
                        }
                    }
                }
                Button("▶ Play") { model.playChime(ChimeChoice.from(client.me?.chime)) }.buttonStyle(.win98)
                Button(SettingsText.ownClip) { uploading = .chime }.buttonStyle(.win98)
            }
            Text(SettingsText.ownClipNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Text("\(Text("\(SettingsText.boardVolume)").foregroundStyle(Theme.text)) \(Text("— never louder than each clip's safe level").foregroundStyle(Theme.hint).font(Theme.small))")
                    .font(Theme.chrome)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
                Text("\(Int((volume * 100).rounded()))%").font(Theme.chrome.monospacedDigit()).foregroundStyle(Theme.text)
            }
            Slider(value: $volume, in: 0...1, step: 0.05).disabled(muted)
            Check98(title: SettingsText.boardMute, isOn: $muted)
                .onChange(of: muted) { _, on in if on { model.sounds.stopClips() } }
        }
        .sheet(item: $uploading) { kind in SoundUpload98(kind: kind) }
    }
}

/// The soundboard's clips: preview, delete (yours, or anyone's for mods), add.
struct SoundboardGroup98: View {
    @Environment(AppModel.self) private var model
    @AppStorage(SoundSettings.volumeKey) private var volume = Soundboard.defaultVolume
    @State private var uploading: SoundUpload.Kind?
    @State private var deleting: Sound?

    var body: some View {
        let client = model.client
        GroupBox98(title: SettingsText.clips) {
            VStack(alignment: .leading, spacing: 0) {
                if client.boardSounds.isEmpty {
                    Text("No sounds yet.").font(Theme.chrome).foregroundStyle(Theme.grayText).padding(4)
                }
                ForEach(client.boardSounds) { sound in
                    HStack(spacing: 6) {
                        Text(sound.emoji ?? "🔊")
                        Text(sound.name).font(Theme.chat).foregroundStyle(Theme.text)
                        Text("(\(String(format: "%.1f", Double(sound.ms) / 1000)) s, \(client.name(of: sound.uid)))")
                            .font(Theme.small).foregroundStyle(Theme.grayText).lineLimit(1)
                        Spacer(minLength: 4)
                        Button("▶") {
                            if let endpoint = client.endpoint {
                                model.sounds.play(clip: sound, gain: Soundboard.playbackGain(clipGain: sound.gain, volume: volume), from: endpoint)
                            }
                        }
                        .buttonStyle(.win98Compact)
                        .accessibilityLabel("Preview \(sound.name)")
                        if sound.uid == client.uid || client.isMod {
                            Button("Delete") { deleting = sound }.buttonStyle(.win98Compact)
                        }
                    }
                    .padding(.horizontal, 4)
                    .padding(.vertical, 2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .well()
            Button("Add a sound…") { uploading = .board }
                .buttonStyle(.win98)
                .disabled(client.boardSounds.count >= Soundboard.maxBoardSounds)
            Text(SettingsText.clipsNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
        .sheet(item: $uploading) { kind in SoundUpload98(kind: kind) }
        .question(isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                  .deleteSound(deleting?.name ?? "this sound") { [deleting] in if let deleting { client.deleteSound(deleting) } })
    }
}

/// Settings › Notifications: On this device, On your phone (ntfy), Quiet
/// hours (which also quiet Yap's own notifications and chimes here).
struct NotificationsGroup98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL

    var body: some View {
        let client = model.client
        let mode = client.notifySettings?.mode ?? "off"
        let quiet = client.notifySettings?.quiet
        GroupBox98(title: SettingsText.onThisDevice) {
            Text(SettingsText.deviceNote)
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            if let url = SettingsText.deviceSettingsURL {
                Button(SettingsText.deviceButton) { openURL(url) }.buttonStyle(.win98)
            }
        }
        GroupBox98(title: SettingsText.onYourPhone) {
            Text("\(Text("\(SettingsText.pushes):").foregroundStyle(Theme.text)) \(Text("— via the free ntfy app").foregroundStyle(Theme.hint).font(Theme.small))")
                .font(Theme.chrome)
            VStack(alignment: .leading, spacing: 0) {
                ForEach(SettingsText.pushModes, id: \.value) { option in
                    Radio98(title: option.title, isOn: mode == option.value) { client.setPushMode(option.value) }
                }
            }
            if let topic = client.notifySettings?.topic, mode != "off" {
                Text("Subscribe to this topic in the ntfy app:").font(Theme.chrome).foregroundStyle(Theme.text)
                Text(topic).font(Theme.mono).foregroundStyle(Theme.text).textSelection(.enabled).padding(4).well()
            }
            Text(SettingsText.pushesNote)
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
        }
        GroupBox98(title: SettingsText.quietHours) {
            Text(SettingsText.quietNote())
                .font(Theme.small)
                .foregroundStyle(Theme.hint)
                .fixedSize(horizontal: false, vertical: true)
            Check98(title: SettingsText.quietOn, isOn: Binding(get: { quiet != nil }, set: { client.setQuietHours($0 ? QuietHours() : nil) }))
            if let quiet {
                HStack(spacing: 8) {
                    Text("From:").font(Theme.chrome).foregroundStyle(Theme.text)
                    DateField98(selection: minutes(quiet.from) { var q = quiet; q.from = $0; return q }, components: .hourAndMinute)
                    Text("to:").font(Theme.chrome).foregroundStyle(Theme.text)
                    DateField98(selection: minutes(quiet.to) { var q = quiet; q.to = $0; return q }, components: .hourAndMinute)
                }
                Check98(title: SettingsText.quietMentions, isOn: Binding(get: { quiet.mentions }, set: { on in
                    var q = quiet
                    q.mentions = on
                    client.setQuietHours(q)
                }))
                if quiet.isSnoozed(at: .now), let until = quiet.until {
                    HStack {
                        Text("Snoozed until \(Date(timeIntervalSince1970: Double(until) / 1000).formatted(date: .omitted, time: .shortened)).")
                            .font(Theme.chrome).foregroundStyle(Theme.text)
                        Button("Resume") {
                            var q = quiet
                            q.until = nil
                            client.setQuietHours(q)
                        }
                        .buttonStyle(.win98)
                    }
                } else {
                    Button("Snooze until tomorrow morning") {
                        var q = quiet
                        q.until = quiet.tomorrowMorning().yapMillis
                        client.setQuietHours(q)
                    }
                    .buttonStyle(.win98)
                }
            }
        }
    }

    private func minutes(_ value: Int, _ apply: @escaping (Int) -> QuietHours) -> Binding<Date> {
        Binding(get: {
            Calendar.current.date(bySettingHour: value / 60, minute: value % 60, second: 0, of: .now) ?? .now
        }, set: { date in
            let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
            var quiet = apply((parts.hour ?? 0) * 60 + (parts.minute ?? 0))
            quiet.tz = TimeZone.current.identifier
            model.client.setQuietHours(quiet)
        })
    }
}
