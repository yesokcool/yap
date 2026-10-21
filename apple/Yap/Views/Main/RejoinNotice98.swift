import SwiftUI
import YapCore
import YapVoice

/// After a relaunch (iOS stopped Yap, a crash, quitting mid-call): "You
/// were in Lounge." with Rejoin and ×, for 10 minutes after the call. A
/// raised bar like a hang's; it never joins by itself.
struct RejoinNotice98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let call = model.rejoinOffer.call,
           let channel = model.client.channelsById[call.channelId],
           call.shows(inVoice: model.voice.isInVoice, channel: channel, now: Date()) {
            HStack(spacing: 6) {
                Image(systemName: "speaker.wave.2.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Theme.text)
                    .accessibilityHidden(true)
                Text(RejoinWords.line(channel.name))
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 4)
                Button(RejoinWords.rejoin) { model.rejoinOffered() }
                    .buttonStyle(.win98Default)
                Button { model.rejoinOffer.dismiss() } label: {
                    Image(systemName: "xmark").font(.system(size: 9, weight: .black))
                }
                .buttonStyle(.win98Compact)
                .accessibilityLabel("Dismiss")
            }
            .padding(6)
            .bevel(.raised)
        }
    }
}

/// The web's words for the bar.
enum RejoinWords {
    static func line(_ channel: String) -> String { "You were in \(channel)." }
    static let rejoin = "Rejoin"
}
