import Foundation

/// What you'd typed in each channel and not sent: it waits there while you
/// go back to the channel list or open another channel, and comes back when
/// you return (the web's per-channel drafts). In memory only, for this
/// account; a message being edited or a reply in progress isn't kept (as
/// before), only the draft behind it.
struct ChannelDrafts {
    private var texts: [String: String] = [:]

    /// The draft waiting in a channel ("" for none).
    func text(for channelId: String) -> String {
        texts[channelId] ?? ""
    }

    /// Leaving a channel: keep what's in its box (nothing to keep if it's blank).
    mutating func keep(_ text: String, for channelId: String) {
        if text.allSatisfy(\.isWhitespace) {
            texts.removeValue(forKey: channelId)
        } else {
            texts[channelId] = text
        }
    }

    /// Signed out: nothing typed for that account stays.
    mutating func removeAll() {
        texts.removeAll()
    }

    var isEmpty: Bool { texts.isEmpty }
}
