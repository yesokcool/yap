import SwiftUI
import YapCore

/// A Windows 98 dialog centered on the teal desktop (first-run screens).
struct DesktopDialog<Content: View>: View {
    var title: String
    var systemImage: String? = nil
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            Window98(title: title, systemImage: systemImage) {
                VStack(alignment: .leading, spacing: 12) {
                    content
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxWidth: 400)
            .padding(.horizontal, 16)
            .padding(.vertical, 40)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Theme.desktop.ignoresSafeArea())
    }
}

/// The Yap logo panel at the top of the sign-on window, like AIM's.
struct WelcomeMark: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "bubble.left.and.bubble.right.fill")
                .font(.system(size: 30))
                .foregroundStyle(Theme.titleStart, Theme.titleEnd)
            VStack(alignment: .leading, spacing: 0) {
                Text("Yap").font(.system(size: 28, weight: .heavy))
                Text("Text and voice for friends").font(Theme.chrome).foregroundStyle(Theme.grayText)
            }
        }
        .foregroundStyle(Theme.text)
        .padding(12)
        .frame(maxWidth: .infinity)
        .well()
        .accessibilityElement(children: .combine)
    }
}

/// First run: paste the invite link a friend sent.
struct InviteView: View {
    @Environment(AppModel.self) private var model
    @State private var link = ""
    /// The system Paste button shows only when the clipboard has text or a link.
    @State private var canPaste = false
    @State private var checking = false
    @State private var error: String?

    var body: some View {
        DesktopDialog(title: "Welcome to Yap", systemImage: "bubble.left.and.bubble.right.fill") {
            WelcomeMark()
            Text("Yap is invite-only. Paste the link a friend sent you — it looks like chat.example.com/#key=…")
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 4) {
                Text("Invite link:").font(Theme.chrome).foregroundStyle(Theme.text)
                Field98 {
                    TextField("", text: $link, prompt: Text("https://chat.example.com/#key=…").foregroundStyle(Theme.grayText), axis: .vertical)
                        .lineLimit(1...3)
                        .textContentType(.URL)
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                        #endif
                        .onSubmit(check)
                        .clipboardHas($canPaste)
                }
            }
            if let message = error ?? model.client.authError?.message {
                ErrorLine(text: message)
            }
            HStack(spacing: 8) {
                Spacer()
                if canPaste {
                    PasteButton(payloadType: String.self) { strings in
                        if let first = strings.first { link = first }
                    }
                    .labelStyle(.titleOnly)
                    .buttonBorderShape(.roundedRectangle(radius: 0))
                    .tint(Theme.face)
                }
                Button(checking ? "Checking…" : "OK", action: check)
                    .buttonStyle(.win98Default)
                    .disabled(link.isBlank || checking)
            }
        }
    }

    private func check() {
        guard let endpoint = ServerEndpoint.parse(invite: link) else {
            error = "That doesn't look like a Yap invite link."
            return
        }
        error = nil
        checking = true
        Task {
            defer { checking = false }
            do {
                let result = try await ServerAPI(endpoint: endpoint).check()
                if result.ok {
                    model.client.useInvite(endpoint)
                    model.refreshConfig()
                } else {
                    error = "That invite key didn't work. Ask for a fresh link."
                }
            } catch {
                self.error = "Couldn't reach that server. Check the link and your connection."
            }
        }
    }
}

/// A red-icon error line, like a Win98 message box's.
struct ErrorLine: View {
    var text: String

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: "xmark.octagon.fill").foregroundStyle(Theme.danger)
            Text(text).font(Theme.chrome).foregroundStyle(Theme.text).fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .combine)
    }
}
