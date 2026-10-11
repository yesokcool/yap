import Foundation
import Testing
@testable import Yap
import YapCore

/// The extras' app-side rules: the quick switcher's ranking, the "New
/// since" wording, the stored sound settings and their defaults.
@Suite("Extras in the app")
struct ExtrasAppTests {
    @Test func switcherRanksExactThenPrefixThenWordsThenLetters() {
        #expect(Switcher.score("general", "general") == 0)
        #expect(Switcher.score("gen", "general") == 1)
        #expect(Switcher.score("board", "game board") == 2)
        #expect(Switcher.score("ame", "game board") == 3)
        #expect(Switcher.score("gb", "game board") == 4, "letters in order")
        #expect(Switcher.score("zz", "general") == nil)
    }

    @Test func sinceYouLeftWording() {
        let now = Date()
        #expect(SinceYouLeft.label(nil) == "New messages")
        #expect(SinceYouLeft.label(0) == "New messages")
        let earlier = now.addingTimeInterval(-60)
        if Calendar.current.isDateInToday(earlier) {
            #expect(SinceYouLeft.label(earlier.yapMillis) == "New since \(earlier.formatted(date: .omitted, time: .shortened))")
        }
        let lastWeek = now.addingTimeInterval(-20 * 86_400)
        #expect(SinceYouLeft.label(lastWeek.yapMillis).hasPrefix("New since "))
    }

    @Test func soundSettingsDefaults() {
        // Stored per device: keys are on people's phones now.
        #expect(SoundSettings.soundsKey == "yap.sounds")
        #expect(SoundSettings.volumeKey == "yap.soundboardVolume")
        #expect(SoundSettings.mutedKey == "yap.soundboardMuted")
        let defaults = UserDefaults.standard
        let saved = (defaults.object(forKey: SoundSettings.volumeKey), defaults.object(forKey: SoundSettings.soundsKey))
        defaults.removeObject(forKey: SoundSettings.volumeKey)
        defaults.removeObject(forKey: SoundSettings.soundsKey)
        #expect(SoundSettings.volume == 0.5, "50 % unless changed")
        #expect(SoundSettings.sounds, "Sounds on unless turned off")
        if let volume = saved.0 { defaults.set(volume, forKey: SoundSettings.volumeKey) }
        if let sounds = saved.1 { defaults.set(sounds, forKey: SoundSettings.soundsKey) }
    }

    @Test func awayChoicesStayWithinAWeek() {
        // The server refuses an away message ending more than a week out.
        let now = Date()
        for choice in AwayEnd.allCases {
            if let until = choice.until(now: now) {
                #expect(until > now && until.timeIntervalSince(now) <= 7 * 86_400)
            }
        }
        #expect(AwayPresets.choices(now: now).first == .back, "Until my next message comes first")
    }

    /// The web's and the desktop app's four choices, in their words.
    @Test func awayChoicesMatchTheWeb() throws {
        #expect(AwayPresets.choices().map(\.title) == ["Until my next message", "For 1 hour", "For 3 hours", "Until tomorrow morning"])
        let morning = try #require(AwayEnd.morning.until(now: Date()))
        #expect(Calendar.current.component(.hour, from: morning) == 9)
    }
}

@MainActor
@Suite("Color names")
struct ColorNameTests {
    @Test func swatchesHaveNames() {
        #expect(ColorName.of(hex: "#e5484d") == "red")
        #expect(ColorName.of(hex: "#000080") == "dark indigo" || ColorName.of(hex: "#000080") == "dark blue")
        #expect(ColorName.of(hex: "#30a46c") == "green")
        #expect(ColorName.of(hex: "#8d8d8d") == "gray")
        #expect(ColorName.of(hex: "#d6409f") == "pink")
        #expect(ColorName.of(hex: "#0090ff") == "blue")
        #expect(ColorName.ink(0) == "white" && ColorName.ink(1) == "black")
        // Every color offered gets a name, never a hex code.
        for hex in Theme.personColors {
            #expect(!ColorName.of(hex: hex).contains("#"))
        }
    }
}
