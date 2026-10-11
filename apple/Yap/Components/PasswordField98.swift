import SwiftUI
import YapCore

/// A password in a Win98 sunken field: dots, or the text itself while
/// `shown` (the form's "Show password" box). Password managers fill it:
/// `isNew` asks for a new one (`.newPassword`, which can suggest one),
/// otherwise the saved one (`.password`). Capped at the server's 128
/// characters; anything else (an old 6-digit PIN too) is the server's to judge.
struct PasswordField98: View {
    @Binding var text: String
    var isNew = false
    var shown = false

    var body: some View {
        Field98 {
            Group {
                if shown {
                    TextField("", text: $text)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                } else {
                    SecureField("", text: $text)
                }
            }
            .textContentType(isNew ? .newPassword : .password)
            .onChange(of: text) { _, new in
                if new.count > Secret.maximum { text = String(new.prefix(Secret.maximum)) }
            }
        }
    }
}

/// The note under a new password's field (the web's words).
struct PasswordRule98: View {
    var body: some View {
        Text(PasswordText.rule)
            .font(Theme.small)
            .foregroundStyle(Theme.grayText)
            .fixedSize(horizontal: false, vertical: true)
    }
}
