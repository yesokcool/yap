import SwiftUI

// Sheet actions (README › Design): an X to
// close that asks "Discard changes?" when there are edits, and a ✓ to
// confirm. Drawn as Windows 98 buttons; the ✓ is the default button (the
// extra black outline). No "Done"/"Cancel" text buttons.

/// The ✓: the default Win98 button, "Done" for VoiceOver.
struct DoneButton: View {
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "checkmark")
                .font(.system(size: 13, weight: .bold))
                .frame(minWidth: 18)
        }
        .buttonStyle(Win98ButtonStyle(isDefault: true, compact: true))
        #if os(macOS)
        // The default button: Return presses it (Win98's thick outline means just that).
        .keyboardShortcut(.defaultAction)
        #endif
        .accessibilityLabel("Done")
        .appScheme()
    }
}

/// The X. With `hasChanges`, a tap asks "Discard changes?" from the button
/// itself instead of closing. Pair with `.interactiveDismissDisabled(hasChanges)`.
struct CloseSheetButton: View {
    var hasChanges: Bool = false
    var title: String = "Discard changes?"
    var message: String? = nil
    var action: () -> Void

    @State private var confirming = false
    @Environment(\.inTitleBar) private var inTitleBar

    var body: some View {
        Group {
            if inTitleBar {
                // A Mac sheet's title bar close box.
                Button {
                    if hasChanges { confirming = true } else { action() }
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(TitleBarButtonStyle())
                .keyboardShortcut(.cancelAction)
                .help("Close")
            } else {
                Button {
                    if hasChanges { confirming = true } else { action() }
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .black))
                        .frame(minWidth: 18)
                }
                .buttonStyle(.win98Compact)
            }
        }
        .accessibilityLabel("Cancel")
        .appScheme()
        .question(isPresented: $confirming, Question(title: title, text: message, action: "Discard", danger: true, perform: { _ in action() }))
    }
}

extension ToolbarContent {
    /// Toolbar items draw their own Win98 buttons, not the glass bubble.
    func win98Item() -> some ToolbarContent {
        sharedBackgroundVisibility(.hidden)
    }
}
