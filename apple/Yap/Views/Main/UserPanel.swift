import SwiftUI
import YapCore
import YapVoice

/// The top of the channels pane (Mac) / channels screen (iPhone), right
/// under the title bar: your profile card — one flat button that opens
/// Settings on Profile — and two big Mute / Deafen buttons, like the web
/// client's user panel.
struct UserPanel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let settings = model.voice.settings
        let muted = settings.muted || settings.deafened
        VStack(spacing: 4) {
            Button(action: openProfile) {
                HStack(spacing: 7) {
                    Avatar(user: model.client.me, size: 32, speaking: model.voice.ownSid.map { model.voice.engine.speaking.contains($0) } ?? false)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(model.client.me?.name ?? model.client.savedName ?? "…")
                            .font(Theme.chatBold)
                            .foregroundStyle(Theme.text)
                            .lineLimit(1)
                        Text(model.client.me?.awayNow().map { "Away: \($0.text)" } ?? "Profile & settings")
                            .font(Theme.small)
                            .foregroundStyle(Theme.hint)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    Image(systemName: "gearshape.fill")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.grayText)
                }
                .padding(.horizontal, 5)
                .padding(.vertical, 3)
            }
            .buttonStyle(FlatToolbarButtonStyle())
            .accessibilityLabel("\(model.client.me?.name ?? "You"), settings")
            .accessibilityHint("Opens your profile and settings")
            .help("Your profile and settings")

            HStack(spacing: 4) {
                BigToggleButton(title: muted ? "Muted" : "Mute",
                                systemImage: muted ? "mic.slash.fill" : "mic.fill",
                                isOn: muted,
                                help: muted ? "Unmute your mic (⌘⇧M)" : "Mute your mic (⌘⇧M)") { model.voice.toggleMute() }
                    // Talking while muted: "You're muted" hangs under the button for a moment.
                    .overlay(alignment: .bottomLeading) {
                        if model.voice.mutedNudgeShown {
                            // Hangs from the button's bottom edge (a zero-height frame there).
                            TooltipNote98(text: TalkWords.youreMuted, systemImage: "mic.slash.fill")
                                .frame(height: 0, alignment: .top)
                                .offset(x: 8, y: 3)
                                .transition(.opacity)
                                .allowsHitTesting(false)
                        }
                    }
                BigToggleButton(title: settings.deafened ? "Deafened" : "Deafen",
                                systemImage: settings.deafened ? "speaker.slash.fill" : "headphones",
                                isOn: settings.deafened,
                                help: settings.deafened ? "Undeafen (⌘⇧D)" : "Deafen: hear nobody, and mute (⌘⇧D)") { model.voice.toggleDeafen() }
            }
            .zIndex(1)
        }
        .padding(.horizontal, 2)
        .padding(.top, 2)
        .padding(.bottom, 5)
        .background(Theme.face)
        .overlay(alignment: .bottom) {
            // Etched line below, between the panel and the list.
            Separator98()
        }
        // The tooltip may hang over whatever's below the panel.
        .zIndex(1)
        .animation(.easeOut(duration: 0.15), value: model.voice.mutedNudgeShown)
    }

    private func openProfile() {
        model.showSettings(.profile)
    }
}

/// A big Win98 toggle (Mute, Deafen): icon + bold label, half the width;
/// while on it stays pushed in and the label turns the danger color.
struct BigToggleButton: View {
    var title: String
    var systemImage: String
    var isOn: Bool
    var help: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .font(.system(size: 18))
                    .frame(width: 22)
                Text(title)
                    .font(Theme.chromeBold)
            }
            .foregroundStyle(isOn ? Theme.danger : Theme.text)
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(BigToggleStyle(isOn: isOn))
        .accessibilityLabel(title)
        .accessibilityAddTraits(isOn ? .isSelected : [])
        .help(help)
        .sensoryFeedback(.impact(weight: .light), trigger: isOn)
    }
}

private struct BigToggleStyle: ButtonStyle {
    var isOn: Bool

    func makeBody(configuration: Configuration) -> some View {
        let down = isOn || configuration.isPressed
        configuration.label
            .padding(.horizontal, 6)
            #if os(iOS)
            .frame(minHeight: 44)
            #else
            .frame(minHeight: 36)
            #endif
            .offset(x: down ? 1 : 0, y: down ? 1 : 0)
            .background(Theme.face)
            .overlay(Bevel(style: down ? .pressed : .raised))
            .contentShape(.rect)
    }
}

/// A flat toolbar-style button: no border at rest, a thin raised ring on
/// hover (Mac), a thin sunken ring while pressed.
struct FlatToolbarButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        FlatToolbarButton(configuration: configuration)
    }

    private struct FlatToolbarButton: View {
        let configuration: Configuration
        @State private var hovering = false

        var body: some View {
            configuration.label
                .frame(maxWidth: .infinity, alignment: .leading)
                #if os(iOS)
                .frame(minHeight: 44)
                #endif
                .offset(x: configuration.isPressed ? 1 : 0, y: configuration.isPressed ? 1 : 0)
                .overlay {
                    if configuration.isPressed {
                        Bevel(style: .status)
                    } else if hovering {
                        Bevel(style: .thin)
                    }
                }
                .contentShape(.rect)
                .onHover { hovering = $0 }
        }
    }
}
