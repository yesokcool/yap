import SwiftUI
import Testing
@testable import Yap

/// A jump that missed walks toward its message a screenful at a time.
@Suite("Jumping to a message")
struct JumpTests {
    @MainActor
    @Test func stepsTowardTheRowPastTheScreensEdge() {
        let rows = (0..<10).map { "msg:m\($0)" }
        let shown: Set = ["msg:m6", "msg:m7", "msg:m8", "day:2026-10-7"]
        // Above: the message just over the top edge goes to the bottom edge.
        let up = ChannelController.step(toward: "msg:m2", in: rows, onScreen: shown)
        #expect(up?.row == "msg:m5" && up?.anchor == .bottom && up?.gap == 4)
        // Below: the message just under the bottom edge goes to the top edge.
        let down = ChannelController.step(toward: "msg:m9", in: rows, onScreen: shown)
        #expect(down?.row == "msg:m9" && down?.anchor == .top && down?.gap == 1)
        // On screen already, not loaded, or nothing on screen: no step.
        #expect(ChannelController.step(toward: "msg:m7", in: rows, onScreen: shown) == nil)
        #expect(ChannelController.step(toward: "msg:zz", in: rows, onScreen: shown) == nil)
        #expect(ChannelController.step(toward: "msg:m2", in: rows, onScreen: []) == nil)
    }
}
