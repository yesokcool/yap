import ImageIO
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import YapCore
#if os(macOS)
import AppKit
#endif

/// A GIF or picture waiting in the tray above the composer: nothing goes out
/// the moment it's picked. Whatever's typed is its caption when you press
/// Send; × (or Esc on the Mac) drops it. A picture starts shrinking in the
/// background as soon as it's staged, while the person types; the tray shows
/// its size and a Full res switch.
struct StagedAttachment: Identifiable, Equatable {
    enum Content {
        case gif(Gif, query: String?)
        case picture(PictureDraft)
    }

    let id = UUID()
    let content: Content

    static func == (a: StagedAttachment, b: StagedAttachment) -> Bool { a.id == b.id }

    static func gif(_ gif: Gif, query: String?) -> StagedAttachment {
        StagedAttachment(content: .gif(gif, query: query))
    }

    @MainActor
    static func picture(_ data: Data) -> StagedAttachment {
        StagedAttachment(content: .picture(PictureDraft(data)))
    }

    var title: String {
        if case .gif = content { return "GIF" }
        return "Picture"
    }

    /// "Add a message if you like · 47 KB, compressed" (the size once it's ready).
    @MainActor
    var subtitle: String {
        switch content {
        case let .gif(gif, _): gif.title.isEmpty ? "from KLIPY" : gif.title
        case let .picture(draft): "Add a message if you like · \(draft.sizeText)"
        }
    }

    /// The picture's Full res switch (nil for a GIF).
    var picture: PictureDraft? {
        if case let .picture(draft) = content { return draft }
        return nil
    }

    var gif: Gif? {
        if case let .gif(gif, _) = content { return gif }
        return nil
    }

    var preview: PlatformImage? {
        if case let .picture(draft) = content { return draft.preview }
        return nil
    }

    /// Sends it with `caption`. False if it couldn't (the caller keeps the text).
    @MainActor
    func send(caption: String, in channelId: String, replyTo: Message? = nil, model: AppModel) async -> Bool {
        switch content {
        case let .gif(gif, query):
            // In an encrypted Personal space KLIPY isn't told.
            let sealed = model.client.isSealed(channelId)
            guard model.client.send(gif: gif, caption: caption, in: channelId, replyTo: replyTo) else { return false }
            if !sealed {
                let klipy = model.klipy
                Task { await klipy.reportShare(slug: gif.slug, query: query) }
            }
            return true
        case let .picture(draft):
            let notice = YapClient.Notice(text: "Sending picture…")
            model.client.notice = notice
            do {
                try await model.client.send(picture: try await draft.prepared.value, caption: caption, in: channelId, replyTo: replyTo)
                if model.client.notice?.id == notice.id { model.client.notice = nil }
                return true
            } catch is CancellationError {
                // Signed out while it uploaded: nothing to say, nothing sent.
                return false
            } catch {
                model.client.notice = YapClient.Notice(text: "Couldn't send that picture: \(error.localizedDescription)")
                return false
            }
        }
    }
}

/// A staged picture: the original bytes, a small preview, and the copy that
/// will go out — compressed by default (≤ 1600 px JPEG), or "Full res" (the
/// original when it fits, else the sharpest copy that does). Flipping Full
/// res prepares it again; `sizeText` follows along for the tray.
@MainActor
@Observable
final class PictureDraft {
    let original: Data
    let preview: PlatformImage?
    var full = false {
        didSet { if full != oldValue { start() } }
    }
    private(set) var prepared: Task<PictureEncoder.Prepared, any Error>
    /// The copy that will go out, once it's ready.
    private(set) var ready: PictureEncoder.Prepared?
    private(set) var failure: String?

    init(_ data: Data) {
        original = data
        preview = ImageStore.downsample(data, maxPixel: 240).map {
            #if os(iOS)
            UIImage(cgImage: $0)
            #else
            NSImage(cgImage: $0, size: NSSize(width: $0.width, height: $0.height))
            #endif
        }
        prepared = Self.encode(data, full: false)
        watch()
    }

    /// "47 KB, compressed", "1.2 MB", or "shrinking…" while it works.
    var sizeText: String {
        if let failure { return failure }
        guard let ready else { return full ? "preparing…" : "shrinking…" }
        return PictureEncoder.sizeLabel(ready.data.count) + (full ? "" : ", compressed")
    }

    static let fullResHelp = "Send the original picture instead of a smaller copy (uses more space)"

    private static func encode(_ data: Data, full: Bool) -> Task<PictureEncoder.Prepared, any Error> {
        Task.detached(priority: .userInitiated) { try PictureEncoder.prepare(data, full: full) }
    }

    private func start() {
        prepared.cancel()
        ready = nil
        failure = nil
        prepared = Self.encode(original, full: full)
        watch()
    }

    private func watch() {
        let task = prepared
        Task { [weak self] in
            let result = await task.result
            guard let self, self.prepared == task else { return }
            switch result {
            case .success(let prepared): self.ready = prepared
            case .failure(let error): self.failure = error.localizedDescription.lowercased()
            }
        }
    }
}

/// Pictures from files and the pasteboard.
enum PictureSending {
    /// A file the person picked (security-scoped on the Mac).
    static func data(fromFile url: URL) -> Data? {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        return try? Data(contentsOf: url)
    }

    #if os(macOS)
    /// An image on the pasteboard (a screenshot, a copied picture or an
    /// image file from Finder), but only when there's no text to paste.
    static func pasteboardPicture() -> Data? {
        let board = NSPasteboard.general
        // A copied file (Finder also puts its name as text): only if it's a picture.
        if let urls = board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], let url = urls.first {
            guard let type = UTType(filenameExtension: url.pathExtension), type.conforms(to: .image) else { return nil }
            return try? Data(contentsOf: url)
        }
        if board.string(forType: .string) != nil { return nil }
        for type in [NSPasteboard.PasteboardType.png, .tiff, NSPasteboard.PasteboardType("public.jpeg"), NSPasteboard.PasteboardType("public.heic")] {
            if let data = board.data(forType: type) { return data }
        }
        return nil
    }
    #endif
}

/// Opens Photos (iPhone) or a file panel (Mac) and hands back the bytes.
struct PicturePickerButton<Label: View>: View {
    var onPick: (Data) -> Void
    @ViewBuilder var label: Label
    #if os(iOS)
    @State private var item: PhotosPickerItem?
    @State private var picking = false
    #else
    @State private var importing = false
    #endif

    var body: some View {
        #if os(iOS)
        Button { picking = true } label: { label }
            .photosPicker(isPresented: $picking, selection: $item, matching: .images, preferredItemEncoding: .current)
            .onChange(of: item) { _, picked in
                guard let picked else { return }
                item = nil
                Task {
                    if let data = try? await picked.loadTransferable(type: Data.self) { onPick(data) }
                }
            }
        #else
        Button { importing = true } label: { label }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.image]) { result in
                if case .success(let url) = result, let data = PictureSending.data(fromFile: url) { onPick(data) }
            }
        #endif
    }
}

#if os(macOS)
/// ⌘V with a picture on the pasteboard sends it (text pastes as usual).
struct PastePictureModifier: ViewModifier {
    var onPaste: (Data) -> Void
    @State private var monitor: Any?

    func body(content: Content) -> some View {
        content
            .onAppear {
                guard monitor == nil else { return }
                monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                    guard event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
                          event.charactersIgnoringModifiers?.lowercased() == "v",
                          let data = PictureSending.pasteboardPicture() else { return event }
                    onPaste(data)
                    return nil
                }
            }
            .onDisappear {
                if let monitor { NSEvent.removeMonitor(monitor) }
                monitor = nil
            }
    }
}
#endif

extension View {
    /// Mac: ⌘V with a picture sends it. iPhone: nothing (Photos picker instead).
    func pastesPictures(_ onPaste: @escaping (Data) -> Void) -> some View {
        #if os(macOS)
        modifier(PastePictureModifier(onPaste: onPaste))
        #else
        self
        #endif
    }
}

extension ImageStore {
    /// A message picture, decoded small enough for the chat (`maxPixel` on
    /// its long side) so a scroll through photos doesn't hold full-size bitmaps.
    func picture(for url: URL, maxPixel: Int = 900) -> PlatformImage? {
        let key = "\(url.absoluteString)#\(maxPixel)"
        if let hit = pictures[key] { return hit }
        loadPicture(url, key: key, maxPixel: maxPixel)
        return nil
    }
}

#if os(iOS)
/// iPhone, Personal space: the system Paste button (no "Allow Paste?" prompt,
/// no share sheet). A copied picture goes to the tray; text or a link goes
/// into the composer.
struct PasteToNote: View {
    var onPicture: (Data) -> Void
    var onText: (String) -> Void
    /// Only while the clipboard has a picture, a link or text.
    @State private var available = false

    var body: some View {
        Group {
            if available {
                PasteButton(supportedContentTypes: [.image, .url, .plainText]) { providers in
                    PastedItems.load(providers,
                                     onPicture: { data in onPicture(data) },
                                     onText: { text in onText(text) })
                }
                .accessibilityLabel("Paste")
            }
        }
        .clipboardHas(pictures: true, $available)
    }
}

/// Reads what PasteButton hands over. NSItemProvider calls back on its own
/// queues, so the callbacks are `@Sendable` and hop to the main actor.
nonisolated enum PastedItems {
    static func load(_ providers: [NSItemProvider],
                     onPicture: @escaping @MainActor @Sendable (Data) -> Void,
                     onText: @escaping @MainActor @Sendable (String) -> Void) {
        guard let provider = providers.first else { return }
        if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
            _ = provider.loadDataRepresentation(for: .image) { @Sendable data, _ in
                guard let data else { return }
                Task { @MainActor in onPicture(data) }
            }
        } else if provider.canLoadObject(ofClass: URL.self) {
            _ = provider.loadObject(ofClass: URL.self) { @Sendable url, _ in
                guard let text = url?.absoluteString else { return }
                Task { @MainActor in onText(text) }
            }
        } else if provider.canLoadObject(ofClass: String.self) {
            _ = provider.loadObject(ofClass: String.self) { @Sendable string, _ in
                guard let string else { return }
                Task { @MainActor in onText(string) }
            }
        }
    }
}
#endif
