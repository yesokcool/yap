import SwiftUI
import YapCore
import YapVoice

/// People, Win98-style: a window opened on demand (the "People" button), not
/// a sidebar. Waiting (owner and mods only: they let people in) then everyone A–Z in
/// a white sunken list, with who's in voice. No online/offline anywhere
/// (PROTOCOL.md › Presence). Tapping someone opens their info.
struct MembersView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var selected: User?
    @State private var highlighted: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 4) {
                ScrollView {
                    groups.padding(.vertical, 2)
                }
                .well()
                HStack(spacing: 2) {
                    // Personal links (Settings › Invite); owner/mods find the main link there too.
                    Button { model.showSettings(.invite) } label: {
                        StatusPanel(text: "Invite someone…", systemImage: "person.badge.plus", color: Theme.link)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(4)
            .background(Theme.face)
            .win98Sheet("People") {
                CloseSheetButton { dismiss() }
            }
        }
        .presentationDetents([.large])
        #if os(macOS)
        .frame(minWidth: 380, minHeight: 480)
        #endif
        .sheet(item: $selected, onDismiss: { highlighted = nil }) { user in
            MemberSheet(user: user)
        }
        .pinResetNotice(active: selected == nil)
    }

    @ViewBuilder private var groups: some View {
        let client = model.client
        let waiting = JoinRequests.waiting(client)
        VStack(alignment: .leading, spacing: 0) {
            if !waiting.isEmpty {
                GroupHeader98(title: "Waiting (\(waiting.count))")
                Text(JoinRequests.note)
                    .font(Theme.small)
                    .foregroundStyle(Theme.grayText)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                ForEach(waiting) { user in waitingRow(user) }
                GroupHeader98(title: "Everyone")
            }
            ForEach(client.members) { user in buddyRow(user) }
        }
    }

    private func waitingRow(_ user: User) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Avatar(user: user, size: 16)
                Text(user.name).font(Theme.chatBold).foregroundStyle(Theme.text)
                Text("wants to join").font(Theme.small).foregroundStyle(Theme.grayText)
            }
            if let invitedBy = user.invitedBy {
                Text("invited by \(invitedBy)").font(Theme.small).foregroundStyle(Theme.grayText)
                    .padding(.leading, 24)
            }
            if let warning = user.invite?.warning {
                // A personal link that was passed on: check before letting them in.
                Label {
                    Text(warning).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.warn)
                }
                .font(Theme.small)
                .foregroundStyle(Theme.text)
                .padding(.leading, 24)
            }
            HStack(spacing: 6) {
                Button("Let in") { model.client.approve(user.id) }
                    .buttonStyle(.win98Default)
                Button("Turn away") { model.client.deny(user.id) }
                    .buttonStyle(.win98)
            }
            .padding(.leading, 24)
        }
        .padding(.leading, 8)
        .padding(.trailing, 6)
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.mentionBackground)
    }

    private func buddyRow(_ user: User) -> some View {
        let client = model.client
        let sessions = client.sessions(of: user.id)
        let voice = sessions.compactMap(\.voice).first
        let speaking = sessions.contains { model.voice.engine.speaking.contains($0.sid) }
        let isMe = user.id == client.uid
        let isSelected = highlighted == user.id
        return Button {
            guard !isMe else { return }
            highlighted = user.id
            selected = user
        } label: {
            HStack(spacing: 6) {
                Avatar(user: user, size: 16, speaking: speaking)
                Text(user.name)
                    .font(Theme.chat)
                    .foregroundStyle(isSelected ? Theme.selectionText : Theme.text)
                    .lineLimit(1)
                if user.role == .owner || user.role == .mod {
                    Text(user.role == .owner ? "(owner)" : "(mod)")
                        .font(Theme.small)
                        .foregroundStyle(isSelected ? Theme.selectionText : Theme.grayText)
                }
                if isMe {
                    Text("(you)").font(Theme.small).foregroundStyle(isSelected ? Theme.selectionText : Theme.grayText)
                }
                Spacer(minLength: 4)
                if let voice, let channel = client.channelsById[voice.channelId] {
                    HStack(spacing: 2) {
                        Image(systemName: voice.deafened ? "speaker.slash.fill" : voice.muted ? "mic.slash.fill" : speaking ? "speaker.wave.2.fill" : "speaker.fill")
                            .font(.system(size: 9))
                            .foregroundStyle(voice.muted || voice.deafened ? Theme.danger : speaking ? Theme.ok : Theme.grayText)
                        Text("in \(channel.name)").font(Theme.small).foregroundStyle(isSelected ? Theme.selectionText : Theme.grayText)
                    }
                } else if sessions.contains(where: { $0.view == "board" }) {
                    Text("on the board").font(Theme.small).foregroundStyle(isSelected ? Theme.selectionText : Theme.grayText)
                }
                if let away = user.awayNow() {
                    // AIM's away message, in context only.
                    Text("away: \(away.text)")
                        .font(Theme.small)
                        .italic()
                        .foregroundStyle(isSelected ? Theme.selectionText : Theme.grayText)
                        .lineLimit(1)
                        .help(away.text)
                }
            }
            .padding(.leading, 8)
            .padding(.trailing, 6)
            .padding(.vertical, 3)
            #if os(iOS)
            .frame(minHeight: 32)
            #endif
            .background(isSelected ? Theme.selection : .clear)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityHint(isMe ? "" : "Shows their info")
    }
}

/// A list group label: bold on the face color, like the channel list's.
struct GroupHeader98: View {
    var title: String

    var body: some View {
        Text(title)
            .font(Theme.chromeBold)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.face)
            .overlay(alignment: .bottom) { Theme.shadow.frame(height: 1) }
            .accessibilityAddTraits(.isHeader)
    }
}

/// "Buddy Info": their volume in calls, and owner/mod actions.
struct MemberSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var user: User
    @State private var confirmBan = false
    @State private var confirmReset = false

    var body: some View {
        let client = model.client
        let live = client.users[user.id] ?? user
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 10) {
                        Avatar(user: live, size: 40)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(live.name).font(Theme.chatBold).foregroundStyle(Theme.text)
                            RoleBadge(role: live.role)
                        }
                    }
                    if let away = live.awayNow() {
                        VStack(alignment: .leading, spacing: 2) {
                            Tooltip98(text: away.line(name: live.name))
                            let until = AwayPresets.untilLabel(away)
                            if !until.isEmpty { Text(until).font(Theme.small).foregroundStyle(Theme.grayText) }
                        }
                    }
                    GroupBox98(title: "Volume in voice") {
                        let volume = Binding(
                            get: { model.voice.settings.volumes[live.id] ?? 1 },
                            set: { model.voice.setVolume($0, for: live.id) }
                        )
                        HStack {
                            Slider(value: volume, in: 0...2, step: 0.05)
                            Text("\(Int((volume.wrappedValue * 100).rounded()))%")
                                .font(Theme.chrome.monospacedDigit())
                                .foregroundStyle(Theme.text)
                                .frame(width: 44, alignment: .trailing)
                        }
                        Text("Only on this device. Above 100% boosts quiet friends.")
                            .font(Theme.small)
                            .foregroundStyle(Theme.grayText)
                    }
                    if client.isMod && live.role != .owner || client.myRole == .owner {
                        GroupBox98(title: "Moderation") {
                            HStack(spacing: 6) {
                                if client.myRole == .owner, live.role == .member || live.role == .mod {
                                    Button(live.role == .mod ? "Remove Mod" : "Make Mod") {
                                        client.setRole(live.id, mod: live.role != .mod)
                                    }
                                    .buttonStyle(.win98)
                                }
                                if client.isMod, live.role != .owner {
                                    Button(PasswordText.reset) { confirmReset = true }
                                        .buttonStyle(.win98)
                                    Button("Ban…") { confirmBan = true }
                                        .buttonStyle(.win98)
                                }
                            }
                        }
                    }
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet(user.name) {
                CloseSheetButton { dismiss() }
            }
            .question(isPresented: $confirmBan, .ban(live.name) { [id = live.id] in
                client.ban(id)
                dismiss()
            })
            .question(isPresented: $confirmReset, .resetPassword(live.name) { [id = live.id] in client.resetPassword(for: id) })
            .pinResetNotice()
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(minWidth: 360, minHeight: 300)
        #endif
    }
}
