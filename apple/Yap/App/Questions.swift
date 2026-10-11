import SwiftUI

/// A question Yap asks — "Delete this message?", "Ban Ada?", the ping
/// note, "Sign out of Yap on this device?" — in a Windows 98 message box
/// (the web's `confirmSheet`, js/sheet.js: a question or warning icon, the
/// action and Cancel centered). Something to hand on (`value`, the
/// temporary PIN) gets the value to copy in the box.
struct Question {
    /// A line of text to fill in (the ping note).
    struct Field {
        /// Above the field.
        var label: String?
        var placeholder = ""
        var maxLength: Int?
    }

    /// What's being asked; Win98 puts it on the title bar.
    var title: String
    /// One or two calm sentences about what happens.
    var text: String?
    /// The action's label ("Delete").
    var action = "OK"
    /// Destructive: drawn as such, and Cancel is the default button.
    var danger = false
    /// nil = no Cancel (just telling you something).
    var cancel: String? = "Cancel"
    var field: Field?
    /// Something to pass on (a temporary PIN): shown on its own, selectable,
    /// with a Copy button that doesn't close the box.
    var value: String?
    var copyLabel = "Copy"
    /// The answer: the field's text ("" without one).
    var perform: (String) -> Void = { _ in }
    var onCancel: () -> Void = {}
}

/// The web's words for each question (`confirmSheet` calls in web/js).
extension Question {
    static func deleteMessage(perform: @escaping () -> Void) -> Question {
        Question(title: "Delete this message?", text: "It’s erased for good, for everyone.", action: "Delete", danger: true,
                 perform: { _ in perform() })
    }

    static func archive(_ name: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Archive \(name)?", text: "It disappears for everyone. Its messages are kept, and the owner can bring it back.",
                 action: "Archive", danger: true, perform: { _ in perform() })
    }

    static func removePlace(_ name: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Remove \(name)?", text: "It goes from Places for everyone.", action: "Remove", danger: true,
                 perform: { _ in perform() })
    }

    static func turnOffInvite(_ label: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Turn off “\(label)”?", text: "Nobody new can join with this link. People already in stay in.",
                 action: "Turn off", danger: true, perform: { _ in perform() })
    }

    static func resetPin(_ name: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Reset \(name)’s PIN?", text: "They get a temporary PIN to sign in with; their old PIN or password stops working.",
                 action: "Reset PIN", danger: true, perform: { _ in perform() })
    }

    static func ban(_ name: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Ban \(name)?", text: "They’re signed out and can’t come back. Their messages stay (mods can delete them).",
                 action: "Ban", danger: true, perform: { _ in perform() })
    }

    static func deleteSound(_ name: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Delete “\(name)”?", text: "It goes from the soundboard for everyone.", action: "Delete", danger: true,
                 perform: { _ in perform() })
    }

    static func cancelHang(_ title: String, perform: @escaping () -> Void) -> Question {
        Question(title: "Cancel “\(title)”?", text: "Everyone sees it’s off.", action: "Cancel it", danger: true, cancel: "Keep it",
                 perform: { _ in perform() })
    }
}

extension View {
    /// Ask `question` while `isPresented` is true, in a Windows 98 message box.
    func question(isPresented: Binding<Bool>, _ question: Question) -> some View {
        modifier(QuestionModifier(isPresented: isPresented, question: question))
    }
}

struct QuestionModifier: ViewModifier {
    @Binding var isPresented: Bool
    var question: Question
    @State private var text = ""
    /// iPhone: the box's cover, switched without the slide-up.
    @State private var shown = false

    /// A Windows 98 message box, over a dimmed screen (iPhone) or as the
    /// window's sheet (Mac).
    func body(content: Content) -> some View {
        #if os(iOS)
        content
            .onChange(of: isPresented, initial: true) { _, on in
                // Win98 is instant: no slide-up.
                var instant = Transaction()
                instant.disablesAnimations = true
                withTransaction(instant) { shown = on }
            }
            .fullScreenCover(isPresented: Binding(get: { shown }, set: { if !$0 { close() } })) {
                ZStack {
                    Color.black.opacity(0.3).ignoresSafeArea()
                        .accessibilityHidden(true)
                    box
                        .padding(16)
                }
                .presentationBackground(.clear)
            }
        #else
        content.sheet(isPresented: $isPresented) {
            box.fixedSize(horizontal: false, vertical: true)
        }
        #endif
    }

    private var box: some View {
        QuestionBox98(question: question, text: $text, answer: answer)
    }

    /// A button was pressed: close, then answer (the question carries what
    /// it's about, so clearing the binding first loses nothing).
    private func answer(_ yes: Bool) {
        let value = trimmed(text)
        text = ""
        close()
        if yes { question.perform(value) } else { question.onCancel() }
    }

    private func close() {
        var instant = Transaction()
        instant.disablesAnimations = true
        withTransaction(instant) {
            shown = false
            isPresented = false
        }
    }

    private func trimmed(_ value: String) -> String {
        let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let limit = question.field?.maxLength else { return clean }
        return String(clean.prefix(limit))
    }
}
