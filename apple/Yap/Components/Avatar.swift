import SwiftUI
import YapCore
import YapVoice

/// A buddy icon: the person's picture (or their initial on their color) in a
/// square sunken frame. Someone speaking gets a solid green frame.
struct Avatar: View {
    @Environment(AppModel.self) private var model
    var user: User?
    var size: CGFloat = 32
    var speaking = false
    /// They're speaking but I've turned them to 0 %: the ring shows, dimmed.
    var speakingDimmed = false

    var body: some View {
        ZStack {
            Theme.color(hex: user?.color ?? "#808080")
            if let picture {
                Image(platformImage: picture)
                    .resizable()
                    .scaledToFill()
            } else {
                Text(initial)
                    .font(.system(size: size * 0.5, weight: .bold))
                    .foregroundStyle(.white)
            }
        }
        .frame(width: size, height: size)
        .clipped()
        .overlay(Bevel(style: .sunken))
        .padding(2)
        .overlay {
            if speaking {
                Rectangle().stroke(Theme.speaking.opacity(speakingDimmed ? 0.35 : 1), lineWidth: 2)
            }
        }
        .accessibilityLabel(user?.name ?? "Someone")
        .accessibilityValue(speaking ? (speakingDimmed ? "Speaking, muted for you" : "Speaking") : "")
    }

    private var picture: PlatformImage? {
        guard let id = user?.avatar, let endpoint = model.client.endpoint else { return nil }
        return model.images.image(for: endpoint.mediaURL(id))
    }

    private var initial: String {
        (user?.name.trimmingCharacters(in: .whitespaces).first).map { String($0).uppercased() } ?? "?"
    }
}

/// "(owner)" / "(mod)" after a name, in gray.
struct RoleBadge: View {
    var role: Role

    var body: some View {
        switch role {
        case .owner, .mod:
            Text(role == .owner ? "(owner)" : "(mod)")
                .font(Theme.small)
                .foregroundStyle(Theme.grayText)
        default:
            EmptyView()
        }
    }
}

