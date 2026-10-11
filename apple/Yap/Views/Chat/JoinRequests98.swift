import SwiftUI
import YapCore

/// Join requests above the chat log, like the web's Windows 98 bars:
/// tooltip yellow with a 1pt black edge, "Dee wants to join · invited by
/// Ada", the passed-on-link warning in dark red, then Let in (the default
/// button) and Turn away. Folded into one bar with Review when they'd crowd
/// the chat (`JoinRequests`).
struct JoinRequests98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let client = model.client
        let waiting = JoinRequests.waiting(client)
        if !waiting.isEmpty {
            VStack(spacing: 3) {
                if waiting.count >= JoinRequests.summaryFrom(model) {
                    let summary = JoinRequests.summary(waiting)
                    bar {
                        Text("\(Text(summary.bold).bold())\(summary.rest)")
                    } buttons: {
                        Button("Review") { model.showingPeople = true }
                            .buttonStyle(Win98ButtonStyle(isDefault: true, compact: true))
                    }
                } else {
                    ForEach(waiting) { user in
                        bar {
                            HStack(alignment: .top, spacing: 6) {
                                Avatar(user: user, size: 16)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text("\(Text(user.name).bold()) wants to join\(Text(user.invitedBy.map { " · invited by \($0)" } ?? "").font(Theme.small).foregroundStyle(Self.muted))")
                                    if let warning = user.invite?.warning {
                                        // A personal link that was passed on: check before letting them in.
                                        Label {
                                            Text(warning).fixedSize(horizontal: false, vertical: true)
                                        } icon: {
                                            Image(systemName: "exclamationmark.triangle.fill")
                                        }
                                        .font(Theme.small)
                                        .foregroundStyle(Self.warning)
                                    }
                                }
                            }
                        } buttons: {
                            Button("Let in") { client.approve(user.id) }
                                .buttonStyle(Win98ButtonStyle(isDefault: true, compact: true))
                            Button("Turn away") { client.deny(user.id) }
                                .buttonStyle(.win98Compact)
                        }
                    }
                }
            }
        }
    }

    /// The web's `.request .muted` and `.request .invite-warn` on yellow.
    private static let muted = Color(white: 0x55 / 255)
    private static let warning = Color(red: 0x80 / 255, green: 0, blue: 0)

    private func bar<Message: View, Buttons: View>(@ViewBuilder message: () -> Message, @ViewBuilder buttons: () -> Buttons) -> some View {
        let text = message()
            .font(Theme.chrome)
            .foregroundStyle(Theme.tooltipText)
            .fixedSize(horizontal: false, vertical: true)
        let actions = HStack(spacing: 4) { buttons() }.fixedSize()
        // Side by side; on a narrow screen or with big text the buttons get their own line.
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                text
                Spacer(minLength: 4)
                actions
            }
            VStack(alignment: .trailing, spacing: 4) {
                text.frame(maxWidth: .infinity, alignment: .leading)
                actions
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Theme.tooltip)
        .overlay(Rectangle().strokeBorder(Theme.dkShadow, lineWidth: 1))
        .accessibilityElement(children: .contain)
    }
}
