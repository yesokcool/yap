import SwiftUI
import YapCore

/// A message's menu (long-press / right-click): quick reactions, Add
/// reaction / Doodle reaction, Reply, Pin, Save to Personal, Edit, Copy
/// text, Delete. Menu items are the system's; the row passes what it can do.
struct MessageMenuItems: View {
    @Environment(AppModel.self) private var model
    var message: Message
    var content: MessageContent
    var react: () -> Void
    /// nil in the Personal space (no replies there).
    var reply: (() -> Void)?
    /// My own message's edit (nil for others').
    var edit: (() -> Void)?
    /// Asks first (`deleteMessagePrompt`).
    var delete: () -> Void

    static let quickReactions = ["👍", "❤️", "😂", "🎉", "😮", "😢"]

    var body: some View {
        let client = model.client
        let mine = message.uid == client.uid
        if !message.isPending {
            ControlGroup {
                ForEach(Self.quickReactions.prefix(4), id: \.self) { emoji in
                    Button(emoji) { client.react(to: message, emoji: emoji, on: true) }
                }
            }
            Button("Add reaction…", systemImage: "face.smiling", action: react)
            Button("Doodle reaction…", systemImage: "scribble") { model.doodleTarget = message }
            if let reply {
                Button("Reply", systemImage: "arrowshape.turn.up.left", action: reply)
            }
            Button(message.pinnedAt == nil ? "Pin" : "Unpin", systemImage: message.pinnedAt == nil ? "pin" : "pin.slash") {
                client.pin(message, on: message.pinnedAt == nil)
            }
            if model.canSaveToPersonal(message) {
                Button("Save to Personal", systemImage: "bookmark") { client.saveToPersonal(message) }
            }
        }
        if mine, !message.isPending, let edit, message.roll == nil, message.poll == nil {
            Button("Edit", systemImage: "pencil", action: edit)
        }
        if !content.text.isEmpty {
            Button("Copy text", systemImage: "doc.on.doc") { copyToPasteboard(EmojiCodec.plain(content.text)) }
        }
        if !message.isPending, mine || client.isMod {
            Button("Delete message…", systemImage: "trash", role: .destructive, action: delete)
        }
    }
}

extension View {
    /// "Delete this message? It's erased for good, for everyone." (a Windows 98 message box).
    func deleteMessagePrompt(isPresented: Binding<Bool>, message: Message, client: YapClient) -> some View {
        question(isPresented: isPresented, .deleteMessage { client.delete(message) })
    }
}
