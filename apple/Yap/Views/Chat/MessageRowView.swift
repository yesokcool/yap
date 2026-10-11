import SwiftUI
import YapCore
import YapVoice

/// One message, AIM-style: `Name: message` inline — my name red, everyone
/// else's blue, bold — with the time in gray at the start of a run of
/// messages and a gray "(edited)" after edited text, then a picture or GIF
/// and reaction buttons. A reply starts with "↪ Name: first line…" (tap to
/// jump to it). Very long messages collapse behind "Show more". Long-press
/// for React / Reply / Edit / Copy / Delete.
struct MessageRowView: View {
    @Environment(AppModel.self) private var model
    var message: Message
    var grouped: Bool
    var react: () -> Void
    /// Starts editing (my own messages only).
    var edit: (() -> Void)? = nil
    /// Starts a reply (nil where there are none: the Personal space).
    var reply: (() -> Void)? = nil
    /// Scrolls to the message a reply quotes.
    var jumpToReply: ((String) -> Void)? = nil
    @State private var confirmingDelete = false
    @State private var expanded = false


    var body: some View {
        let client = model.client
        let user = client.user(message.uid)
        let mine = message.uid == client.uid
        let mentionsMe = client.mentionsMe(message)
        let content = MessageContent(message, client: client)
        VStack(alignment: .leading, spacing: 3) {
            if let quote = message.reply {
                if let source = model.quoteSource(quote, in: message.channelId) {
                    SavedFrom98(source: source)
                }
                ReplyQuote98(reply: quote) { model.openQuote(quote, in: message.channelId, local: jumpToReply) }
            }
            if message.pinnedAt != nil {
                Text("📌 Pinned").font(Theme.small).foregroundStyle(Theme.grayText)
                    .accessibilityLabel("Pinned")
            }
            if let notice = content.sealedNotice {
                Text("\(prefix(user: user, mine: mine))\(Text(notice).foregroundStyle(Theme.grayText).italic())")
                    .font(Theme.chat)
            } else if let poll = message.poll {
                prefix(user: user, mine: mine).font(Theme.chat)
                PollBox98(message: message, poll: poll)
                    .padding(.leading, 12)
            } else if let roll = message.roll {
                prefix(user: user, mine: mine).font(Theme.chat)
                RollPanel98(roll: roll)
                    .padding(.leading, 12)
            } else {
                MessageBodyView(source: content.shownText(expanded: expanded), prefix: prefix(user: user, mine: mine), suffix: editedSuffix)
            }
            if content.isLong {
                Button { expanded.toggle() } label: {
                    // The full label when it fits on one line (Win98 buttons don't wrap).
                    ViewThatFits(in: .horizontal) {
                        Text(content.foldLabel(expanded: expanded))
                        Text(expanded ? "Show less" : "Show more")
                    }
                }
                .buttonStyle(.win98Compact)
                .padding(.leading, 12)
            }
            if let image = content.image {
                PictureView(image: image)
                    .padding(.leading, 12)
            }
            if let sealed = content.sealedImage {
                SealedPictureView(image: sealed)
                    .padding(.leading, 12)
            }
            if let gif = content.gif {
                GifView(gif: gif)
                    .padding(.leading, 12)
            }
            if let video = YouTube.firstVideoID(in: content.text) {
                WatchButton98(videoId: video)
                    .padding(.leading, 12)
            }
            if !message.doodles.isEmpty {
                DoodleTiles98(message: message)
                    .padding(.leading, 12)
            }
            if !message.emojiReactions.isEmpty {
                ReactionPills(message: message, add: react)
                    .padding(.leading, 12)
            }
            AwayNotes98(messageId: message.id)
                .padding(.leading, 12)
        }
        .padding(.horizontal, 8)
        .padding(.top, grouped ? 1 : 5)
        .padding(.bottom, 1)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(mentionsMe ? Theme.mentionBackground : .clear)
        .opacity(message.isPending ? 0.55 : 1)
        .contentShape(.rect)
        .contextMenu {
            MessageMenuItems(message: message, content: content, react: react, reply: reply, edit: edit) { confirmingDelete = true }
        }
        .deleteMessagePrompt(isPresented: $confirmingDelete, message: message, client: client)
        .accessibilityElement(children: message.poll == nil ? .combine : .contain)
    }

    /// " (edited)" in gray after the text.
    private var editedSuffix: Text? {
        guard message.editedAt != nil else { return nil }
        var edited = AttributedString(" (edited)")
        edited.font = Theme.small
        edited.foregroundColor = Theme.grayText
        return Text(edited)
    }

    /// "(9:41 PM) Name: " — the time only at the start of a run.
    private func prefix(user: User?, mine: Bool) -> Text {
        var name = AttributedString(user?.name ?? "Someone")
        name.font = Theme.chatBold
        name.foregroundColor = mine ? Theme.ownName : Theme.otherName
        var colon = AttributedString(": ")
        colon.font = Theme.chatBold
        colon.foregroundColor = mine ? Theme.ownName : Theme.otherName
        if grouped { return Text(name + colon) }
        var time = AttributedString("(\(message.date.formatted(date: .omitted, time: .shortened))) ")
        time.font = Theme.small
        time.foregroundColor = Theme.grayText
        return Text(time + name + colon)
    }
}

/// "↪ Name: first line…" above a reply, in gray; tap to jump to the original.
struct ReplyQuote98: View {
    @Environment(AppModel.self) private var model
    var reply: Reply
    var jump: () -> Void

    var body: some View {
        Button(action: jump) {
            HStack(spacing: 4) {
                Image(systemName: "arrow.turn.down.right").font(.system(size: 9, weight: .bold))
                if reply.deleted {
                    Text(reply.preview).italic()
                } else {
                    Text("\(Text(model.client.name(of: reply.uid)).bold()): \(reply.preview)")
                }
            }
            .font(Theme.small)
            .foregroundStyle(Theme.grayText)
            .lineLimit(1)
            .padding(.leading, 12)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(reply.deleted)
        .accessibilityLabel(reply.deleted ? "Reply to a deleted message" : "Reply to \(model.client.name(of: reply.uid)): \(reply.preview)")
        .accessibilityHint(reply.deleted ? "" : "Shows the original")
    }
}

/// Reactions as small Win98 buttons: emoji + count; mine stay pushed in.
/// Tap toggles mine; the last one adds a new reaction.
struct ReactionPills: View {
    @Environment(AppModel.self) private var model
    var message: Message
    var add: () -> Void

    var body: some View {
        FlowLayout(spacing: 4) {
            ForEach(message.emojiReactions) { reaction in
                let mine = reaction.uids.contains(model.client.uid)
                Button {
                    model.client.toggleReaction(reaction.emoji, on: message)
                } label: {
                    HStack(spacing: 3) {
                        ReactionGlyph(emoji: reaction.emoji, size: 14)
                        Text("\(reaction.uids.count)").font(Theme.small.monospacedDigit())
                    }
                }
                .buttonStyle(Win98ToggleButtonStyle(isOn: mine, compact: true))
                .accessibilityLabel("\(reactionName(reaction.emoji)), \(reaction.uids.count)\(mine ? ", including you" : "")")
                .help(names(reaction))
            }
            Button(action: add) {
                Image(systemName: "plus").font(.system(size: 9, weight: .bold)).frame(minHeight: 16)
            }
            .buttonStyle(Win98ToggleButtonStyle(isOn: false, compact: true))
            .accessibilityLabel("Add reaction")
        }
        .sensoryFeedback(.selection, trigger: message.reactions)
    }

    private func names(_ reaction: Reaction) -> String {
        reaction.uids.compactMap { model.client.user($0)?.name }.joined(separator: ", ")
    }

    private func reactionName(_ emoji: String) -> String {
        EmojiCodec.tokens(in: emoji).first.map { ":\($0.name):" } ?? emoji
    }
}

/// A reaction's emoji: a character, or a custom `<:name:id>` image.
struct ReactionGlyph: View {
    @Environment(AppModel.self) private var model
    var emoji: String
    var size: CGFloat = 16

    var body: some View {
        if let token = EmojiCodec.tokens(in: emoji).first, let endpoint = model.client.endpoint {
            if let image = model.images.image(for: endpoint.emojiURL(token.id)) {
                Image(platformImage: image).resizable().scaledToFit().frame(width: size, height: size)
            } else {
                Text(":\(token.name):").font(Theme.small).foregroundStyle(Theme.grayText)
            }
        } else {
            Text(emoji).font(.system(size: size * 0.95))
        }
    }
}
