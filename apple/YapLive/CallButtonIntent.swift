#if os(iOS)
import AppIntents

/// The call Live Activity's buttons.
nonisolated enum CallButton: String, Sendable {
    case mute, deafen, askToLeave, leave, stay
}

/// A button on the call's Live Activity. A `LiveActivityIntent` performs in
/// the app's process, where the call is, so this file is in both targets:
/// YapLive names it in `Button(intent:)`, the app runs it. (A type only in
/// the extension would do nothing on a device.)
struct CallButtonIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Voice call button"
    static let isDiscoverable = false

    @Parameter(title: "Button") var button: String

    init() {}

    init(_ button: CallButton) {
        self.button = button.rawValue
    }

    func perform() async throws -> some IntentResult {
        if let button = CallButton(rawValue: button) {
            await MainActor.run { CallButtons.press?(button) }
        }
        return .result()
    }
}

/// Where the app picks up a press (`LiveCall` sets it). Never set in the
/// extension, whose copy of the intent never runs.
@MainActor
enum CallButtons {
    static var press: ((CallButton) -> Void)?
}
#endif
