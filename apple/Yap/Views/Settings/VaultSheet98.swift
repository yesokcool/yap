import SwiftUI
import YapCore

/// The encrypted Personal space's passcode window, Win98-style: turn it on,
/// unlock, change the passcode, or start over.
struct VaultSheet98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var form: VaultForm
    @FocusState private var focused: Bool

    init(mode: VaultMode) {
        _form = State(initialValue: VaultForm(mode: mode))
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: form.mode == .reset ? "exclamationmark.triangle.fill" : "lock.fill")
                            .font(.system(size: 22))
                            .foregroundStyle(form.mode == .reset ? Theme.danger : Theme.text)
                        Text(form.mode.text)
                            .font(Theme.chrome)
                            .foregroundStyle(Theme.text)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if form.mode == .change {
                        Text("Current passcode:").font(Theme.chrome).foregroundStyle(Theme.text)
                        Field98 { SecureField("", text: $form.old).textContentType(.password) }
                    }
                    Text(form.mode == .change ? "New passcode:" : "Passcode:").font(Theme.chrome).foregroundStyle(Theme.text)
                    Field98 {
                        SecureField("", text: $form.pass)
                            .textContentType(form.mode == .unlock ? .password : .newPassword)
                            .focused($focused)
                            .onSubmit(submit)
                    }
                    if form.mode.asksAgain {
                        Text("Again:").font(Theme.chrome).foregroundStyle(Theme.text)
                        Field98 { SecureField("", text: $form.again).textContentType(.newPassword).onSubmit(submit) }
                    }
                    if let error = form.error {
                        ErrorLine(text: error)
                    }
                    HStack(spacing: 6) {
                        if form.mode == .unlock {
                            Button("Forgot it? Start over…") {
                                form.mode = .reset
                                form.error = nil
                            }
                            .buttonStyle(.win98)
                        }
                        Spacer()
                        if form.working { ProgressView().controlSize(.small) }
                        Button(form.working ? "Working…" : form.mode.action, action: submit)
                            .buttonStyle(.win98Default)
                            .disabled(!form.canSubmit)
                    }
                    .padding(.top, 4)
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet(form.mode.title) {
                CloseSheetButton { dismiss() }
            }
            .onAppear { focused = true }
        }
        .presentationDetents([.medium, .large])
        .interactiveDismissDisabled(form.working)
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 320)
        #endif
    }

    private func submit() {
        guard form.canSubmit else { return }
        Task {
            if await form.submit(model.client) { dismiss() }
        }
    }
}

/// Settings › Profile: the encrypted Personal space.
struct PersonalSpaceGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let client = model.client
        GroupBox98(title: "Personal space") {
            Text(client.vaultWrap == nil
                 ? "Encrypt your Personal space with a passcode only you know. Yap's server then only sees scrambled notes."
                 : client.vaultUnlocked ? "Encrypted, and unlocked on this device." : "Encrypted, and locked on this device.")
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 6) {
                if client.vaultWrap == nil {
                    Button("Encrypt it…") { model.vaultSheet = .setup }.buttonStyle(.win98)
                } else {
                    if client.vaultUnlocked {
                        Button("Change passcode…") { model.vaultSheet = .change }.buttonStyle(.win98)
                        Button("Lock on this device") { client.lockVault() }.buttonStyle(.win98)
                    } else {
                        Button("Unlock…") { model.vaultSheet = .unlock }.buttonStyle(.win98)
                    }
                }
            }
        }
    }
}
