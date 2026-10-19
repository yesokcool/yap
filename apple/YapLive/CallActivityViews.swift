#if os(iOS)
import AppIntents
import SwiftUI
import UIKit

// The call's Live Activity, the Windows 98 way: a little window on the teal
// desktop. Its navy title bar has the channel, the clock and a × that hangs
// up (after "Leave Lounge?"). Under it, an AIM buddy line (you red, friends
// blue, whoever's talking bold with a speaker), then Mute and Deafen, which
// stay pushed in while on. The Dynamic Island has the same parts on black.
//
// In both targets, so the app's tests can draw it; YapLive puts it on screen
// (`YapLiveBundle`). Colors are `Theme`'s (shared from the app). Bevels are
// plain shapes rather than the app's Canvas `Bevel`: iOS draws a Live
// Activity from an archived copy of its views.

typealias CallState = CallActivityAttributes.ContentState

/// The Lock Screen (and the banner on an iPhone without a Dynamic Island).
struct CallLockScreen: View {
    var joinedAt: Date
    var state: CallState
    var isStale: Bool

    var body: some View {
        VStack(spacing: 0) {
            CallTitleBar(joinedAt: joinedAt, state: state, isStale: isStale)
            VStack(alignment: .leading, spacing: 6) {
                if isStale {
                    Text("Yap isn't answering. The call may have ended.")
                        .font(.footnote)
                        .foregroundStyle(Theme.text)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, minHeight: 22, alignment: .leading)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 3)
                        .liveWell()
                } else {
                    BuddyLine(state: state)
                        .liveWell()
                    CallButtonsRow(state: state)
                }
            }
            .padding(6)
        }
        .padding(3)
        .background(Theme.face)
        .overlay(LiveBevel(style: .raised))
        .padding(10)
        .callScheme(state.dark)
    }
}

/// The expanded Dynamic Island's lower half: the buddy line and the buttons
/// on a raised face panel.
struct CallIslandPanel: View {
    var state: CallState
    var isStale: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if isStale {
                Text("Yap isn't answering. The call may have ended.")
                    .font(.footnote)
                    .foregroundStyle(Theme.text)
            } else {
                BuddyLine(state: state)
                    .liveWell()
                CallButtonsRow(state: state)
            }
        }
        .padding(6)
        .background(Theme.face)
        .overlay(LiveBevel(style: .raised))
        .callScheme(state.dark)
    }
}

/// Expanded island, top left: the channel (the speaker goes green while someone talks).
struct CallIslandChannel: View {
    var state: CallState

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: "speaker.wave.2.fill")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(state.anyoneTalking ? Theme.speaking : Theme.titleText)
            Text(state.channel)
                .font(.subheadline.bold())
                .foregroundStyle(Theme.titleText)
                .lineLimit(1)
        }
        .environment(\.colorScheme, .dark)
    }
}

/// Expanded island, top right: the clock and the × (the island is always dark).
struct CallIslandClose: View {
    var joinedAt: Date
    var state: CallState
    var isStale: Bool

    var body: some View {
        HStack(spacing: 2) {
            if !isStale {
                CallClock(joinedAt: joinedAt, status: state.status)
                    .foregroundStyle(Theme.titleText)
            }
            CloseBox()
        }
        .callScheme(state.dark)
    }
}

/// The pill's left: a speaker, green while someone talks.
struct CallCompactLeading: View {
    var state: CallState

    var body: some View {
        Image(systemName: state.anyoneTalking ? "speaker.wave.2.fill" : "speaker.fill")
            .font(.system(size: 13, weight: .bold))
            .foregroundStyle(state.anyoneTalking ? Theme.speaking : Theme.titleText)
            .environment(\.colorScheme, .dark)
    }
}

/// The pill's right: red when you're muted or deafened, else how many are in.
struct CallCompactTrailing: View {
    var state: CallState

    var body: some View {
        Group {
            if state.quiet {
                CallMinimal(state: state)
            } else {
                HStack(spacing: 2) {
                    Image(systemName: "person.fill")
                    Text("\(state.count)")
                }
                .font(.caption.bold())
                .foregroundStyle(Theme.titleText)
            }
        }
        .environment(\.colorScheme, .dark)
    }
}

/// The smallest form (another app's activity shares the island).
struct CallMinimal: View {
    var state: CallState

    var body: some View {
        Group {
            if state.deafened {
                Image(systemName: "speaker.slash.fill").foregroundStyle(Theme.danger)
            } else if state.muted {
                Image(systemName: "mic.slash.fill").foregroundStyle(Theme.danger)
            } else {
                Image(systemName: "speaker.wave.2.fill")
                    .foregroundStyle(state.anyoneTalking ? Theme.speaking : Theme.titleText)
            }
        }
        .font(.system(size: 13, weight: .bold))
        .environment(\.colorScheme, .dark)
    }
}

/// Behind the Lock Screen window: the desktop, in the scheme the window uses.
enum CallDesktop {
    static func color(dark: Bool?) -> Color {
        guard let dark else { return Theme.desktop }
        let traits = UITraitCollection(userInterfaceStyle: dark ? .dark : .light)
        return Color(uiColor: UIColor(Theme.desktop).resolvedColor(with: traits))
    }
}

// MARK: - Parts

private struct CallTitleBar: View {
    var joinedAt: Date
    var state: CallState
    var isStale: Bool

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: "speaker.wave.2.fill")
                .font(.system(size: 10, weight: .bold))
            Text(state.channel)
                .font(.subheadline.bold())
                .lineLimit(1)
            Spacer(minLength: 4)
            if !isStale {
                CallClock(joinedAt: joinedAt, status: state.status)
            }
            CloseBox()
        }
        .foregroundStyle(isStale ? Theme.inactiveTitleText : Theme.titleText)
        .padding(.leading, 5)
        .frame(height: 24)
        .background(
            LinearGradient(colors: isStale ? [Theme.inactiveTitleStart, Theme.inactiveTitleEnd] : [Theme.titleStart, Theme.titleEnd],
                           startPoint: .leading, endPoint: .trailing)
        )
    }
}

/// How long you've been in (it counts by itself), or "Connecting…".
private struct CallClock: View {
    var joinedAt: Date
    var status: String?

    var body: some View {
        Group {
            if let status {
                Text(status)
            } else {
                Text(timerInterval: joinedAt...Date.distantFuture, countsDown: false)
                    .monospacedDigit()
                    .multilineTextAlignment(.trailing)
                    .frame(width: 56, alignment: .trailing)
            }
        }
        .font(.caption.bold())
        .lineLimit(1)
    }
}

/// The title bar's ×: asks "Leave Lounge?" first. (On a stale window it
/// wakes Yap, which closes it.)
private struct CloseBox: View {
    var body: some View {
        Button(intent: CallButtonIntent(.askToLeave)) {
            Image(systemName: "xmark")
                .font(.system(size: 8, weight: .black))
                .foregroundStyle(Theme.text)
                .frame(width: 18, height: 16)
                .background(Theme.face)
                .overlay(LiveBevel(style: .raised))
                .padding(4)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Leave voice")
    }
}

/// Who's in: AIM names (you red, friends blue), whoever's talking in bold
/// with a green speaker, "+2" for the rest.
private struct BuddyLine: View {
    var state: CallState

    var body: some View {
        line
            .font(.footnote)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 22, alignment: .leading)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
    }

    private var line: Text {
        guard !state.people.isEmpty else {
            return Text("Joining…").foregroundStyle(Theme.grayText)
        }
        var line = Text(verbatim: "")
        for (index, person) in state.people.enumerated() {
            if index > 0 { line = Text("\(line)   ") }
            if person.talking {
                line = Text("\(line)\(Text(Image(systemName: "speaker.wave.2.fill")).foregroundStyle(Theme.speaking)) ")
            }
            let name = Text(verbatim: person.name)
                .foregroundStyle(person.me ? Theme.ownName : Theme.otherName)
                .bold(person.talking)
            line = Text("\(line)\(name)")
        }
        if state.more > 0 {
            line = Text("\(line)   \(Text(verbatim: "+\(state.more)").foregroundStyle(Theme.grayText))")
        }
        return line
    }
}

/// Mute and Deafen, or (after the ×) "Leave Lounge?" with Leave and Stay.
private struct CallButtonsRow: View {
    var state: CallState

    var body: some View {
        HStack(spacing: 6) {
            if state.askingToLeave {
                Image(systemName: "questionmark.circle.fill")
                    .foregroundStyle(Theme.accent)
                Text("Leave \(state.channel)?")
                    .font(.footnote.bold())
                    .foregroundStyle(Theme.text)
                    .lineLimit(1)
                Spacer(minLength: 4)
                Button(intent: CallButtonIntent(.leave)) {
                    LiveButtonFace(title: "Leave", systemImage: "phone.down.fill", tint: Theme.danger)
                }
                .accessibilityLabel("Leave voice")
                Button(intent: CallButtonIntent(.stay)) {
                    LiveButtonFace(title: "Stay", isDefault: true)
                }
            } else {
                Button(intent: CallButtonIntent(.mute)) {
                    LiveButtonFace(title: state.quiet ? "Unmute" : "Mute",
                                   systemImage: state.quiet ? "mic.slash.fill" : "mic.fill",
                                   tint: state.quiet ? Theme.danger : nil, on: state.quiet)
                }
                Button(intent: CallButtonIntent(.deafen)) {
                    LiveButtonFace(title: state.deafened ? "Undeafen" : "Deafen",
                                   systemImage: state.deafened ? "speaker.slash.fill" : "headphones",
                                   tint: state.deafened ? Theme.danger : nil, on: state.deafened)
                }
                Spacer(minLength: 4)
                Text("\(state.count) in call")
                    .font(.caption)
                    .foregroundStyle(Theme.hint)
                    .lineLimit(1)
            }
        }
        .buttonStyle(.plain)
    }
}

/// A Win98 push button's face; `on` = pushed in (a toggle that's on), like
/// the app's `Win98ToggleButtonStyle`. `isDefault` adds the default button's outline.
private struct LiveButtonFace: View {
    var title: String
    var systemImage: String? = nil
    var tint: Color? = nil
    var on = false
    var isDefault = false

    var body: some View {
        HStack(spacing: 4) {
            if let systemImage {
                Image(systemName: systemImage)
                    .foregroundStyle(tint ?? Theme.text)
            }
            Text(title)
                .foregroundStyle(Theme.text)
        }
        .font(.footnote)
        .lineLimit(1)
        .padding(.horizontal, 10)
        .frame(minWidth: 64, minHeight: 28)
        .offset(x: on ? 1 : 0, y: on ? 1 : 0)
        .background(on ? Theme.light : Theme.face)
        .overlay(LiveBevel(style: on ? .pressed : .raised))
        .padding(isDefault ? 1 : 0)
        .overlay(isDefault ? Rectangle().stroke(Theme.dkShadow, lineWidth: 1) : nil)
        .contentShape(.rect)
    }
}

/// The app's two-ring bevel (`Bevel` in Win98.swift), in plain shapes.
private struct LiveBevel: View {
    enum Style { case raised, pressed, sunken }
    var style: Style

    var body: some View {
        let (outerTopLeft, outerBottomRight, innerTopLeft, innerBottomRight): (Color, Color, Color, Color) = switch style {
        case .raised: (Theme.hilight, Theme.dkShadow, Theme.light, Theme.shadow)
        case .pressed: (Theme.dkShadow, Theme.hilight, Theme.shadow, Theme.light)
        case .sunken: (Theme.shadow, Theme.hilight, Theme.dkShadow, Theme.light)
        }
        ZStack {
            BevelHalf(inset: 0, topLeft: true).fill(outerTopLeft)
            BevelHalf(inset: 0, topLeft: false).fill(outerBottomRight)
            BevelHalf(inset: 1, topLeft: true).fill(innerTopLeft)
            BevelHalf(inset: 1, topLeft: false).fill(innerBottomRight)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// One 1 pt ring's top-left or bottom-right half, `inset` from the edge
/// (the same rects `Bevel` fills).
private nonisolated struct BevelHalf: Shape {
    var inset: CGFloat
    var topLeft: Bool

    func path(in rect: CGRect) -> Path {
        let ring = rect.insetBy(dx: inset, dy: inset)
        var path = Path()
        guard ring.width > 2, ring.height > 2 else { return path }
        if topLeft {
            path.addRect(CGRect(x: ring.minX, y: ring.minY, width: ring.width - 1, height: 1))
            path.addRect(CGRect(x: ring.minX, y: ring.minY, width: 1, height: ring.height - 1))
        } else {
            path.addRect(CGRect(x: ring.minX, y: ring.maxY - 1, width: ring.width, height: 1))
            path.addRect(CGRect(x: ring.maxX - 1, y: ring.minY, width: 1, height: ring.height))
        }
        return path
    }
}

private extension View {
    /// A white sunken well (the app's `well()`).
    func liveWell() -> some View {
        padding(2).background(Theme.window).overlay(LiveBevel(style: .sunken))
    }

    /// Settings › Look's scheme, or the system's.
    func callScheme(_ dark: Bool?) -> some View {
        modifier(CallScheme(dark: dark))
    }
}

private struct CallScheme: ViewModifier {
    var dark: Bool?
    @Environment(\.colorScheme) private var system

    func body(content: Content) -> some View {
        content.environment(\.colorScheme, dark.map { $0 ? .dark : .light } ?? system)
    }
}
#endif
