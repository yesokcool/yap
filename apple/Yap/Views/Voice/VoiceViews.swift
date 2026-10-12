import AVKit
import SwiftUI
import YapCore
import YapVoice

/// The call, compact: rides in the tab bar's bottom accessory (iPhone) or
/// the bottom of the channels pane (Mac). Tap the name for the full panel.
struct VoiceBar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let voice = model.voice
        let channel = voice.wantedChannelId.flatMap { model.client.channelsById[$0] }
        HStack(spacing: 6) {
            Button { model.showingVoicePanel = true } label: {
                HStack(spacing: 6) {
                    Image(systemName: "speaker.wave.2.fill")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(voice.isConnected ? Theme.ok : Theme.warn)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(channel?.name ?? "Voice").font(Theme.chromeBold).foregroundStyle(Theme.text).lineLimit(1)
                        Text(statusLine)
                            .font(Theme.small)
                            .foregroundStyle(statusIsWarning ? Theme.warn : Theme.hint)
                            .lineLimit(statusIsWarning ? 2 : 1)
                            .help(voice.ownConnectionWeak ? TalkWords.ownConnectionDetail(lost: voice.engine.ownLinkLost) : statusLine)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Opens the call")
            Spacer(minLength: 4)
            if voice.settings.mode == .pushToTalk {
                PushToTalkButton(compact: true)
            }
            MuteButton(compact: true)
            #if os(macOS)
            ShareScreenButton()
            #endif
            Button("Leave") { voice.leave() }
                .buttonStyle(.win98Compact)
                .accessibilityLabel("Leave voice")
        }
        .padding(.horizontal, 8)
        #if os(macOS)
        .padding(.vertical, 5)
        .bevel(.raised)
        #endif
    }

    /// "You're muted", "Your connection is dropping some of your audio": in the warning color.
    private var statusIsWarning: Bool {
        let voice = model.voice
        return !voice.isResuming && !voice.micSendsSilence && (voice.mutedNudgeShown || voice.ownConnectionWeak)
    }

    private var statusLine: String {
        let voice = model.voice
        if voice.isResuming { return VoiceHintText.resuming }
        if voice.micSendsSilence { return VoiceHintText.silence }
        if let cue = CallCue.line(voice) { return cue }
        if voice.ownConnectionWeak { return TalkWords.ownConnection }
        if let quality = voice.sharing { return ScreenText.sharing(quality) }
        if !voice.isConnected { return voice.micDenied ? "Listening only — no mic" : "Connecting…" }
        var parts: [String] = []
        if voice.micDenied { parts.append("Listening only") }
        if let rtt = voice.engine.summary?.rttMs { parts.append("\(rtt) ms") }
        if let shared = voice.wantedChannelId.flatMap({ model.client.channelsById[$0] })?.bitrate { parts.append("\(shared / 1000)k for everyone") }
        return parts.isEmpty ? "Connected" : parts.joined(separator: " · ")
    }
}

#if os(macOS)
/// Share screen / Stop sharing, next to Mute in the call bar (Mac).
struct ShareScreenButton: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let sharing = model.voice.sharing != nil
        Button {
            if sharing { model.voice.stopSharing() } else { model.shareScreen() }
        } label: {
            Label(sharing ? ScreenText.stopSharing : ScreenText.share, systemImage: "display")
                .labelStyle(.iconOnly)
                .foregroundStyle(sharing ? Theme.danger : Theme.text)
        }
        .buttonStyle(Win98ToggleButtonStyle(isOn: sharing, compact: true))
        .help(sharing ? ScreenText.stopSharing : "\(ScreenText.share): pick a window or a screen")
    }
}
#endif

struct MuteButton: View {
    @Environment(AppModel.self) private var model
    var compact = false

    var body: some View {
        let settings = model.voice.settings
        let muted = settings.muted || settings.deafened || model.voice.micDenied
        Button { model.voice.toggleMute() } label: {
            Label(muted ? "Unmute" : "Mute", systemImage: muted ? "mic.slash.fill" : "mic.fill")
                .labelStyle(.iconOnly)
                .foregroundStyle(muted ? Theme.danger : Theme.text)
        }
        .buttonStyle(Win98ToggleButtonStyle(isOn: muted, compact: compact))
        .sensoryFeedback(.impact(weight: .light), trigger: settings.muted)
        .help(muted ? "Unmute (⌘⇧M)" : "Mute (⌘⇧M)")
    }
}

/// Hold to talk (push-to-talk mode): a press-and-hold gesture, so it works
/// with a thumb without lifting. Drawn as a big Win98 button that stays
/// pushed in while you talk.
struct PushToTalkButton: View {
    @Environment(AppModel.self) private var model
    var compact = false
    @GestureState private var pressed = false

    var body: some View {
        let down = model.voice.engine.pushToTalkDown
        let latched = model.voice.engine.pushToTalkLatched
        Group {
            if compact {
                Image(systemName: latched ? "waveform.badge.mic" : "waveform")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(down ? Theme.ok : Theme.text)
                    .frame(minWidth: 30, minHeight: 26)
                    .offset(x: down ? 1 : 0, y: down ? 1 : 0)
                    .bevel(down ? .pressed : .raised, fill: down ? Theme.light : Theme.face)
                    .help(latched ? TalkWords.live : "Hold to talk; double-tap to stay live")
            } else {
                VStack(spacing: 4) {
                    Image(systemName: latched ? "waveform.badge.mic" : "waveform").font(.system(size: 26, weight: .bold))
                    Text(latched ? TalkWords.live : down ? "Talking…" : "Hold to talk").font(Theme.chromeBold)
                    if !down {
                        Text("Double-tap to stay live").font(Theme.small).foregroundStyle(Theme.hint)
                    }
                }
                .foregroundStyle(down ? Theme.ok : Theme.text)
                .frame(maxWidth: .infinity)
                .frame(height: 84)
                .offset(x: down ? 1 : 0, y: down ? 1 : 0)
                .bevel(down ? .pressed : .raised, fill: down ? Theme.light : Theme.face)
            }
        }
        .contentShape(.rect)
        .gesture(
            DragGesture(minimumDistance: 0)
                .updating($pressed) { _, state, _ in state = true }
        )
        .onChange(of: pressed) { _, isPressed in model.voice.setPushToTalk(isPressed) }
        .sensoryFeedback(.impact(weight: .medium), trigger: pressed)
        .accessibilityLabel("Push to talk")
        .accessibilityValue(latched ? TalkWords.live : down ? "Talking" : "")
        .accessibilityHint("Hold to talk. Double-tap to stay live; tap again to stop.")
        .accessibilityAddTraits(.isButton)
    }
}

/// The full call: who's here (speaking = green frame), mute / deafen /
/// output / leave, push to talk, quality.
struct VoicePanel: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let voice = model.voice
        let channelId = model.callSheetChannel
        let sessions = model.client.voiceRoster(in: channelId)
        let obs = model.client.obsStillListening(in: channelId)
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(sessions) { session in
                            person(session)
                        }
                        // OBS links whose account left: greyed out, not in the call.
                        ForEach(obs) { session in
                            ObsListeningRow98(session: session, large: true)
                        }
                        if sessions.count <= 1, obs.isEmpty {
                            Text("Nobody else here yet. Friends hear you the moment they join.")
                                .font(Theme.chrome)
                                .foregroundStyle(Theme.grayText)
                                .padding(8)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .well()

                    if voice.settings.mode == .pushToTalk {
                        PushToTalkButton()
                    }

                    HStack(spacing: 6) {
                        Button { voice.toggleMute() } label: {
                            Label(voice.settings.muted ? "Unmute" : "Mute", systemImage: voice.settings.muted || voice.settings.deafened ? "mic.slash.fill" : "mic.fill")
                        }
                        .buttonStyle(Win98ToggleButtonStyle(isOn: voice.settings.muted || voice.settings.deafened))
                        Button { voice.toggleDeafen() } label: {
                            Label(voice.settings.deafened ? "Undeafen" : "Deafen", systemImage: voice.settings.deafened ? "speaker.slash.fill" : "headphones")
                        }
                        .buttonStyle(Win98ToggleButtonStyle(isOn: voice.settings.deafened))
                        #if os(iOS)
                        RoutePicker()
                            .frame(width: 36, height: Theme.minButtonSize.height)
                            .bevel(.raised)
                            .accessibilityLabel("Audio output")
                        #endif
                        Spacer(minLength: 0)
                        if let channel = model.client.channelsById[channelId] {
                            // The channel's sound quality, out of the way (the web's ⋯).
                            Menu {
                                CallQualityMenuItems(channel: channel)
                                if voice.isInVoice {
                                    Divider()
                                    Button(CallReportWords.copy, systemImage: "doc.on.clipboard") { model.copyCallReport() }
                                }
                            } label: {
                                Image(systemName: "ellipsis").font(.system(size: 13, weight: .bold))
                            }
                            .menuStyle(.button)
                            .buttonStyle(.win98Compact)
                            .menuIndicator(.hidden)
                            .fixedSize()
                            .accessibilityLabel(CallQuality.menuLabel)
                            .help(CallQuality.menuTitle)
                        }
                        Button("Leave") {
                            voice.leave()
                            dismiss()
                        }
                        .buttonStyle(.win98)
                        .accessibilityLabel("Leave voice")
                    }
                    .labelStyle(.titleAndIcon)

                    if voice.mutedNudgeShown {
                        TooltipNote98(text: TalkWords.youreMuted, systemImage: "mic.slash.fill")
                    }

                    // Said only when someone set one quality for the whole channel (⋯ changes it).
                    if let shared = CallQuality.shared(model.client.channelsById[channelId]) {
                        Text(shared)
                            .font(Theme.chrome)
                            .foregroundStyle(Theme.text)
                            .help("Set for everyone in this channel (⋯ changes it)")
                        if let bitrate = model.client.channelsById[channelId]?.bitrate, let warning = VoiceSettings.qualityWarning(bitrate) {
                            WarningLine98(text: warning)
                        }
                    }

                    if voice.ownConnectionWeak {
                        WarningLine98(text: TalkWords.ownConnection, detail: TalkWords.ownConnectionDetail(lost: voice.engine.ownLinkLost))
                    }

                    if voice.micDenied {
                        ErrorLine(text: "Microphone is off for Yap — you can listen but not talk. Allow it in Settings.")
                    } else if voice.micSendsSilence {
                        ErrorLine(text: VoiceHintText.silence)
                            .help(VoiceHintText.silenceDetail)
                            .accessibilityHint(VoiceHintText.silenceDetail)
                    }

                    if VoiceController.canShareScreen, voice.channelId != nil, voice.channelId == channelId {
                        ScreenShareGroup98(model: model)
                    }

                    Soundboard98()
                }
                .padding(10)
            }
            .background(Theme.face)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HStack(spacing: 2) {
                    StatusPanel(text: voice.isResuming ? "Reconnecting…" : voice.isConnected ? "Connected" : "Connecting…",
                                systemImage: "circle.fill", color: voice.isConnected ? Theme.ok : Theme.warn)
                    StatusPanel(text: voice.engine.summary?.rttMs.map { "\($0) ms" } ?? voice.settings.mode.title)
                }
                .padding(2)
                .background(Theme.face)
            }
            .win98Sheet(model.client.channelsById[channelId]?.name ?? "Voice") {
                CloseSheetButton { dismiss() }
            }
            .onChange(of: voice.isInVoice) { _, inVoice in if !inVoice { dismiss() } }
        }
        .presentationDetents([.medium, .large])
    }

    private func person(_ session: Session) -> some View {
        let user = model.client.user(session.uid)
        let speaking = model.voice.isSpeaking(session)
        let me = session.sid == model.voice.ownSid
        let hint = me ? nil : model.voice.hint(for: session.sid)
        let connecting = !me && hint == nil && model.voice.engine.peerStates[session.sid] != .connected
        // Your volume for them, on the row; someone at 0 % speaks with a dimmed ring.
        let volume = me ? 1 : model.voice.settings.volumes[session.uid] ?? 1
        let silenced = PersonVolume.isMuted(volume)
        return HStack(spacing: 8) {
            Avatar(user: user, size: 28, speaking: speaking, speakingDimmed: silenced)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(me ? "\(user?.name ?? "You") (you)" : (user?.name ?? "Someone"))
                        .font(speaking && !silenced ? Theme.chatBold : Theme.chat)
                        .foregroundStyle(connecting ? Theme.grayText : Theme.text)
                    if !me { VolumeBadge98(volume: volume) }
                    if session.voice?.screen == true {
                        LiveBadge98(session: session)
                    }
                }
                if let hint {
                    VoiceHint98(hint: hint)
                } else if me, model.voice.ownConnectionWeak {
                    Text("your connection is dropping audio")
                        .font(Theme.small)
                        .foregroundStyle(Theme.warn)
                        .lineLimit(1)
                } else {
                    Text(connecting ? "Connecting…" : speaking ? (silenced ? "Speaking (muted for you)" : "Speaking") : "")
                        .font(Theme.small)
                        .foregroundStyle(speaking && !silenced ? Theme.ok : Theme.grayText)
                }
            }
            Spacer()
            if session.voice?.deafened == true {
                Image(systemName: "speaker.slash.fill").foregroundStyle(Theme.danger)
            } else if session.voice?.muted == true {
                Image(systemName: "mic.slash.fill").foregroundStyle(Theme.danger)
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
        .accessibilityValue(me ? "" : PersonVolume.spoken(volume) ?? "")
        .voiceHintHelp(hint, name: user?.name ?? "They")
        .liveBadgeAction(session)
        .voicePersonMenu(session)
    }
}

/// A quiet note next to someone in the call (the web's `.vq`): warning
/// yellow for a weak connection, gray italic while they reconnect, red when
/// there's no way through.
struct VoiceHint98: View {
    var hint: ConnectionHint

    var body: some View {
        let kind = VoiceHintText.kind(hint)
        Text(VoiceHintText.label(hint))
            .font(Theme.small)
            .italic(kind == .away)
            .foregroundStyle(kind == .away ? Theme.grayText : kind == .blocked ? Theme.danger : Theme.warn)
            .lineLimit(1)
    }
}

#if os(iOS)
/// The system audio route picker (speaker, AirPods, car…).
struct RoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let view = AVRoutePickerView()
        view.activeTintColor = UIColor(Theme.ok)
        view.tintColor = UIColor(Theme.text)
        view.prioritizesVideoDevices = false
        return view
    }

    func updateUIView(_ view: AVRoutePickerView, context: Context) {}
}
#endif
