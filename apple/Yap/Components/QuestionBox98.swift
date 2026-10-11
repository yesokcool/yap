import SwiftUI

/// A Windows 98 message box for a `Question`: the question on the title bar
/// (its close box cancels), a question or warning icon beside the words,
/// an optional field, then the action and Cancel centered. The action is
/// the default button, unless it's destructive: then Cancel is.
struct QuestionBox98: View {
    var question: Question
    @Binding var text: String
    var answer: (Bool) -> Void
    @FocusState private var typing: Bool
    @State private var copied = false

    var body: some View {
        let cancels = question.cancel != nil
        VStack(spacing: 0) {
            TitleBar(title: question.title) {
                Button { answer(!cancels) } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(TitleBarButtonStyle())
                .accessibilityLabel(cancels ? "Cancel" : question.action)
            }
            VStack(spacing: 14) {
                HStack(alignment: .top, spacing: 12) {
                    MessageBoxIcon98(warning: question.danger, info: !question.danger && question.cancel == nil)
                    VStack(alignment: .leading, spacing: 8) {
                        Text(question.text ?? question.title)
                            .font(Theme.chat)
                            .foregroundStyle(Theme.text)
                            .fixedSize(horizontal: false, vertical: true)
                        if let value = question.value {
                            // Something to pass on: in a sunken box to select, with Copy beside
                            // it, or under it when the whole value wouldn't fit beside (a
                            // temporary password is longer than the old PIN was): never cut short.
                            ViewThatFits(in: .horizontal) {
                                HStack(spacing: 6) {
                                    valueBox(value)
                                    copyButton(value)
                                }
                                VStack(alignment: .trailing, spacing: 6) {
                                    valueBox(value)
                                    copyButton(value)
                                }
                            }
                        }
                        if let field = question.field {
                            if let label = field.label {
                                Text(label).font(Theme.chrome).foregroundStyle(Theme.text)
                            }
                            Field98 {
                                TextField("", text: $text, prompt: Text(field.placeholder).foregroundStyle(Theme.grayText))
                                    .focused($typing)
                                    .onSubmit { answer(true) }
                                    .onChange(of: text) { _, new in
                                        if let limit = field.maxLength, new.count > limit { text = String(new.prefix(limit)) }
                                    }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
                }
                HStack(spacing: 6) {
                    Button(question.action) { answer(true) }
                        .buttonStyle(Win98ButtonStyle(isDefault: !question.danger || !cancels))
                        .modifier(DefaultKey(active: !question.danger || !cancels || question.field != nil))
                    if let cancel = question.cancel {
                        Button(cancel) { answer(false) }
                            .buttonStyle(Win98ButtonStyle(isDefault: question.danger))
                            .keyboardShortcut(.cancelAction)
                    }
                }
                .frame(maxWidth: .infinity)
            }
            .padding(.horizontal, 12)
            .padding(.top, 14)
            .padding(.bottom, 12)
        }
        .padding(3)
        .background(Theme.face)
        .overlay(Bevel(style: .raised))
        .frame(maxWidth: 380)
        .onAppear { if question.field != nil { typing = true } }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
    }

    private func valueBox(_ value: String) -> some View {
        Text(value)
            .font(.system(size: 17, weight: .bold, design: .monospaced))
            .foregroundStyle(Theme.text)
            .textSelection(.enabled)
            .speechSpellsOutCharacters()
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.window)
            .overlay(Bevel(style: .sunken))
    }

    private func copyButton(_ value: String) -> some View {
        Button(copied ? "Copied" : question.copyLabel) {
            copyToPasteboard(value)
            copied = true
        }
        .buttonStyle(.win98)
        .fixedSize()
    }

    /// Return presses the default button (the Mac; iPad keyboards).
    private struct DefaultKey: ViewModifier {
        var active: Bool

        func body(content: Content) -> some View {
            if active { content.keyboardShortcut(.defaultAction) } else { content }
        }
    }
}

/// Windows 98's message-box icons, drawn crisp: a white speech bubble with
/// a navy "?" (a question) or a blue "i" (just telling you something), or a
/// yellow triangle with a "!" (a warning).
struct MessageBoxIcon98: View {
    var warning = false
    var info = false

    var body: some View {
        Canvas { context, size in
            let s = min(size.width, size.height) / 32
            if warning {
                var triangle = Path()
                triangle.move(to: CGPoint(x: 16 * s, y: 2 * s))
                triangle.addLine(to: CGPoint(x: 31 * s, y: 29 * s))
                triangle.addLine(to: CGPoint(x: 1 * s, y: 29 * s))
                triangle.closeSubpath()
                context.fill(triangle, with: .color(Color(red: 1, green: 1, blue: 0)))
                context.stroke(triangle, with: .color(.black), style: StrokeStyle(lineWidth: 1.5 * s, lineJoin: .round))
                context.fill(Path(CGRect(x: 14.5 * s, y: 10 * s, width: 3 * s, height: 11 * s)), with: .color(.black))
                context.fill(Path(CGRect(x: 14.5 * s, y: 23 * s, width: 3 * s, height: 3 * s)), with: .color(.black))
            } else {
                let bubble = Path(ellipseIn: CGRect(x: 2 * s, y: 2 * s, width: 28 * s, height: 24 * s))
                var tail = Path()
                tail.move(to: CGPoint(x: 9 * s, y: 23 * s))
                tail.addLine(to: CGPoint(x: 6 * s, y: 30 * s))
                tail.addLine(to: CGPoint(x: 15 * s, y: 25 * s))
                tail.closeSubpath()
                for shape in [tail, bubble] {
                    context.fill(shape, with: .color(.white))
                    context.stroke(shape, with: .color(.black), lineWidth: 1.5 * s)
                }
                // The bubble covers the tail's top edge.
                context.fill(Path(ellipseIn: CGRect(x: 2.75 * s, y: 2.75 * s, width: 26.5 * s, height: 22.5 * s)), with: .color(.white))
                let mark = Text(info ? "i" : "?").font(.system(size: 19 * s, weight: .bold, design: .serif))
                    .foregroundStyle(info ? Color(red: 0, green: 0, blue: 1) : Color(red: 0, green: 0, blue: 0.5))
                context.draw(mark, at: CGPoint(x: 16 * s, y: 14 * s))
            }
        }
        .frame(width: 32, height: 32)
        .accessibilityHidden(true)
    }
}
