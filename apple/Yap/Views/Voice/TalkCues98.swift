import SwiftUI
import YapCore
import YapVoice

/// A Win98 tooltip: tooltip yellow, a 1 pt black edge, small black text.
/// "You're muted" by the Mute button.
struct TooltipNote98: View {
    var text: String
    var systemImage: String?

    var body: some View {
        HStack(spacing: 4) {
            if let systemImage { Image(systemName: systemImage).font(.system(size: 10, weight: .bold)) }
            Text(text).font(Theme.small).lineLimit(1)
        }
        .foregroundStyle(Theme.tooltipText)
        .padding(.horizontal, 5)
        .padding(.vertical, 2)
        .background(Theme.tooltip)
        .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
        .fixedSize()
        .accessibilityElement(children: .combine)
    }
}

/// A quiet warning about my own audio (the warning color, a triangle):
/// "Your connection is dropping some of your audio", "Too loud (clipping)",
/// the quality warning at 256/510 kbps.
struct WarningLine98: View {
    var text: String
    var detail: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10)).foregroundStyle(Theme.warn)
            Text(text).font(Theme.small).foregroundStyle(Theme.text).fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint(detail ?? "")
        .help(detail ?? text)
    }
}

/// How loud someone is for me, next to their name in voice, only when I've
/// changed it (PROTOCOL.md › Voice › Your volume, on the row): "60%",
/// "150%", or a muted speaker at 0 %. Nothing at 100 %.
enum PersonVolume {
    /// "60%" / "150%", nil at 100 % or 0 % (that's the glyph).
    static func label(_ volume: Double) -> String? {
        let percent = Int((volume * 100).rounded())
        return percent == 100 || percent == 0 ? nil : "\(percent)%"
    }

    static func isMuted(_ volume: Double) -> Bool { Int((volume * 100).rounded()) == 0 }

    /// What VoiceOver says after the name.
    static func spoken(_ volume: Double) -> String? {
        if isMuted(volume) { return "muted for you" }
        return label(volume).map { "\($0) for you" }
    }

    /// Ctrl/⌘ + scroll: 5 % a step, 0–200 %.
    static func stepped(_ volume: Double, by steps: Int) -> Double {
        min(2, max(0, ((volume * 20).rounded() + Double(steps)) / 20))
    }
}

/// The badge itself: small gray text, or the muted-speaker glyph.
struct VolumeBadge98: View {
    var volume: Double

    var body: some View {
        if PersonVolume.isMuted(volume) {
            Image(systemName: "speaker.slash")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(Theme.grayText)
                .help("Muted for you (0%)")
        } else if let label = PersonVolume.label(volume) {
            Text(label)
                .font(Theme.small.monospacedDigit())
                .foregroundStyle(Theme.grayText)
                .help("Their volume for you")
        }
    }
}

/// "Live — tap to stop" by the push-to-talk button after a double-tap, or
/// "You're muted" while the nudge shows; nil otherwise. The call bar's
/// status line, and what VoiceOver hears.
@MainActor
enum CallCue {
    static func line(_ voice: VoiceController) -> String? {
        if voice.mutedNudgeShown { return TalkWords.youreMuted }
        if voice.settings.mode == .pushToTalk, voice.engine.pushToTalkLatched { return TalkWords.live }
        return nil
    }
}
