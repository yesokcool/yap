import CoreText
import SwiftUI
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Every color, font and metric the app draws with, in one place.
///
/// The look is Windows 98 / AIM: gray 3D "face" chrome with two-ring bevels,
/// white text wells, navy gradient title bars, square corners, no soft
/// shadows, Tahoma where the system has it. These are the same tokens the
/// web and desktop clients use. Light = "Windows Standard"; dark (the
/// default) = a period "Charcoal" scheme.
enum Theme {
    // MARK: System colors (light = Windows Standard, dark = Charcoal)

    /// 3D object face: window chrome, buttons, toolbars.
    static let face = Color.yap(light: 0xc0c0c0, dark: 0x3a3a3a)
    /// Bevel: outer top-left of raised things.
    static let hilight = Color.yap(light: 0xffffff, dark: 0x707070)
    /// Bevel: inner top-left of raised things.
    static let light = Color.yap(light: 0xdfdfdf, dark: 0x525252)
    /// Bevel: inner bottom-right of raised things.
    static let shadow = Color.yap(light: 0x808080, dark: 0x1f1f1f)
    /// Bevel: outer bottom-right of raised things.
    static let dkShadow = Color.yap(light: 0x000000, dark: 0x000000)
    /// Text wells: the chat log, lists, inputs.
    static let window = Color.yap(light: 0xffffff, dark: 0x1c1c1c)
    static let text = Color.yap(light: 0x000000, dark: 0xe6e6e6)
    static let grayText = Color.yap(light: 0x808080, dark: 0x8a8a8a)
    /// Hints on the grey face (Settings' notes, your card, the call bar, the
    /// sidebar's footer): grayText there was ~2:1, so faint it read as
    /// switched off. The web's mix: 62% text into the face. (Disabled text
    /// and grey words on the white wells stay grayText.)
    static let hint = Color.yap(light: 0x494949, dark: 0xa5a5a5)

    /// Active title bar: a horizontal gradient, bold white text.
    static let titleStart = Color.yap(light: 0x000080, dark: 0x000080)
    static let titleEnd = Color.yap(light: 0x1084d0, dark: 0x1084d0)
    static let titleText = Color.white
    static let inactiveTitleStart = Color.yap(light: 0x808080, dark: 0x2a2a2a)
    static let inactiveTitleEnd = Color.yap(light: 0xb5b5b5, dark: 0x555555)
    static let inactiveTitleText = Color.yap(light: 0xc0c0c0, dark: 0x9a9a9a)

    static let selection = Color.yap(light: 0x000080, dark: 0x0b3d91)
    static let selectionText = Color.white
    /// The desktop behind dialogs (sign-on, waiting room).
    static let desktop = Color.yap(light: 0x008080, dark: 0x0b2a2a)
    /// Tooltips and notices; the same in both schemes, with black text.
    static let tooltip = Color.yap(light: 0xffffe1, dark: 0xffffe1)
    static let tooltipText = Color.black
    static let link = Color.yap(light: 0x0000ff, dark: 0x8ab4ff)

    /// AIM screen names: my own red, everyone else blue.
    static let ownName = Color.yap(light: 0xff0000, dark: 0xff7070)
    static let otherName = Color.yap(light: 0x0000ff, dark: 0x7fb2ff)

    // MARK: Semantic (derived, kept period-plain)

    /// App tint for system controls (sliders, toggles, the tab bar).
    static let accent = Color.yap(light: 0x000080, dark: 0x8ab4ff)
    static let danger = Color.yap(light: 0xc00000, dark: 0xff7070)
    static let ok = Color.yap(light: 0x008000, dark: 0x5fd35f)
    static let warn = Color.yap(light: 0x808000, dark: 0xd8c050)
    /// Rows that @-mention me: tooltip yellow by day, a dim olive by night.
    static let mentionBackground = Color.yap(light: 0xffffe1, dark: 0x33321e)
    static let codeBackground = Color.yap(light: 0xf0f0f0, dark: 0x262626)
    /// The frame around someone who's speaking.
    static let speaking = ok

    // MARK: Metrics

    /// Square corners everywhere.
    static let radius: CGFloat = 0
    #if os(macOS)
    static let titleBarHeight: CGFloat = 20
    static let minButtonSize = CGSize(width: 75, height: 23)
    #else
    static let titleBarHeight: CGFloat = 26
    /// Drawn Win98-size; the hit target is padded out to 44 pt.
    static let minButtonSize = CGSize(width: 64, height: 32)
    #endif

    // MARK: Type

    /// Tahoma if the system has it (the Mac does), otherwise the system font.
    static let hasTahoma: Bool = {
        let font = CTFontCreateWithName("Tahoma" as CFString, 12, nil)
        return (CTFontCopyFamilyName(font) as String) == "Tahoma"
    }()

    #if os(macOS)
    /// Chrome: menus, labels, buttons (~11 pt).
    static let chrome = mac(11)
    static let chromeBold = mac(11, bold: true)
    /// Window and panel titles: bold.
    static let title = mac(11, bold: true)
    static let small = mac(10)
    /// Chat text (13 pt).
    static let chat = mac(13)
    static let chatBold = mac(13, bold: true)
    static let mono = Font.system(size: 12, design: .monospaced)

    private static func mac(_ size: CGFloat, bold: Bool = false) -> Font {
        hasTahoma ? .custom(bold ? "Tahoma-Bold" : "Tahoma", size: size) : .system(size: size, weight: bold ? .bold : .regular)
    }
    #else
    // iOS: small system sizes that still follow Dynamic Type.
    static let chrome = Font.footnote
    static let chromeBold = Font.footnote.bold()
    static let title = Font.subheadline.bold()
    static let small = Font.caption
    static let chat = Font.subheadline
    static let chatBold = Font.subheadline.bold()
    static let mono = Font.system(.footnote, design: .monospaced)
    #endif

    /// The person's color ("#rrggbb") — for their buddy icon only.
    static func color(hex: String) -> Color {
        Color.yap(hex: hex) ?? grayText
    }

    /// Colors offered at sign-up (the server stores one per person).
    static let personColors = ["#000080", "#800000", "#008000", "#808000", "#008080", "#800080", "#c06000", "#0060c0", "#606060", "#a00060"]
}

/// Appearance: Charcoal (dark, the default), Windows Standard (light), or the system's.
enum Appearance: String, CaseIterable, Identifiable {
    case charcoal, standard, system
    static let storageKey = "yap.appearance"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .charcoal: "Charcoal (dark)"
        case .standard: "Windows Standard (light)"
        case .system: "Match system"
        }
    }
    var colorScheme: ColorScheme? {
        switch self {
        case .charcoal: .dark
        case .standard: .light
        case .system: nil
        }
    }
}

extension Color {
    /// A light/dark pair from hex values. The provider is `@Sendable` because
    /// UIKit/AppKit may resolve colors off the main thread.
    static func yap(light: UInt32, dark: UInt32) -> Color {
        #if os(iOS)
        Color(uiColor: UIColor { @Sendable traits in
            traits.userInterfaceStyle == .dark ? UIColor(rgb: dark) : UIColor(rgb: light)
        })
        #else
        Color(nsColor: NSColor(name: nil) { @Sendable appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? NSColor(rgb: dark) : NSColor(rgb: light)
        })
        #endif
    }

    static func yap(hex: String) -> Color? {
        let digits = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        guard digits.count == 6, let value = UInt32(digits, radix: 16) else { return nil }
        return Color(red: Double((value >> 16) & 0xFF) / 255, green: Double((value >> 8) & 0xFF) / 255, blue: Double(value & 0xFF) / 255)
    }
}

#if os(iOS)
extension UIColor {
    nonisolated convenience init(rgb: UInt32, alpha: CGFloat = 1) {
        self.init(red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255, blue: CGFloat(rgb & 0xFF) / 255, alpha: alpha)
    }
}
#else
extension NSColor {
    nonisolated convenience init(rgb: UInt32, alpha: CGFloat = 1) {
        self.init(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255, blue: CGFloat(rgb & 0xFF) / 255, alpha: alpha)
    }
}
#endif
