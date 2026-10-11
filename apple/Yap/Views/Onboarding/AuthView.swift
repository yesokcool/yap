import SwiftUI
import YapCore

/// The AIM-style "Sign On" window: "I'm new" (name + password + color →
/// signup, then the waiting room) or "I have an account" (name + password;
/// an old 6-digit PIN still signs in, so that field takes anything and the
/// server judges it). A new password is 8 characters or more.
struct AuthView: View {
    @Environment(AppModel.self) private var model
    @State private var mode: YapClient.AuthMode = .login
    @State private var name = ""
    @State private var pin = ""
    @State private var showPassword = false
    @State private var color = Theme.personColors[0]
    @State private var localError: String?
    @State private var initialized = false
    @FocusState private var field: Field?

    enum Field { case name, pin }

    var body: some View {
        DesktopDialog(title: "Sign On", systemImage: "person.fill") {
            WelcomeMark()

            VStack(alignment: .leading, spacing: 0) {
                Radio98(title: AuthText.new, isOn: mode == .signup) { mode = .signup }
                Radio98(title: AuthText.returning, isOn: mode == .login) { mode = .login }
            }

            labeled("Screen name:") {
                Field98 {
                    TextField("", text: $name)
                        .textContentType(mode == .signup ? .nickname : .username)
                        .autocorrectionDisabled()
                        .focused($field, equals: .name)
                        .submitLabel(.next)
                        .onSubmit { field = .pin }
                }
            }
            labeled("\(PasswordText.field):") {
                PasswordField98(text: $pin, isNew: mode == .signup, shown: showPassword)
                    .focused($field, equals: .pin)
                    .onSubmit(submit)
                if mode == .signup {
                    PasswordRule98()
                }
                Check98(title: PasswordText.show, isOn: $showPassword)
            }
            if mode == .signup {
                labeled("Color:") {
                    ColorSwatches(selection: $color)
                }
            }

            Text(AuthText.hint(signup: mode == .signup))
                .font(Theme.chrome)
                .foregroundStyle(Theme.grayText)
                .fixedSize(horizontal: false, vertical: true)

            if let message = localError ?? model.client.authError?.message {
                ErrorLine(text: message)
                if localError == nil, let help = AuthText.help(for: model.client.authError) {
                    Text(help).font(Theme.chrome).foregroundStyle(Theme.text)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if model.client.authError?.code == "name_taken", mode == .signup {
                    Button("Sign in instead") { mode = .login }
                        .buttonStyle(.win98)
                }
            }

            Separator98()
            HStack(spacing: 8) {
                Button("Other invite…") { model.client.forgetServer() }
                    .buttonStyle(.win98)
                Spacer()
                Button(model.client.phase == .connecting ? "Signing on…" : (mode == .signup ? "Sign Up" : "Sign On"), action: submit)
                    .buttonStyle(.win98Default)
                    .disabled(!canSubmit)
            }
        }
        .onAppear {
            guard !initialized else { return }
            initialized = true
            name = model.client.savedName ?? ""
            if let saved = model.client.savedColor, Theme.personColors.contains(saved) {
                color = saved
            } else {
                color = Theme.personColors.randomElement()!
            }
            mode = model.client.authError?.mode ?? (model.client.savedName == nil ? .signup : .login)
        }
        .onChange(of: mode) { localError = nil }
        .onChange(of: showPassword) {
            // The field is swapped for its shown/hidden twin: keep typing in it.
            if field == .pin { Task { field = .pin } }
        }
    }

    private func labeled<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(Theme.chrome).foregroundStyle(Theme.text)
            content()
        }
    }

    /// Signing up needs a new password's 8 characters; signing in takes
    /// whatever was set (an old PIN too).
    private var secretFits: Bool {
        mode == .signup ? Secret.isPassword(pin) : Secret.canSignIn(pin)
    }

    private var canSubmit: Bool {
        !name.isBlank && secretFits && model.client.phase != .connecting
    }

    private func submit() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        guard secretFits else {
            localError = mode == .signup ? "Use a password of at least 8 characters." : "Type your password."
            return
        }
        localError = nil
        field = nil
        if mode == .signup {
            model.client.signUp(name: trimmed, pin: pin, color: color)
        } else {
            model.client.logIn(name: trimmed, pin: pin)
        }
    }
}

/// Square color chips (for your buddy icon).
struct ColorSwatches: View {
    @Binding var selection: String

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Theme.personColors, id: \.self) { hex in
                Button {
                    selection = hex
                } label: {
                    Theme.color(hex: hex)
                        .frame(width: 20, height: 20)
                        .overlay(Bevel(style: .sunken))
                        .padding(2)
                        .overlay(selection == hex ? Rectangle().stroke(Theme.text, style: StrokeStyle(lineWidth: 1, dash: [1, 1])) : nil)
                        .padding(2)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(ColorName.of(hex: hex))
                .accessibilityAddTraits(selection == hex ? .isSelected : [])
            }
        }
        .sensoryFeedback(.selection, trigger: selection)
    }
}
