import SwiftUI
import YapCore
import YapVoice

/// Wording and behaviour of Settings › Voice's options that apply from the
/// next join (the web's wording).
enum VoiceOptions {
    static let relayTitle = "Hide my IP address in calls"

    /// nil = not asked yet. On without a relay: no voice at all (it never
    /// goes straight to friends instead), so it says so, and it can still be
    /// turned off.
    static func relayNote(available: Bool?, on: Bool = false) -> String {
        switch available {
        case false? where on: "This server has no voice relay, so while this is on you can't connect to anyone in a call."
        case false?: "Needs the voice relay, which isn't switched on for this server yet."
        default: "Sends your voice through the server's voice relay instead of straight to friends (a little more delay)."
        }
    }

    /// In the call (and next to each person) while it's on with no relay to go through.
    static let relayMissing = "Hide my IP address is on, but this server has no voice relay, so you can't connect to anyone. Turn it off in Settings › Voice."
    /// The call bar's short version.
    static let relayMissingShort = "Hide my IP is on: no voice relay"

    static let headphonesTitle = "High-quality headphone audio (uses the phone's mic)"
    static let headphonesNote = "Bluetooth headphones keep their full stereo sound instead of switching to call quality; your voice comes from the phone's own mic."

    static let meterOffNote = "Join a voice channel (or hear yourself) to see your mic level."

    /// The mic level the meter shows: the call's, or Hear yourself's while it records.
    @MainActor
    static func meterLevel(_ model: AppModel) -> Double? {
        if model.voice.isInVoice { return model.voice.engine.localLevel }
        return model.micCheck.phase == .recording ? model.micCheck.level : nil
    }

    /// The same mic's peaks keep reaching −1 dBFS ("Too loud (clipping)").
    @MainActor
    static func meterTooLoud(_ model: AppModel) -> Bool {
        if model.voice.isInVoice { return model.voice.engine.tooLoud }
        return model.micCheck.phase == .recording && model.micCheck.tooLoud
    }

    // Mic check (the web's wording).
    static let hearTitle = "Hear yourself"
    static let hearHeading = "Mic check"
    #if os(iOS)
    static let hearNote = "Hear yourself the way others do: the phone's voice processing, voice activity and the call's quality included."
    #else
    static let hearNote = "Hear yourself the way others do: voice processing, voice activity and the call's quality included."
    #endif

    /// It needs the mic to itself, so it's offered outside a call.
    @MainActor
    static func canHear(_ model: AppModel) -> Bool {
        !model.voice.isInVoice && !model.micCheck.isRunning
    }

    @MainActor
    static func hearStatus(_ model: AppModel) -> String {
        let check = model.micCheck
        let codec = check.throughCodec ? "" : " (without the codec)"
        switch check.phase {
        case .recording: return "Recording 4 seconds — say something…"
        case .playing: return "Playing it back…" + codec
        case .done: return "That’s how you sound to others." + codec
        case .failed(let message): return message
        case .idle: return model.voice.isInVoice ? "Leave the call first: the check needs your mic to itself." : ""
        }
    }

    /// Both apply when you next join a voice channel.
    @MainActor
    static func changed(_ model: AppModel) {
        if model.voice.isInVoice {
            model.client.notice = YapClient.Notice(text: "Takes effect the next time you join a voice channel.")
        }
    }
}

/// Round 10's words (PROTOCOL.md › Voice › Transmit gate, Connection hints,
/// Shared media, for yourself).
enum TalkWords {
    /// By the Mute button when you talk while muted.
    static let youreMuted = "You're muted"
    /// Said once per device, the first time the nudge blips.
    static let explainNudge = "That sound means you're muted. Turn it off in Settings › Voice."
    static let reminder = "Remind me when I talk while muted"
    static let reminderNote = "“You're muted” by the button, and a soft blip the first time after you mute."
    static let pushToTalkClick = "Push-to-talk click"
    static let pushToTalkClickNote = "A quiet click when it opens and closes (only you hear it)."
    /// By the push-to-talk button after a double-tap.
    static let live = "Live — tap to stop"
    static let ownConnection = "Your connection is dropping some of your audio"
    static func ownConnectionDetail(lost: Double) -> String {
        let percent = max(1, Int((lost * 100).rounded()))
        return "The others say about \(percent)% of what you send never reaches them, so it's your connection, not theirs: a busy Wi‑Fi or mobile network. It usually passes; a cable, or getting closer to the router, helps."
    }
    static let dip = "Lower shared media while friends talk"
    static let dipNote = "A shared screen's sound goes to half while someone in your call speaks, then back. (Watch together plays in your browser, with its own volume.)"
    static let tooLoud = "Too loud (clipping)"
    #if os(iOS)
    static let tooLoudDetail = "The loudest moments hit the top and distort for everyone. Hold the mic a little further away."
    #else
    static let tooLoudDetail = "The loudest moments hit the top and distort for everyone. Turn your mic's input level down (in System Settings › Sound), or sit a little further from it."
    #endif
}

/// The call report (PROTOCOL.md › Voice › Call report), the web's words.
enum CallReportWords {
    /// The call's ⋯ menu.
    static let copy = "Copy call report"
    /// Settings › Voice.
    static let copyLast = "Copy last call report"
    static let note = "What Yap noticed about your last call: connections, lost audio, devices. No messages or IP addresses. Send it along when a call sounded off."
    static let copied = "Call report copied."
    static let none = "No call yet."
}

/// Settings › Voice › Mic mode (iPhone and iPad; Apple's picker).
enum MicModeWords {
    /// "Mic mode: Standard" (or Voice Isolation / Wide Spectrum: what iOS reports).
    static func row(_ mode: String) -> String { "Mic mode: \(mode)" }
    static let change = "Change…"
    static let note = "Voice Isolation cuts the noise around you; Wide Spectrum keeps all of it. Apple's setting, available while you're in a call."
}

/// The call's sound quality, the web's way (its voice panel): the panel
/// says "Sound: 256k for everyone" only when someone set one quality for
/// the whole channel; the picker is under the call's ⋯.
enum CallQuality {
    static let menuTitle = "Sound quality for everyone here"
    /// Under 256k and 510k in the menu (short: a menu line).
    static let menuWarning = "No dropout protection; several times the upload"
    static let menuLabel = "Call settings"

    /// "Sound: 256k for everyone", or nil while each person uses their own.
    static func shared(_ channel: Channel?) -> String? {
        channel?.bitrate.map { "Sound: \($0 / 1000)k for everyone" }
    }

    /// A ⋯ menu choice: "Each person's own (yours: 128k)" / "256k for everyone".
    static func choice(_ bitrate: Int?, own: Int) -> String {
        bitrate.map { "\($0 / 1000)k for everyone" } ?? "Each person's own (yours: \(own / 1000)k)"
    }
}

/// The ⋯ menu's items: one quality for everyone in the channel, or each
/// person's own (`channel.bitrate`), with a check on the current one.
struct CallQualityMenuItems: View {
    @Environment(AppModel.self) private var model
    var channel: Channel

    var body: some View {
        Section(CallQuality.menuTitle) {
            ForEach(VoiceSettings.channelBitrateChoices, id: \.self) { bitrate in
                Button {
                    model.voice.setChannelBitrate(bitrate, for: channel.id)
                } label: {
                    let title = CallQuality.choice(bitrate, own: model.voice.settings.bitrate)
                    if bitrate == channel.bitrate {
                        Label(title, systemImage: "checkmark")
                    } else {
                        Text(title)
                    }
                    // Next to 256 and 510 (the full words show under the call's quality line).
                    if bitrate.flatMap(VoiceSettings.qualityWarning) != nil {
                        Text(CallQuality.menuWarning)
                    }
                }
            }
        }
    }
}
