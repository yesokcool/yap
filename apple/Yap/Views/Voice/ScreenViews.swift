import AVKit
import SwiftUI
import YapCore
import YapVoice

// Watching someone's screen (iPhone, iPad, Mac) and sharing yours (Mac),
// in Windows 98 dress. PROTOCOL.md › Screen sharing; the logic is
// `VoiceController` / `AppModel+ScreenSharing`.

/// Someone's screen, big in the main area: fit to the window (letterboxed,
/// never cropped or stretched), with Pop out (picture-in-picture), Full
/// screen (or double-click / double-tap the picture), Stop watching and the
/// share's own sound volume. The line over the picture says what's going on:
/// connecting, paused, stopped (then it closes itself).
struct ScreenViewer98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let watch = model.voice.watch
        let name = watch.flatMap { model.client.user($0.uid)?.name } ?? "Someone"
        let full = model.screenFullScreen
        Group {
            #if os(macOS)
            if full {
                stage(watch: watch, name: name, full: true)
            } else {
                Window98(title: ScreenText.title(name), systemImage: "display") {
                    VStack(spacing: 4) {
                        stage(watch: watch, name: name, full: false).well()
                        controls(watch: watch)
                    }
                }
            }
            #else
            VStack(spacing: 0) {
                stage(watch: watch, name: name, full: full)
                if !full {
                    controls(watch: watch)
                        .padding(6)
                        .background(Theme.face)
                }
            }
            .background(full ? Color.black : Theme.face)
            .modifier(FullScreenChrome(full: full, title: ScreenText.title(name)))
            #endif
        }
        .onChange(of: watch == nil) { _, gone in
            // It closed itself (they stopped 2 s ago), or watching stopped elsewhere.
            if gone { model.closeScreen() }
        }
        .task {
            // Picture-in-picture needs the layer in a window first.
            try? await Task.sleep(for: .milliseconds(300))
            model.screenPiP.prepare()
        }
        .onDisappear { model.screenViewerGone() }
    }

    /// The picture and the line over it.
    private func stage(watch: ScreenWatch?, name: String, full: Bool) -> some View {
        ZStack {
            Group {
                Color.black
                ScreenLayerView(renderer: model.voice.screenRenderer)
                    .accessibilityLabel(ScreenText.title(name))
            }
            #if os(iOS)
            // Full screen: the picture uses the whole glass; the way out stays clear of the edges.
            .ignoresSafeArea(edges: full ? .all : [])
            #endif
            if let status = watch?.status(name: name) {
                Text(status)
                    .font(Theme.chrome)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                    .padding(12)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            #if os(iOS)
            if full {
                // A way out without the bars (double-tap works too).
                Button(ScreenText.exitFullScreen, systemImage: "arrow.down.right.and.arrow.up.left") {
                    model.setScreenFullScreen(false)
                }
                .labelStyle(.iconOnly)
                .buttonStyle(.win98Compact)
                .appScheme()
                .padding(10)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
            }
            #endif
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(.rect)
        .onTapGesture(count: 2) { model.setScreenFullScreen(!model.screenFullScreen) }
        .accessibilityAction(named: Text(full ? ScreenText.exitFullScreen : ScreenText.fullScreen)) {
            model.setScreenFullScreen(!full)
        }
    }

    /// Pop out · Full screen · Stop watching, and the share's sound (on a
    /// narrow screen, the sound gets its own row).
    private func controls(watch: ScreenWatch?) -> some View {
        let buttons = HStack(spacing: 6) {
            Button(ScreenText.popOut) { model.screenPiP.start() }
                .buttonStyle(.win98)
                .disabled(!model.screenPiP.isPossible)
                .help(ScreenText.popOutHelp)
            Button(ScreenText.fullScreen) { model.setScreenFullScreen(true) }
                .buttonStyle(.win98)
            Button(ScreenText.stopWatching) { model.closeScreen() }
                .buttonStyle(.win98)
        }
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) {
                buttons
                Spacer(minLength: 8)
                if let uid = watch?.uid { ScreenSoundSlider(uid: uid).fixedSize() }
            }
            VStack(alignment: .leading, spacing: 4) {
                buttons
                if let uid = watch?.uid { ScreenSoundSlider(uid: uid) }
            }
        }
    }
}

/// The share's sound, 0–100 %, remembered per person on this device (their
/// voice has its own volume).
struct ScreenSoundSlider: View {
    @Environment(AppModel.self) private var model
    var uid: String

    var body: some View {
        let volume = Binding(
            get: { model.voice.screenVolume(for: uid) },
            set: { model.voice.setScreenVolume($0, for: uid) }
        )
        HStack(spacing: 4) {
            Image(systemName: volume.wrappedValue == 0 ? "speaker.slash.fill" : "speaker.wave.2.fill")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(model.voice.watchedHasSound ? Theme.text : Theme.grayText)
                .accessibilityHidden(true)
            Slider(value: volume, in: 0...1, step: 0.05)
                .frame(minWidth: 100, idealWidth: 140, maxWidth: 220)
                .accessibilityLabel("Screen sound")
                .accessibilityValue("\(Int((volume.wrappedValue * 100).rounded()))%")
            Text("\(Int((volume.wrappedValue * 100).rounded()))%")
                .font(Theme.small.monospacedDigit())
                .foregroundStyle(Theme.text)
                .frame(width: 34, alignment: .trailing)
                .accessibilityHidden(true)
        }
        .help(ScreenText.soundHelp)
    }
}

#if os(iOS)
/// iPhone: a navy title bar, or nothing at all in full screen (no status
/// bar, no call bar; turn the phone for a wide share).
private struct FullScreenChrome: ViewModifier {
    var full: Bool
    var title: String

    func body(content: Content) -> some View {
        if full {
            content
                .toolbar(.hidden, for: .navigationBar)
                .statusBarHidden(true)
                .persistentSystemOverlays(.hidden)
        } else {
            content
                .modifier(InCallBar98())
                .win98NavigationBar(title)
        }
    }
}
#endif

// MARK: - The picture

/// Hosts the renderer's display layer (one view at a time shows it).
#if os(iOS)
struct ScreenLayerView: UIViewRepresentable {
    var renderer: ScreenRenderer

    func makeUIView(context: Context) -> LayerHost {
        let view = LayerHost()
        view.show(renderer.displayLayer)
        return view
    }

    func updateUIView(_ view: LayerHost, context: Context) {
        view.show(renderer.displayLayer)
    }

    final class LayerHost: UIView {
        private weak var shown: CALayer?

        func show(_ layer: CALayer) {
            backgroundColor = .black
            guard shown !== layer else { return }
            layer.removeFromSuperlayer()
            self.layer.addSublayer(layer)
            shown = layer
            setNeedsLayout()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            shown?.frame = bounds
            CATransaction.commit()
        }
    }
}
#else
struct ScreenLayerView: NSViewRepresentable {
    var renderer: ScreenRenderer

    func makeNSView(context: Context) -> LayerHost {
        let view = LayerHost()
        view.show(renderer.displayLayer)
        return view
    }

    func updateNSView(_ view: LayerHost, context: Context) {
        view.show(renderer.displayLayer)
    }

    final class LayerHost: NSView {
        private weak var shown: CALayer?

        func show(_ layer: CALayer) {
            wantsLayer = true
            self.layer?.backgroundColor = .black
            guard shown !== layer else { return }
            layer.removeFromSuperlayer()
            self.layer?.addSublayer(layer)
            shown = layer
            needsLayout = true
        }

        override func layout() {
            super.layout()
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            shown?.frame = bounds
            CATransaction.commit()
        }
    }
}
#endif

/// "Pop out": picture-in-picture from the renderer's layer (AVKit's
/// sample-buffer source, iOS and macOS). It keeps playing while you use the
/// rest of Yap; on iPhone it also starts by itself when you leave the app
/// while watching. Live: no seeking, no pause.
@MainActor
@Observable
final class ScreenPictureInPicture: NSObject {
    private(set) var isActive = false
    private(set) var isPossible = false
    @ObservationIgnored var renderer: ScreenRenderer?
    /// The PiP window's "back to the app" button.
    @ObservationIgnored var onRestore: (() -> Void)?
    /// The PiP window was closed.
    @ObservationIgnored var onClosed: (() -> Void)?
    @ObservationIgnored private var controller: AVPictureInPictureController?
    @ObservationIgnored private var possible: NSKeyValueObservation?
    @ObservationIgnored private var bridge: PlaybackBridge?

    /// Make the controller once the layer is in a window.
    func prepare() {
        guard controller == nil, AVPictureInPictureController.isPictureInPictureSupported(), let layer = renderer?.displayLayer else { return }
        let bridge = PlaybackBridge(owner: self)
        self.bridge = bridge
        let source = AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer: layer, playbackDelegate: bridge)
        let controller = AVPictureInPictureController(contentSource: source)
        controller.delegate = bridge
        controller.requiresLinearPlayback = true
        #if os(iOS)
        controller.canStartPictureInPictureAutomaticallyFromInline = true
        #endif
        possible = controller.observe(\.isPictureInPicturePossible, options: [.initial, .new]) { [weak self] controller, _ in
            let value = controller.isPictureInPicturePossible
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.isPossible = value } }
        }
        self.controller = controller
    }

    func start() {
        prepare()
        controller?.startPictureInPicture()
    }

    func stop() {
        if controller?.isPictureInPictureActive == true { controller?.stopPictureInPicture() }
    }

    fileprivate func changed(active: Bool) {
        isActive = active
        if !active { onClosed?() }
    }

    fileprivate func restore() { onRestore?() }

    /// AVKit's callbacks (nonisolated) → the main actor.
    private final class PlaybackBridge: NSObject, AVPictureInPictureControllerDelegate, AVPictureInPictureSampleBufferPlaybackDelegate, @unchecked Sendable {
        nonisolated(unsafe) weak var owner: ScreenPictureInPicture?

        init(owner: ScreenPictureInPicture) {
            self.owner = owner
        }

        private func main(_ body: @escaping @MainActor (ScreenPictureInPicture) -> Void) {
            DispatchQueue.main.async { [weak self] in
                MainActor.assumeIsolated { if let owner = self?.owner { body(owner) } }
            }
        }

        func pictureInPictureControllerDidStartPictureInPicture(_ controller: AVPictureInPictureController) {
            main { $0.changed(active: true) }
        }

        func pictureInPictureControllerDidStopPictureInPicture(_ controller: AVPictureInPictureController) {
            main { $0.changed(active: false) }
        }

        func pictureInPictureController(_ controller: AVPictureInPictureController,
                                        restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completion: @escaping (Bool) -> Void) {
            main { $0.restore() }
            completion(true)
        }

        // Live video: always "playing", no timeline.
        func pictureInPictureController(_ controller: AVPictureInPictureController, setPlaying playing: Bool) {}
        func pictureInPictureControllerTimeRangeForPlayback(_ controller: AVPictureInPictureController) -> CMTimeRange {
            CMTimeRange(start: .negativeInfinity, duration: .positiveInfinity)
        }
        func pictureInPictureControllerIsPlaybackPaused(_ controller: AVPictureInPictureController) -> Bool { false }
        func pictureInPictureController(_ controller: AVPictureInPictureController, didTransitionToRenderSize newRenderSize: CMVideoDimensions) {}
        func pictureInPictureController(_ controller: AVPictureInPictureController, skipByInterval skipInterval: CMTime,
                                        completion completionHandler: @escaping () -> Void) {
            completionHandler()
        }
    }
}

// MARK: - Sharing

/// The call sheet's screen group: Share screen, or while sharing the
/// quality (Smooth / Sharp / Max, changeable without stopping), who's
/// watching, Share something else (Mac) and Stop sharing. Picture only (no
/// Share sound: see `MacScreenFeeder`). iPhone and iPad share through iOS's
/// broadcast sheet (`BroadcastPicker`).
struct ScreenShareGroup98: View {
    var sharing: ScreenQuality?
    var quality: ScreenQuality
    var watching: String
    var canChange: Bool
    var share: () -> Void
    var setQuality: (ScreenQuality) -> Void
    var change: () -> Void
    var stop: () -> Void

    var body: some View {
        GroupBox98(title: "Screen") {
            HStack(spacing: 12) {
                ForEach(ScreenQuality.allCases) { option in
                    Radio98(title: option.title, isOn: (sharing ?? quality) == option) { setQuality(option) }
                }
            }
            .help("Smooth: 1080p, 60 fps (games, video). Sharp: 1440p, 30 fps (text, code). Max: up to 4K, 60 fps (one or two viewers).")
            if sharing != nil {
                Text(watching)
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                    .accessibilityAddTraits(.updatesFrequently)
                HStack(spacing: 6) {
                    if canChange {
                        Button(ScreenText.changeContent, action: change).buttonStyle(.win98)
                    }
                    Button(ScreenText.stopSharing, action: stop).buttonStyle(.win98)
                }
            } else {
                Button(ScreenText.share, systemImage: "display", action: share)
                    .buttonStyle(.win98)
                    .labelStyle(.titleAndIcon)
                    #if os(iOS)
                    // iOS's broadcast picker, which the button opens.
                    .background(alignment: .center) { BroadcastPickerHost().frame(width: 1, height: 1) }
                    #endif
                Text(ScreenText.shareHint)
                    .font(Theme.small)
                    .foregroundStyle(Theme.grayText)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

extension ScreenShareGroup98 {
    /// Wired to the app.
    @MainActor
    init(model: AppModel) {
        let voice = model.voice
        self.init(sharing: voice.sharing, quality: voice.settings.shareQuality, watching: model.screenWatchersText,
                  canChange: voice.canChangeSharedContent,
                  share: { model.shareScreen() },
                  setQuality: { voice.setShareQuality($0) },
                  change: { voice.changeSharedContent() },
                  stop: { voice.stopSharing() })
    }
}

/// Under the call bar while sharing (Mac): "Sharing · Smooth", who's
/// watching, Stop sharing.
struct SharingStrip98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let quality = model.voice.sharing {
            HStack(spacing: 6) {
                Image(systemName: "display").font(.system(size: 11, weight: .bold)).foregroundStyle(Theme.danger)
                VStack(alignment: .leading, spacing: 0) {
                    Text(ScreenText.sharing(quality)).font(Theme.chromeBold).foregroundStyle(Theme.text).lineLimit(1)
                    Text(model.screenWatchersText).font(Theme.small).foregroundStyle(Theme.hint).lineLimit(1)
                        .help(model.screenWatchersText)
                }
                Spacer(minLength: 4)
                Button(ScreenText.stopSharing) { model.voice.stopSharing() }
                    .buttonStyle(.win98Compact)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .bevel(.raised)
            .accessibilityElement(children: .contain)
        }
    }
}
