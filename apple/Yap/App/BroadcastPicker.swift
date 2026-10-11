#if os(iOS)
import ReplayKit
import SwiftUI
import UIKit

/// iOS's "Screen Broadcast" sheet, preset to Yap's extension
/// (`<bundle id>.broadcast`, the `YapBroadcast` target). The picker only
/// opens from a view in a window, so an invisible one sits next to the
/// Share screen button, which asks it to open.
struct BroadcastPickerHost: UIViewRepresentable {
    func makeUIView(context: Context) -> RPSystemBroadcastPickerView {
        let picker = RPSystemBroadcastPickerView(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
        picker.preferredExtension = BroadcastPicker.extensionIdentifier
        picker.showsMicrophoneButton = false
        picker.alpha = 0.02
        picker.isAccessibilityElement = false
        picker.accessibilityElementsHidden = true
        BroadcastPicker.current = picker
        return picker
    }

    func updateUIView(_ picker: RPSystemBroadcastPickerView, context: Context) {
        BroadcastPicker.current = picker
    }
}

enum BroadcastPicker {
    /// The picker on screen right now (next to the Share screen button).
    static weak var current: RPSystemBroadcastPickerView?

    static var extensionIdentifier: String { AppIdentity.bundleIdentifier + ".broadcast" }

    /// Opens the sheet with Yap selected; false when there's no picker in a window.
    static func open() -> Bool {
        guard let picker = current, picker.window != nil,
              let button = picker.subviews.lazy.compactMap({ $0 as? UIButton }).first else { return false }
        button.sendActions(for: .touchUpInside)
        return true
    }
}
#endif
