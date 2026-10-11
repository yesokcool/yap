import SwiftUI
import YapCore

/// A picture in a message, Win98-style: at most 360 pt on its long side, in a
/// sunken frame like a GIF, sized from the message (no layout jump). Tap for
/// full size.
struct PictureView: View {
    @Environment(AppModel.self) private var model
    var image: ImageRef
    private let maxSide: CGFloat = 360
    @State private var viewing = false

    var body: some View {
        let size = image.fittedSize(maxWidth: maxSide, maxHeight: maxSide)
        ZStack {
            Theme.window
            if let endpoint = model.client.endpoint, let picture = model.images.picture(for: endpoint.mediaURL(image.id)) {
                Image(platformImage: picture).resizable().scaledToFill()
            } else {
                Image(systemName: "photo").foregroundStyle(Theme.grayText)
            }
        }
        .frame(width: size.width, height: size.height)
        .clipped()
        .padding(2)
        .background(Theme.window)
        .overlay(Bevel(style: .sunken))
        .contentShape(.rect)
        .onTapGesture { viewing = true }
        .accessibilityLabel("Picture")
        .accessibilityHint("Opens it full size")
        .accessibilityAddTraits(.isButton)
        .sheet(isPresented: $viewing) { PictureViewer98(image: image) }
    }
}

/// An encrypted Personal-space picture: fetched and opened on the device.
struct SealedPictureView: View {
    @Environment(AppModel.self) private var model
    var image: Vault.SealedImage
    private let maxSide: CGFloat = 360

    var body: some View {
        let size = image.ref.fittedSize(maxWidth: maxSide, maxHeight: maxSide)
        let client = model.client
        ZStack {
            Theme.window
            if let endpoint = client.endpoint,
               let picture = model.images.sealedPicture(for: endpoint.mediaURL(image.id), open: { try client.openBlob($0) }) {
                Image(platformImage: picture).resizable().scaledToFill()
            } else {
                Image(systemName: "lock.fill").foregroundStyle(Theme.grayText)
            }
        }
        .frame(width: size.width, height: size.height)
        .clipped()
        .padding(2)
        .background(Theme.window)
        .overlay(Bevel(style: .sunken))
        .accessibilityLabel("Encrypted picture")
    }
}

/// Full size, in a Win98 window: the picture in a well, scrollable when big.
/// Pinch (or double-click / double-tap) to zoom, up to 4×; no animation.
struct PictureViewer98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    var image: ImageRef
    @State private var zoom: CGFloat = 1
    @GestureState private var pinch: CGFloat = 1

    var body: some View {
        let url = model.client.endpoint?.mediaURL(image.id)
        NavigationStack {
            VStack(spacing: 6) {
                GeometryReader { geo in
                    let fit = image.fittedSize(maxWidth: geo.size.width - 8, maxHeight: geo.size.height - 8)
                    let scale = Self.clamp(zoom * pinch)
                    ScrollView([.horizontal, .vertical]) {
                        AsyncImage(url: url) { loaded in
                            loaded.resizable().scaledToFit()
                        } placeholder: {
                            ProgressView()
                        }
                        .frame(width: fit.width * scale, height: fit.height * scale)
                        .frame(minWidth: geo.size.width - 8, minHeight: geo.size.height - 8)
                        .onTapGesture(count: 2) { zoom = zoom > 1 ? 1 : 2.5 }
                    }
                    .gesture(MagnifyGesture()
                        .updating($pinch) { value, state, _ in state = value.magnification }
                        .onEnded { value in zoom = Self.clamp(zoom * value.magnification) })
                    .padding(4)
                }
                .well()
                HStack(spacing: 2) {
                    StatusPanel(text: zoom > 1 ? "\(image.width) × \(image.height) · \(Int((zoom * 100).rounded()))%" : "\(image.width) × \(image.height)")
                    if let url {
                        Button { openURL(url) } label: {
                            StatusPanel(text: "Open in browser", systemImage: "link", color: Theme.link)
                        }
                        .buttonStyle(.plain)
                        #if os(macOS)
                        ShareLink(item: url) {
                            StatusPanel(text: "Share…", systemImage: "square.and.arrow.up", color: Theme.link)
                        }
                        .buttonStyle(.plain)
                        #endif
                    }
                }
            }
            .padding(6)
            .background(Theme.face)
            .win98Sheet("Picture") {
                CloseSheetButton { dismiss() }
            }
            #if os(iOS)
            .toolbar {
                if let url {
                    ToolbarItem(placement: .primaryAction) {
                        ShareLink(item: url) { Image(systemName: "square.and.arrow.up") }
                            .buttonStyle(.win98Compact)
                            .appScheme()
                    }
                    .win98Item()
                }
            }
            #endif
        }
        .presentationDetents([.large])
        #if os(macOS)
        .frame(minWidth: 520, minHeight: 420)
        #endif
    }
    /// 1× (fits the window) to 4×.
    static func clamp(_ scale: CGFloat) -> CGFloat { min(4, max(1, scale)) }
}
