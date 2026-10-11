import SwiftUI
import YapCore

/// A newcomer waits here until someone already in Yap lets them in.
struct WaitingRoomView: View {
    @Environment(AppModel.self) private var model
    var message: String

    var body: some View {
        DesktopDialog(title: "Waiting room", systemImage: "hourglass") {
            HStack(alignment: .top, spacing: 12) {
                // Turns over once a second, in one step, like Windows' hourglass
                // (Win98 has no smooth motion).
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    let flipped = Int(context.date.timeIntervalSinceReferenceDate) % 2 == 1
                    Image(systemName: "hourglass")
                        .font(.system(size: 30))
                        // The text color reads on Charcoal and Windows Standard alike (navy vanished on Charcoal).
                        .foregroundStyle(Theme.text)
                        .rotationEffect(.degrees(flipped ? 180 : 0))
                }
                .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 8) {
                    Text("Waiting to be let in").font(Theme.chromeBold)
                    Text(AuthText.waiting(model.client.savedName))
                        .fixedSize(horizontal: false, vertical: true)
                    // The server's line usually says the same thing again; show it only when it adds something.
                    if !message.hasPrefix("Waiting for someone") {
                        Text(message).foregroundStyle(Theme.grayText)
                    }
                }
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
            }
            Separator98()
            HStack {
                Spacer()
                Button("Use a different account") { model.client.signOut() }
                    .buttonStyle(.win98)
            }
        }
    }
}

/// Banned or turned away: no more reconnects, just a way to start over.
struct BlockedView: View {
    @Environment(AppModel.self) private var model
    var title: String
    var message: String

    var body: some View {
        DesktopDialog(title: "Yap", systemImage: "exclamationmark.triangle.fill") {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "xmark.octagon.fill")
                    .font(.system(size: 30))
                    .foregroundStyle(.white, Theme.danger)
                VStack(alignment: .leading, spacing: 6) {
                    Text(title).font(Theme.chromeBold)
                    Text(message).font(Theme.chrome).fixedSize(horizontal: false, vertical: true)
                }
                .foregroundStyle(Theme.text)
            }
            HStack {
                Spacer()
                Button("Start over") { model.client.forgetServer() }
                    .buttonStyle(.win98Default)
            }
        }
    }
}
