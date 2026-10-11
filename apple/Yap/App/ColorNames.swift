import Foundation
import YapCore

/// A spoken name for a color swatch ("dark blue", "pink"), so VoiceOver
/// doesn't read "#5b6cff".
enum ColorName {
    static func of(hex: String) -> String {
        guard let rgb = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) else { return "Color" }
        return of(r: Double((rgb >> 16) & 0xff) / 255, g: Double((rgb >> 8) & 0xff) / 255, b: Double(rgb & 0xff) / 255)
    }

    static func of(r: Double, g: Double, b: Double) -> String {
        let high = max(r, g, b), low = min(r, g, b)
        let saturation = high == 0 ? 0 : (high - low) / high
        if high < 0.15 { return "black" }
        if saturation < 0.15 {
            return high < 0.2 ? "black" : high > 0.9 ? "white" : high < 0.45 ? "dark gray" : "gray"
        }
        var hue: Double
        let span = high - low
        if high == r { hue = (g - b) / span } else if high == g { hue = 2 + (b - r) / span } else { hue = 4 + (r - g) / span }
        hue = (hue * 60).truncatingRemainder(dividingBy: 360)
        if hue < 0 { hue += 360 }
        let name: String
        switch hue {
        case ..<15, 345...: name = "red"
        case ..<42: name = "orange"
        case ..<70: name = "yellow"
        case ..<160: name = "green"
        case ..<195: name = "teal"
        case ..<228: name = "blue"
        case ..<258: name = "indigo"
        case ..<292: name = "purple"
        default: name = "pink"
        }
        return high < 0.6 ? "dark \(name)" : name
    }
}

extension ColorName {
    /// A doodle ink by its palette index.
    static func ink(_ index: Int) -> String {
        let palette = DoodleDrawing.palette
        guard palette.indices.contains(index) else { return "Ink" }
        let c = palette[index]
        return of(r: Double(c.r) / 255, g: Double(c.g) / 255, b: Double(c.b) / 255)
    }
}
