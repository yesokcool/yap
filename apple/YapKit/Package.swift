// swift-tools-version: 6.2
//
// YapKit: everything in the Yap Apple client that isn't a view.
//  - YapCore: the wire protocol (PROTOCOL.md), the WebSocket session, the
//    client state store, emoji/format/GIF helpers. No WebRTC.
//  - YapVoice: the WebRTC voice mesh (stasel/WebRTC), Opus SDP tuning,
//    audio session handling.
//  - YapScreenLink: how the iPhone/iPad screen-broadcast extension hands
//    its pictures to the app (a Unix socket in the App Group). No WebRTC:
//    the extension links only this.
// The app target (../Yap) is SwiftUI on top of these. Tests run on the Mac
// with `swift test` (see ../README.md); the integration and voice tests talk
// to a local dev server and skip themselves when it isn't configured.
import PackageDescription

let package = Package(
    name: "YapKit",
    platforms: [.iOS(.v26), .macOS(.v26)],
    products: [
        .library(name: "YapCore", targets: ["YapCore"]),
        .library(name: "YapVoice", targets: ["YapVoice"]),
        .library(name: "YapScreenLink", targets: ["YapScreenLink"]),
    ],
    dependencies: [
        .package(url: "https://github.com/stasel/WebRTC.git", from: "154.0.0"),
    ],
    targets: [
        .target(
            name: "YapCore",
            resources: [.copy("Resources/emoji-data.json")]
        ),
        .target(
            name: "YapVoice",
            dependencies: ["YapCore", "YapScreenLink", .product(name: "WebRTC", package: "WebRTC")]
        ),
        .target(name: "YapScreenLink"),
        .testTarget(name: "YapCoreTests", dependencies: ["YapCore"]),
        .testTarget(name: "YapVoiceTests", dependencies: ["YapVoice", "YapCore", "YapScreenLink"]),
        .testTarget(name: "YapScreenLinkTests", dependencies: ["YapScreenLink"]),
    ],
    swiftLanguageModes: [.v6]
)
