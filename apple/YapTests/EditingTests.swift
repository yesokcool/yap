import Foundation
import Testing
@testable import Yap
import YapCore

/// Editing in the composer: the message's text comes in with custom emoji
/// as `:name:`, and Cancel gives back what was typed before.
@Suite("Message editing")
struct EditingTests {
    @MainActor
    @Test func beginKeepsTheDraftAndShowsShortcodes() throws {
        var draft = "half a thought"
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        let message = Message(id: "m1", channelId: "general", uid: "me", body: "nice <:yap:0123456789abcdef0123456789abcdef> work", ts: 1)
        let edit = try #require(MessageEdit.begin(message, client: client, draft: &draft))
        #expect(draft == "nice :yap: work")
        #expect(edit.savedDraft == "half a thought")
        #expect(edit.message.id == "m1")
        // A sealed note can't be edited while the space is locked.
        var other = ""
        let sealed = Message(id: "m2", channelId: "p_me", uid: "me", body: "e1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", ts: 2)
        #expect(MessageEdit.begin(sealed, client: client, draft: &other) == nil)
        #expect(other.isEmpty)
    }

    @MainActor
    @Test func sealedNotesShowWhatTheySay() throws {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        let plain = Message(id: "m1", channelId: "general", uid: "u", body: "hi", ts: 1, image: ImageRef(id: "a", width: 1, height: 1))
        let content = MessageContent(plain, client: client)
        #expect(content.text == "hi" && content.image?.id == "a" && content.sealedNotice == nil)
        let locked = MessageContent(Message(id: "m2", channelId: "p", uid: "u", body: "e1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", ts: 2), client: client)
        #expect(locked.text.isEmpty && locked.sealedNotice?.contains("Unlock") == true)
        let long = MessageContent(Message(id: "m3", channelId: "g", uid: "u", body: String(repeating: "word ", count: 900), ts: 3), client: client)
        #expect(long.isLong && long.shownText(expanded: false).count < long.text.count && long.shownText(expanded: true) == long.text)
    }

    @Test func emojiToneSettingIsClamped() {
        let defaults = UserDefaults.standard
        let old = defaults.object(forKey: EmojiTone.storageKey)
        defer { defaults.set(old, forKey: EmojiTone.storageKey) }
        defaults.set(9, forKey: EmojiTone.storageKey)
        #expect(EmojiToneSetting.current == 5)
        defaults.set(2, forKey: EmojiTone.storageKey)
        #expect(EmojiPick.standard(StandardEmoji(char: "👋", group: 1, codes: ["wave"], words: "", skins: ["👋🏻", "👋🏼", "👋🏽", "👋🏾", "👋🏿"])).reactionValue == "👋🏼")
    }
}
