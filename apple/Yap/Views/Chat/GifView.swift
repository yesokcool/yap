import SwiftUI
import YapCore

/// A GIF message: KLIPY's mp4 as a muted, looping video (much lighter than a
/// .gif), sized from the message's width/height up front (no layout jump),
/// with `still` as the poster. Plays only while on screen; with Reduce
/// Motion it waits on the still until tapped. Media loads straight from
/// KLIPY's URLs, unmodified.
struct GifView: View {
    var gif: Gif
    private let maxSide: CGFloat = 240
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var visible = false
    @State private var tappedToPlay = false

    var body: some View {
        let size = gif.fittedSize(maxSide: maxSide)
        ZStack {
            Theme.window
            if let still = gif.stillURL {
                AsyncImage(url: still) { image in
                    image.resizable().scaledToFill()
                } placeholder: {
                    Image(systemName: "photo").foregroundStyle(Theme.grayText)
                }
            }
            if visible, !reduceMotion || tappedToPlay {
                if let video = gif.videoURL {
                    LoopingVideo(url: video)
                } else if let image = gif.imageURL {
                    AnimatedRemoteImage(url: image)
                }
            }
            if reduceMotion, !tappedToPlay {
                Tooltip98(text: "GIF — tap to play")
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .padding(6)
            }
        }
        .frame(width: size.width, height: size.height)
        .clipped()
        .padding(2)
        .background(Theme.window)
        .overlay(Bevel(style: .sunken))
        .onAppear { visible = true }
        .onDisappear { visible = false }
        .onTapGesture { tappedToPlay.toggle() }
        .accessibilityLabel(gif.title.isEmpty ? "GIF" : "GIF: \(gif.title)")
    }
}
