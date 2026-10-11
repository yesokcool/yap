import Foundation
import SwiftUI
import Testing
@testable import Yap

/// One look: Windows 98, in light or dark. Earlier versions stored a skin
/// per device (`yap.skin`: win98 / apple / arch); whatever it says, the app
/// opens in Windows 98, silently, and keeps the light/dark choice.
@Suite("One look")
struct LookTests {
    private func defaults() -> UserDefaults {
        let name = "yap-look-test-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return defaults
    }

    @Test(arguments: ["apple", "arch", "win98", "aqua", ""])
    func aRemovedSkinLoadsAsWindows98(stored: String) {
        let defaults = defaults()
        defaults.set(stored, forKey: "yap.skin")
        defaults.set(Appearance.standard.rawValue, forKey: Appearance.storageKey)
        Look.settle(defaults)
        #expect(defaults.object(forKey: "yap.skin") == nil, "the old choice is forgotten, so nothing can pick another look")
        #expect(defaults.string(forKey: Appearance.storageKey) == "standard", "light stays light")
        #expect(Appearance(rawValue: defaults.string(forKey: Appearance.storageKey) ?? "") == .standard)
    }

    @Test func darkAndSystemStayToo() {
        for appearance in Appearance.allCases {
            let defaults = defaults()
            defaults.set("arch", forKey: Look.retiredSkinKey)
            defaults.set(appearance.rawValue, forKey: Appearance.storageKey)
            Look.settle(defaults)
            #expect(defaults.string(forKey: Appearance.storageKey) == appearance.rawValue)
        }
    }

    @Test func nothingStoredIsFine() {
        let defaults = defaults()
        Look.settle(defaults)
        #expect(defaults.object(forKey: Look.retiredSkinKey) == nil)
        #expect(defaults.object(forKey: Appearance.storageKey) == nil, "Charcoal stays the default")
    }

    @Test func appearanceKeysAreStable() {
        #expect(Appearance.storageKey == "yap.appearance")
        #expect(Appearance.allCases.map(\.rawValue) == ["charcoal", "standard", "system"])
        #expect(Appearance.charcoal.title == "Charcoal (dark)")
        #expect(Appearance.standard.title == "Windows Standard (light)")
    }

    #if os(iOS)
    @Test @MainActor func statusBarIsWhite() {
        // Every screen has a navy title bar (or the teal desktop) on top.
        #expect(Look.statusBarStyle == .lightContent)
    }
    #endif
}
