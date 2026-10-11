import SwiftUI
import YapCore
import YapVoice

/// The web's wording for the quiet notes next to people in a call
/// (web/js/app.js › voiceNote, PROTOCOL.md › Voice › Connection hints) and
/// for the call's own warnings, wherever the app shows them.
enum VoiceHintText {
    /// How a note is coloured: like the web's `.vq`, `.vq.away`, `.vq.blocked`.
    enum Kind {
        case weak, away, blocked
    }

    static func label(_ hint: ConnectionHint) -> String {
        switch hint {
        case .reconnecting: "reconnecting…"
        case .needsRelay: "needs the voice relay"
        case .cantConnect: "can't connect"
        case .weak: "weak connection"
        }
    }

    static func kind(_ hint: ConnectionHint) -> Kind {
        switch hint {
        case .reconnecting: .away
        case .needsRelay, .cantConnect: .blocked
        case .weak: .weak
        }
    }

    /// The longer explanation: the Mac's tooltip, VoiceOver's hint.
    static func detail(_ hint: ConnectionHint, name: String) -> String {
        switch hint {
        case .reconnecting:
            "\(name) lost their connection to Yap. Their voice is still coming through; it usually sorts itself out in a few seconds."
        case .cantConnect:
            "Your network and \(name)'s can't reach each other right now. Yap keeps trying."
        case .needsRelay:
            "Your network and \(name)'s can't reach each other directly (strict firewalls or mobile networks do this). The server's owner can switch on the voice relay, then it just works — see SELF_HOSTING.md."
        case let .weak(concealed, rttMs):
            "\(name)'s voice may break up: \(weakReason(concealed: concealed, rttMs: rttMs)). It usually passes; if not, one of you is on a busy network."
        }
    }

    private static func weakReason(concealed: Double, rttMs: Double?) -> String {
        let percent = Int((concealed * 100).rounded())
        var why: [String] = []
        if percent >= 3 { why.append("about \(percent)% of their audio is getting lost or arriving late") }
        if let rttMs, rttMs > LinkQuality.rttLimitMs { why.append("the delay is \(Int(rttMs.rounded())) ms") }
        return why.isEmpty ? "their audio is arriving unevenly" : why.joined(separator: ", and ")
    }

    /// The call's status while our socket is down and the call goes on.
    static let resuming = "Reconnecting… voice is still on"

    static let silence = "Your mic is sending silence"
    #if os(iOS)
    static let silenceDetail = "Nothing at all is coming from your microphone. If a phone call or another app has it, it comes back when that's done."
    #else
    static let silenceDetail = "Nothing at all is coming from your microphone. Check its mute switch, or pick another one in System Settings › Sound."
    #endif
}

extension View {
    /// A hint's explanation: a tooltip on the Mac, a VoiceOver hint everywhere.
    func voiceHintHelp(_ hint: ConnectionHint?, name: String) -> some View {
        let detail = hint.map { VoiceHintText.detail($0, name: name) } ?? ""
        return help(detail).accessibilityHint(detail)
    }
}
