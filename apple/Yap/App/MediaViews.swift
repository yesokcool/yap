import AVFoundation
import ImageIO
import SwiftUI
import YapCore

// Media views the chat uses: looping GIF video, animated images, the
// staged picture's thumbnail.

#if os(iOS)
/// AVPlayerLayer + AVPlayerLooper, muted.
struct LoopingVideo: UIViewRepresentable {
    var url: URL

    func makeUIView(context: Context) -> PlayerView {
        let view = PlayerView()
        view.play(url)
        return view
    }

    func updateUIView(_ view: PlayerView, context: Context) {}

    static func dismantleUIView(_ view: PlayerView, coordinator: ()) {
        view.stop()
    }

    final class PlayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        private var looper: AVPlayerLooper?
        private var player: AVQueuePlayer?

        func play(_ url: URL) {
            let item = AVPlayerItem(url: url)
            let player = AVQueuePlayer()
            player.isMuted = true
            player.preventsDisplaySleepDuringVideoPlayback = false
            looper = AVPlayerLooper(player: player, templateItem: item)
            (layer as! AVPlayerLayer).player = player
            (layer as! AVPlayerLayer).videoGravity = .resizeAspectFill
            self.player = player
            player.play()
        }

        func stop() {
            player?.pause()
            looper = nil
            player = nil
        }
    }
}
#else
struct LoopingVideo: NSViewRepresentable {
    var url: URL

    func makeNSView(context: Context) -> PlayerView {
        let view = PlayerView()
        view.play(url)
        return view
    }

    func updateNSView(_ view: PlayerView, context: Context) {}

    static func dismantleNSView(_ view: PlayerView, coordinator: ()) {
        view.stop()
    }

    final class PlayerView: NSView {
        private var looper: AVPlayerLooper?
        private var player: AVQueuePlayer?
        private let playerLayer = AVPlayerLayer()

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            layer = CALayer()
            playerLayer.videoGravity = .resizeAspectFill
            layer?.addSublayer(playerLayer)
        }

        required init?(coder: NSCoder) { fatalError() }

        override func layout() {
            super.layout()
            playerLayer.frame = bounds
        }

        func play(_ url: URL) {
            let player = AVQueuePlayer()
            player.isMuted = true
            looper = AVPlayerLooper(player: player, templateItem: AVPlayerItem(url: url))
            playerLayer.player = player
            self.player = player
            player.play()
        }

        func stop() {
            player?.pause()
            looper = nil
            player = nil
        }
    }
}
#endif

/// Animated WebP/GIF from a URL via ImageIO (picker tiles, GIF messages
/// without mp4). Downloads with KLIPY's no-cache session.
/// Until the first frame: `placeholder` (clear, so the caller's still or
/// background shows through).
struct AnimatedRemoteImage: View {
    var url: URL
    var placeholder: Color = .clear
    @State private var model = AnimatedImageModel()

    var body: some View {
        Group {
            if let frame = model.frame {
                Image(decorative: frame, scale: 1).resizable().scaledToFill()
            } else {
                placeholder
            }
        }
        .task(id: url) { await model.load(url) }
        .onDisappear { model.stop() }
    }
}

@MainActor
@Observable
final class AnimatedImageModel {
    var frame: CGImage?
    @ObservationIgnored private var stopper: AnimationStopper?

    func load(_ url: URL) async {
        stop()
        guard let (data, _) = try? await KlipyClient.mediaSession.data(from: url) else { return }
        let stopper = AnimationStopper()
        self.stopper = stopper
        Self.animate(data, stopper: stopper) { [weak self] image in
            self?.frame = image
        }
    }

    func stop() {
        stopper?.stop()
        stopper = nil
    }

    /// ImageIO calls the block on its own schedule; it's nonisolated here and
    /// hops to the main actor for each frame (Swift 6 would trap otherwise).
    nonisolated private static func animate(_ data: Data, stopper: AnimationStopper, onFrame: @escaping @MainActor @Sendable (CGImage) -> Void) {
        let status = CGAnimateImageDataWithBlock(data as CFData, nil) { @Sendable _, image, stop in
            if stopper.isStopped {
                stop.pointee = true
                return
            }
            let frame = image
            DispatchQueue.main.async { MainActor.assumeIsolated { onFrame(frame) } }
        }
        if status != noErr {
            // Not animatable: show the first frame.
            if let source = CGImageSourceCreateWithData(data as CFData, nil), let first = CGImageSourceCreateImageAtIndex(source, 0, nil) {
                let frame = first
                DispatchQueue.main.async { MainActor.assumeIsolated { onFrame(frame) } }
            }
        }
    }
}

/// Read from ImageIO's thread: nonisolated (the app target defaults to MainActor).
nonisolated final class AnimationStopper: @unchecked Sendable {
    private let lock = NSLock()
    private var stopped = false
    var isStopped: Bool { lock.withLock { stopped } }
    func stop() { lock.withLock { stopped = true } }
}

/// The picture, or the GIF's first frame.
struct StagedThumbnail: View {
    var staged: StagedAttachment

    var body: some View {
        if let preview = staged.preview {
            Image(platformImage: preview).resizable().scaledToFill()
        } else if let still = staged.gif?.stillURL {
            AsyncImage(url: still) { $0.resizable().scaledToFill() } placeholder: { Color.secondary.opacity(0.2) }
        } else if let image = staged.gif?.imageURL {
            AnimatedRemoteImage(url: image, placeholder: Color.secondary.opacity(0.2))
        } else {
            Image(systemName: "photo")
        }
    }
}
