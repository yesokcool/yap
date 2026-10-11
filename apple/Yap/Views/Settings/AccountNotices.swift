import SwiftUI
import YapCore

/// Account notes above your nameplate, Win98-style (tooltip-yellow bars):
/// a sign-in on a new device, and "Choose a password" once per device
/// (`ready.weakPin`: an old PIN, a reset's temporary password or a guessable one).
struct AccountNotices98: View {
    @Environment(AppModel.self) private var model
    @State private var signingOut = false

    var body: some View {
        let client = model.client
        VStack(spacing: 2) {
            if let device = client.signinAlert {
                bar(icon: "exclamationmark.shield.fill") {
                    Text("\(Text("New sign-in").bold()) to your account: \(device.label), just now. Not you?")
                } actions: {
                    FlowLayout(spacing: 4) {
                        Button("Sign out everywhere else…") { signingOut = true }
                            .buttonStyle(.win98Compact)
                        Button("It was me") { client.signinAlert = nil }
                            .buttonStyle(.win98Compact)
                    }
                } dismiss: { client.signinAlert = nil }
            }
            if let nudge = model.passwordPrompt {
                bar(icon: "key.fill") {
                    Text("\(Text(PasswordText.promptTitle).bold())\n\(PasswordText.prompt(nudge))")
                } actions: {
                    FlowLayout(spacing: 4) {
                        Button(PasswordText.choose) { model.changingPassword = true }
                            .buttonStyle(Win98ButtonStyle(isDefault: true, compact: true))
                        Button(PasswordText.later) { model.passwordPromptsPutOff.insert(nudge.rawValue) }
                            .buttonStyle(.win98Compact)
                    }
                }
            }
        }
        .signOutOthersPrompt(isPresented: $signingOut)
    }

    /// `dismiss` adds a close box (the prompt has Later instead).
    private func bar<Message: View, Actions: View>(icon: String, @ViewBuilder message: () -> Message,
                                                   @ViewBuilder actions: () -> Actions, dismiss: (() -> Void)? = nil) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .top, spacing: 6) {
                Image(systemName: icon).font(.system(size: 11, weight: .bold))
                message().font(Theme.chrome).fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 4)
                if let dismiss {
                    Button(action: dismiss) {
                        Image(systemName: "xmark").font(.system(size: 9, weight: .black))
                    }
                    .buttonStyle(.win98Compact)
                    .accessibilityLabel("Dismiss")
                }
            }
            actions().padding(.leading, 17)
        }
        .foregroundStyle(Theme.tooltipText)
        .padding(6)
        .background(Theme.tooltip)
        .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
    }
}

/// Settings › Account: every device signed in as you, with Sign Out.
struct DevicesGroup98: View {
    @Environment(AppModel.self) private var model
    @State private var showingAll = false
    @State private var signingOut = false

    var body: some View {
        let client = model.client
        VStack(alignment: .leading, spacing: 6) {
            Text("Signed in on:").font(Theme.chrome).foregroundStyle(Theme.text)
            VStack(alignment: .leading, spacing: 0) {
                if let devices = client.devices {
                    ForEach(DeviceText.shown(devices, all: showingAll)) { device in
                        HStack(spacing: 6) {
                            Image(systemName: DeviceText.symbol(device)).font(.system(size: 11)).frame(width: 16)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(device.label).font(Theme.chatBold)
                                Text(DeviceText.detail(device)).font(Theme.small).foregroundStyle(Theme.grayText)
                            }
                            Spacer(minLength: 4)
                            if !device.current {
                                Button("Sign out") { client.revokeDevice(device.id) }
                                    .buttonStyle(.win98Compact)
                            }
                        }
                        .foregroundStyle(Theme.text)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 4)
                    }
                } else {
                    ProgressView().padding(6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .well()
            if let count = client.devices?.count, count > DeviceText.shownAtFirst, !showingAll {
                Button("Show all \(count)") { showingAll = true }
                    .buttonStyle(.win98)
            }
            if (client.devices?.count ?? 0) > 1 {
                Button("Sign out everywhere else…") { signingOut = true }
                    .buttonStyle(.win98)
            }
        }
        .signOutOthersPrompt(isPresented: $signingOut)
        .onChange(of: client.mySid, initial: true) { _, sid in
            if sid != nil { client.refreshDevices() }
        }
    }
}

enum DeviceText {
    /// Settings lists this many devices until "Show all".
    static let shownAtFirst = 5

    /// This device first, then the rest (most recently used first, as the
    /// server sends them); the first few unless `all`.
    static func shown(_ devices: [Device], all: Bool) -> [Device] {
        let ordered = devices.filter(\.current) + devices.filter { !$0.current }
        return all ? ordered : Array(ordered.prefix(shownAtFirst))
    }

    static func symbol(_ device: Device) -> String {
        if device.isListener { return "tv" }
        switch device.client {
        case "ios": return "iphone"
        case "ipad": return "ipad"
        case "mac", "macos": return "laptopcomputer"
        case "desktop": return "desktopcomputer"
        default: return "globe"
        }
    }

    /// "This device" / "Last used 3 hours ago · since Oct 5".
    static func detail(_ device: Device) -> String {
        var parts: [String] = []
        if device.current {
            parts.append("This device")
        } else if device.lastSeen > 0 {
            parts.append("Last used \(Date(timeIntervalSince1970: Double(device.lastSeen) / 1000).formatted(.relative(presentation: .named)))")
        }
        if device.createdAt > 0 {
            parts.append("since \(Date(timeIntervalSince1970: Double(device.createdAt) / 1000).formatted(.dateTime.month(.abbreviated).day()))")
        }
        if device.isListener { parts.append("OBS link") }
        return parts.joined(separator: " · ")
    }
}
