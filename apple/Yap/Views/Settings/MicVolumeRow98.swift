import SwiftUI
import YapVoice

/// Voice › Mic & output › Mic volume: the mic's own level (`MicVolume`),
/// only where the device lets Yap set it. iPhone: in a call (that's when
/// the mic is Yap's); the Mac: its input device's level, any time.
struct MicVolumeRow98: View {
    @Environment(AppModel.self) private var model
    @State private var settable = false
    @State private var level: Float = 1
    #if os(macOS)
    @State private var device: String?
    #endif

    var body: some View {
        let inCall = model.voice.isInVoice
        Group {
            if settable {
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("\(MicVolumeWords.title):").font(Theme.chrome).foregroundStyle(Theme.text)
                        Spacer()
                        Text(MicVolumeWords.percent(level)).font(Theme.chrome.monospacedDigit()).foregroundStyle(Theme.text)
                    }
                    Slider(value: Binding(get: { level }, set: { level = $0; MicVolume.pick($0) }), in: 0...1, step: 0.05)
                        .accessibilityLabel(MicVolumeWords.title)
                        .accessibilityValue(MicVolumeWords.percent(level))
                    #if os(macOS)
                    Text(MicVolumeWords.macNote(device))
                        .font(Theme.small)
                        .foregroundStyle(Theme.hint)
                        .fixedSize(horizontal: false, vertical: true)
                    #else
                    Text(MicVolumeWords.phoneNote)
                        .font(Theme.small)
                        .foregroundStyle(Theme.hint)
                        .fixedSize(horizontal: false, vertical: true)
                    #endif
                }
            }
        }
        // The level is the device's: look again every second while this
        // shows (a new mic, a route change, System Settings moving it).
        .task(id: inCall) {
            while !Task.isCancelled {
                refresh()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    private func refresh() {
        #if os(iOS)
        let can = model.voice.isInVoice && MicVolume.settable
        if can != settable { settable = can }
        if can { level = MicVolume.chosen ?? MicVolume.current }
        #else
        let can = MicVolume.settable
        if can != settable { settable = can }
        if can, let now = MicVolume.current, abs(now - level) > 0.004 { level = now }
        let name = MicVolume.deviceName
        if name != device { device = name }
        #endif
    }
}

enum MicVolumeWords {
    static let title = "Mic volume"
    static func percent(_ level: Float) -> String { "\(Int((level * 100).rounded()))%" }
    static let phoneNote = "How loud your mic picks you up in calls. Friends say you're too quiet or too loud? Move this."
    static func macNote(_ device: String?) -> String {
        "Your Mac's input level\(device.map { " for \($0)" } ?? ""), the same as System Settings › Sound › Input."
    }
}
