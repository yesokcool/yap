import Foundation
import Testing
@testable import Yap

/// Pins the identifiers the Dev sandbox build derives from its bundle id.
/// Run under the "Yap" scheme (Debug): the App Store identity.
@Suite("App identity")
struct AppIdentityTests {
    @Test func productionIdentity() {
        if AppIdentity.isDevBuild {
            #expect(AppIdentity.bundleIdentifier == AppIdentity.productionBundleIdentifier + ".dev")
            #expect(AppIdentity.buildFlavor == "Dev")
        } else {
            #expect(AppIdentity.bundleIdentifier == AppIdentity.productionBundleIdentifier)
            #expect(AppIdentity.buildFlavor == nil)
        }
        #expect(AppIdentity.keychainService == AppIdentity.bundleIdentifier)
        #expect(!AppIdentity.productionBundleIdentifier.contains("$("), "the Info.plist expands YAP_BUNDLE_ID")
    }

    /// iPhone and iPad: the App Group shared with the broadcast extension
    /// follows the bundle id (the Mac shares without one).
    @Test func screenSharingAppGroup() {
        #if os(iOS)
        #expect(AppIdentity.appGroup == "group." + AppIdentity.bundleIdentifier, "\(AppIdentity.appGroup ?? "none")")
        #expect(BroadcastPicker.extensionIdentifier == AppIdentity.bundleIdentifier + ".broadcast")
        #else
        #expect(AppIdentity.appGroup == nil)
        #endif
    }

    /// The Win98 look uses Tahoma or the system font; no pixel fonts ship.
    @Test func noBundledPixelFonts() {
        #expect(Bundle.main.urls(forResourcesWithExtension: "woff2", subdirectory: nil)?.isEmpty ?? true)
        #expect(Theme.radius == 0, "square corners")
    }
}
