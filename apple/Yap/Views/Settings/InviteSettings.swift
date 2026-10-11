import SwiftUI
import YapCore

/// Settings › Invite: make a personal link (shown once, with Copy and
/// Share), and the list of your links (everyone's for owner/mods) with
/// "Turn off".
struct InviteGroup: View {
    @Environment(AppModel.self) private var model
    @State private var label = ""
    @State private var showingAll = false

    var body: some View {
        let client = model.client
        GroupBox98(title: "Invite") {
            Text("Make a link for a friend. They still wait in the waiting room until someone lets them in, and everyone sees whose invite it was.")
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            Text("Who's it for? (optional)").font(Theme.chrome).foregroundStyle(Theme.text)
            HStack(spacing: 6) {
                Field98 {
                    TextField("", text: $label, prompt: Text("Cleo").foregroundStyle(Theme.grayText))
                        .onSubmit(make)
                }
                Button("Make a link", action: make)
                    .buttonStyle(.win98Default)
            }
            if let created = client.createdInvite {
                CreatedInvite98(created: created)
            }
            Text("Your links:").font(Theme.chrome).foregroundStyle(Theme.text)
            VStack(alignment: .leading, spacing: 0) {
                if client.invites.isEmpty {
                    Text("No invite links yet.")
                        .font(Theme.chrome)
                        .foregroundStyle(Theme.grayText)
                        .padding(6)
                } else {
                    ForEach(ShortList.rows(client.invites, all: showingAll)) { invite in
                        InviteRow98(invite: invite)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .well()
            if ShortList.isCut(client.invites.count, all: showingAll) {
                Button("Show all \(client.invites.count)") { showingAll = true }
                    .buttonStyle(.win98)
            }
            if client.isMod, let endpoint = client.endpoint {
                // Owner/mods only, like the web: the main link never expires.
                Text(MainInvite.warning)
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                FlowLayout(spacing: 6) {
                    Button("Copy main invite link") { MainInvite.copy(endpoint, client: client) }
                        .buttonStyle(.win98)
                    ShareLink(item: endpoint.inviteLink) { Text("Share…") }
                        .buttonStyle(.win98)
                }
            }
        }
        .onChange(of: client.mySid, initial: true) { _, sid in
            if sid != nil { client.refreshInvites() }
        }
    }

    private func make() {
        model.client.createInvite(label: label.isEmpty ? "Invite" : label)
        label = ""
    }
}

/// The new link, once: a tooltip-yellow note with Copy and Share.
struct CreatedInvite98: View {
    @Environment(AppModel.self) private var model
    var created: YapClient.CreatedInvite

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Your new link (\(created.invite.label)). Copy it now — Yap only keeps a fingerprint, so it can't show it again.")
                .font(Theme.chrome)
                .fixedSize(horizontal: false, vertical: true)
            Text(created.link)
                .font(Theme.mono)
                .textSelection(.enabled)
                .lineLimit(3)
                .padding(4)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Theme.window)
                .overlay(Bevel(style: .sunken))
            HStack(spacing: 6) {
                Button("Copy") {
                    copyToPasteboard(created.link)
                    model.client.notice = YapClient.Notice(text: "Invite link copied.")
                }
                .buttonStyle(.win98)
                ShareLink(item: created.link) { Text("Share…") }
                    .buttonStyle(.win98)
                Spacer()
                Button("Done") { model.client.createdInvite = nil }
                    .buttonStyle(.win98)
            }
        }
        .foregroundStyle(Theme.tooltipText)
        .padding(8)
        .background(Theme.tooltip)
        .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
    }
}

struct InviteRow98: View {
    @Environment(AppModel.self) private var model
    var invite: Invite
    @State private var confirming = false

    var body: some View {
        let client = model.client
        HStack(alignment: .center, spacing: 6) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(invite.label).font(Theme.chatBold).foregroundStyle(invite.revoked ? Theme.grayText : Theme.text)
                    if invite.by != client.uid {
                        Text("by \(client.name(of: invite.by))").font(Theme.small).foregroundStyle(Theme.grayText)
                    }
                }
                Text(InviteText.summary(invite))
                    .font(Theme.small)
                    .foregroundStyle(Theme.grayText)
            }
            Spacer(minLength: 4)
            if !invite.revoked {
                Button("Turn off") { confirming = true }
                    .buttonStyle(.win98Compact)
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .question(isPresented: $confirming, .turnOffInvite(invite.label) { [client = model.client, id = invite.id] in client.revokeInvite(id) })
    }
}

/// The server's main invite link: owner/mods only (Settings › Invite), with
/// the web's warning. Everyone else makes personal links.
enum MainInvite {
    static let warning = "The main link never expires and doesn't say who invited whom. Only share it if you mean to."

    @MainActor
    static func copy(_ endpoint: ServerEndpoint, client: YapClient) {
        copyToPasteboard(endpoint.inviteLink)
        client.notice = YapClient.Notice(text: "Invite link copied.")
    }
}

enum InviteText {
    /// "2 people joined · made 3 days ago · turned off"
    static func summary(_ invite: Invite) -> String {
        var parts = [invite.uses == 1 ? "1 person joined" : "\(invite.uses) people joined"]
        let made = Date(timeIntervalSince1970: Double(invite.createdAt) / 1000)
        parts.append("made \(made.formatted(.relative(presentation: .named)))")
        if invite.revoked { parts.append("turned off") }
        return parts.joined(separator: " · ")
    }
}

/// Settings › Owner & mods: who did what (never message text).
struct ModsGroup: View {
    @Environment(AppModel.self) private var model
    @State private var showingAll = false

    var body: some View {
        let client = model.client
        GroupBox98(title: "Activity") {
            Text("Who let people in, banned, made invites, changed Places… Never message text. Everyone can read it.")
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 0) {
                if let entries = client.modLog {
                    if entries.isEmpty {
                        Text("Nothing yet.").font(Theme.chrome).foregroundStyle(Theme.grayText).padding(6)
                    }
                    ForEach(Array(ShortList.rows(entries, all: showingAll).enumerated()), id: \.offset) { _, entry in
                        let who = Text(client.name(of: entry.actor)).bold()
                        let when = Text(entry.date.formatted(.relative(presentation: .named))).foregroundStyle(Theme.grayText).font(Theme.small)
                        Text("\(who) \(entry.phrase { client.name(of: $0) }) \(when)")
                            .font(Theme.chrome)
                            .foregroundStyle(Theme.text)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                } else {
                    ProgressView().padding(6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .well()
            if let entries = client.modLog, ShortList.isCut(entries.count, all: showingAll) {
                Button("Show all \(entries.count)") { showingAll = true }
                    .buttonStyle(.win98)
            }
        }
        .onChange(of: client.mySid, initial: true) { _, sid in
            if sid != nil { client.refreshModLog() }
        }
    }
}
