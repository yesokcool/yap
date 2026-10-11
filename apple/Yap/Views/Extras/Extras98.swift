import SwiftUI
import YapCore
import YapVoice

// The v0.36 extras in the Windows 98 skin (PROTOCOL.md › Extras): polls as
// a group box of radio buttons with progress bars, dice in a little sunken
// panel, Watch together as a push button, away messages as AIM's yellow
// note, doodle reactions as framed tiles.

/// A poll: the question in bold, each answer a radio button with a blocky
/// progress bar and its count. Click to vote; click yours again to take it back.
struct PollBox98: View {
    @Environment(AppModel.self) private var model
    var message: Message
    var poll: Poll

    var body: some View {
        let client = model.client
        let choice = poll.choice(of: client.uid)
        GroupBox98(title: "Poll") {
            MessageBodyView(source: message.body)
                .font(Theme.chatBold)
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(poll.options.enumerated()), id: \.offset) { index, option in
                    let picked = choice == index
                    HStack(spacing: 8) {
                        Radio98(title: option, isOn: picked) { client.vote(message, option: picked ? nil : index) }
                            .disabled(message.isPending)
                            .frame(minWidth: 120, alignment: .leading)
                        ProgressBar98(fraction: poll.share(index))
                            .frame(width: 90, height: 12)
                        Text("\(poll.count(index))")
                            .font(Theme.small.monospacedDigit())
                            .foregroundStyle(Theme.text)
                            .frame(minWidth: 18, alignment: .trailing)
                    }
                    .help(poll.votes[index].map { client.name(of: $0) }.joined(separator: ", "))
                    .accessibilityElement(children: .combine)
                    .accessibilityValue("\(poll.count(index)) \(poll.count(index) == 1 ? "vote" : "votes")")
                }
            }
            Text(poll.total == 0 ? "No votes yet." : "\(poll.total) \(poll.total == 1 ? "vote" : "votes")\(choice != nil ? " — \(PollText.takeBack) yours again to take it back" : "")")
                .font(Theme.small)
                .foregroundStyle(Theme.grayText)
        }
        // A little raised dialog on the white log: group boxes and radio
        // buttons are drawn for the grey face.
        .padding(6)
        .frame(maxWidth: 380, alignment: .leading)
        .bevel(.raised)
    }
}

/// A Win98 progress bar: navy blocks in a sunken well.
struct ProgressBar98: View {
    var fraction: Double

    var body: some View {
        GeometryReader { geo in
            let inner = max(0, geo.size.width - 4)
            let blocks = Int((inner * fraction) / 7)
            HStack(spacing: 1) {
                ForEach(0..<blocks, id: \.self) { _ in Theme.selection.frame(width: 6) }
            }
            .padding(2)
            .frame(width: geo.size.width, height: geo.size.height, alignment: .leading)
        }
        .background(Theme.window)
        .overlay(Bevel(style: .status))
        .accessibilityHidden(true)
    }
}

/// `/roll` and `/flip`: the result big and bold in a sunken panel.
struct RollPanel98: View {
    var roll: Roll

    var body: some View {
        HStack(spacing: 8) {
            Text(roll.isFlip ? "🪙" : "🎲").font(.system(size: 22))
            VStack(alignment: .leading, spacing: 0) {
                Text(roll.headline).font(Theme.chatBold).monospacedDigit().foregroundStyle(Theme.text)
                if !roll.detail.isEmpty {
                    Text(roll.detail).font(Theme.small.monospacedDigit()).foregroundStyle(Theme.grayText)
                }
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Theme.window)
        .overlay(Bevel(style: .sunken))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(roll.isFlip ? "Coin flip: \(roll.coin)" : "Rolled \(roll.total)")
    }
}

/// "▶ Watch together" under a message with a YouTube link.
struct WatchButton98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    var videoId: String

    var body: some View {
        Button {
            model.watchTogether(videoId, openURL: openURL)
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "play.fill").font(.system(size: 9))
                Text("Watch together")
            }
        }
        .buttonStyle(.win98Compact)
        .help("Play it in Watch together for everyone (opens in the browser)")
    }
}

/// "Ada is away: at work til 6" — AIM's away message, on yellow.
struct AwayNotes98: View {
    @Environment(AppModel.self) private var model
    var messageId: String

    var body: some View {
        if let notes = model.client.awayNotes[messageId] {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(notes, id: \.uid) { note in
                    Tooltip98(text: Away(text: note.text, until: note.until).line(name: model.client.name(of: note.uid)))
                }
            }
        }
    }
}

/// Doodle reactions: framed tiles about two lines tall; click for a big one.
struct DoodleTiles98: View {
    @Environment(AppModel.self) private var model
    var message: Message
    @State private var viewing: Reaction?

    var body: some View {
        FlowLayout(spacing: 4) {
            ForEach(message.doodles) { reaction in
                if let id = Doodle.id(fromReaction: reaction.emoji) {
                    Button { viewing = reaction } label: {
                        DoodleImage(id: id)
                            .frame(width: 40, height: 40)
                            .background(Color.white)
                            .padding(2)
                            .overlay(Bevel(style: reaction.uids.contains(model.client.uid) ? .pressed : .sunken))
                    }
                    .buttonStyle(.plain)
                    .help("Doodle by \(reaction.uids.map { model.client.name(of: $0) }.joined(separator: ", "))")
                    .accessibilityLabel("Doodle by \(reaction.uids.map { model.client.name(of: $0) }.joined(separator: ", "))")
                }
            }
        }
        .sheet(item: $viewing) { reaction in DoodleViewer98(message: message, reaction: reaction) }
    }
}

/// One doodle, big, in a Win98 window; yours can be removed.
struct DoodleViewer98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var message: Message
    var reaction: Reaction

    var body: some View {
        let client = model.client
        NavigationStack {
            VStack(spacing: 10) {
                if let id = Doodle.id(fromReaction: reaction.emoji) {
                    DoodleImage(id: id)
                        .frame(width: 192, height: 192)
                        .background(Color.white)
                        .padding(2)
                        .overlay(Bevel(style: .sunken))
                }
                Text("by \(reaction.uids.map { client.name(of: $0) }.joined(separator: ", "))")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                if reaction.uids.contains(client.uid) {
                    Button("Remove my doodle") {
                        client.react(to: message, emoji: reaction.emoji, on: false)
                        dismiss()
                    }
                    .buttonStyle(.win98)
                }
                Spacer(minLength: 0)
            }
            .padding(12)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Theme.face)
            .win98Sheet("Doodle") {
                CloseSheetButton { dismiss() }
            }
        }
        .presentationDetents([.medium])
        #if os(macOS)
        .frame(minWidth: 300, minHeight: 320)
        #endif
    }
}

/// Paint a doodle reaction: the 48 × 48 box in a sunken well, a color
/// palette like MS Paint's, brush sizes, eraser, wiggle.
struct DoodleWindow98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var message: Message
    @State private var editor = DoodleEditor()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .top, spacing: 10) {
                        DoodleCanvas(editor: editor, side: 240)
                            .padding(2)
                            .overlay(Bevel(style: .sunken))
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Brush:").font(Theme.chrome).foregroundStyle(Theme.text)
                            ForEach(DoodleDrawing.sizes, id: \.self) { size in
                                Radio98(title: size == 1 ? "Fine" : size == 2 ? "Medium" : "Thick", isOn: editor.size == size) { editor.size = size }
                            }
                            Check98(title: "Wiggle", isOn: $editor.drawing.wiggle)
                        }
                    }
                    // The palette, two rows like Paint's.
                    LazyVGrid(columns: Array(repeating: GridItem(.fixed(22), spacing: 2), count: 6), alignment: .leading, spacing: 2) {
                        ForEach(1..<DoodleDrawing.palette.count, id: \.self) { index in
                            Button { editor.color = index } label: {
                                DoodleEditor.color(index)
                                    .frame(width: 18, height: 18)
                                    .padding(2)
                                    .overlay(Bevel(style: editor.color == index ? .pressed : .sunken))
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(ColorName.ink(index))
                        }
                    }
                    HStack(spacing: 6) {
                        Button(editor.color == 0 ? "Eraser ✓" : "Eraser") { editor.color = 0 }.buttonStyle(.win98)
                        Button("Undo") { editor.undo() }.buttonStyle(.win98).disabled(!editor.canUndo)
                        Button("Clear") { editor.clear() }.buttonStyle(.win98).disabled(!editor.canUndo)
                    }
                    if let error = editor.error { ErrorLine(text: error) }
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet("Doodle reaction") {
                CloseSheetButton(hasChanges: editor.canUndo) { dismiss() }
            } done: {
                DoneButton { Task { if await editor.send(on: message, client: model.client) { dismiss() } } }
                    .disabled(!editor.canSend)
            }
        }
        .interactiveDismissDisabled(editor.canUndo)
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 420)
        #endif
    }
}

/// "Saved from #general" above a Personal-space bookmark.
struct SavedFrom98: View {
    var source: String

    var body: some View {
        Text("🔖 Saved from \(source)")
            .font(Theme.small)
            .foregroundStyle(Theme.grayText)
            .padding(.leading, 12)
    }
}

/// The call window's soundboard: a group of push buttons.
struct Soundboard98: View {
    @Environment(AppModel.self) private var model
    @AppStorage(SoundSettings.mutedKey) private var muted = false

    var body: some View {
        let client = model.client
        let inCall = model.voice.channelId != nil
        if !client.boardSounds.isEmpty {
            GroupBox98(title: muted ? "Soundboard (muted for you)" : "Soundboard") {
                FlowLayout(spacing: 4) {
                    ForEach(client.boardSounds) { sound in
                        Button { client.playSound(sound) } label: {
                            Text("\(sound.emoji ?? "🔊") \(sound.name)").lineLimit(1)
                        }
                        .buttonStyle(Win98ToggleButtonStyle(isOn: model.sounds.playing.contains(sound.id), compact: true))
                        .disabled(!inCall)
                        .accessibilityLabel("Play \(sound.name)")
                    }
                }
            }
        }
    }
}
