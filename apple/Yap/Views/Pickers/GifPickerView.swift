import SwiftUI
import YapCore

/// GIFs from KLIPY, straight from the device (PROTOCOL.md › GIFs): results
/// in KLIPY's own order and grid, "Powered by KLIPY", media loaded from
/// their URLs untouched. Testing keys get 100 requests an hour for the whole
/// server, so search is debounced and trending is cached.
struct GifPickerView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// The picked GIF and the search it came from (for KLIPY's share report).
    var onPick: (Gif, String?) -> Void

    @State private var search = GifSearch()

    var body: some View {
        @Bindable var search = search
        NavigationStack {
            VStack(alignment: .leading, spacing: 6) {
                Field98 {
                    TextField("", text: $search.query, prompt: Text("Search KLIPY").foregroundStyle(Theme.grayText))
                        .autocorrectionDisabled()
                }
                if search.query.isEmpty {
                    Tabs98(tabs: [(KlipyClient.Mode.trending, "Trending"), (KlipyClient.Mode.recent, "Recent")], selection: $search.mode)
                        .padding(.top, 2)
                }
                ScrollView {
                    VStack(spacing: 8) {
                        MasonryGrid(gifs: search.gifs, columns: 2) { gif in
                            onPick(gif, search.pickedQuery)
                            dismiss()
                        }
                        if let status = search.status {
                            Text(status)
                                .font(Theme.chrome)
                                .foregroundStyle(Theme.grayText)
                                .multilineTextAlignment(.center)
                                .padding(20)
                        }
                        if search.hasNext, !search.loading {
                            Color.clear.frame(height: 1).onAppear { search.loadMore(model) }
                        }
                        if search.loading { ProgressView().padding() }
                    }
                    .padding(4)
                }
                .well()
                HStack(spacing: 2) {
                    StatusPanel(text: search.loading ? "Loading…" : "\(search.gifs.count) GIFs")
                    Link(destination: URL(string: "https://klipy.com")!) {
                        StatusPanel(text: "Powered by KLIPY", systemImage: "link", color: Theme.link)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(6)
            .background(Theme.face)
            .win98Sheet("GIFs") {
                CloseSheetButton { dismiss() }
            }
            .task(id: search.key(model)) { await search.reload(model) }
        }
        .presentationDetents([.large])
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 560)
        #endif
    }

}

/// Trending is the same for everyone and changes slowly: keep it ~10 min.
@MainActor
final class TrendingCache {
    static let shared = TrendingCache()
    private var pages: [Int: KlipyClient.Page] = [:]
    private var at = Date.distantPast

    func page(klipy: KlipyClient, mode: KlipyClient.Mode, query: String, page: Int) async throws -> KlipyClient.Page {
        guard mode == .trending else { return try await klipy.page(mode, query: query, page: page) }
        if Date().timeIntervalSince(at) > 600 { pages = [:] }
        if let hit = pages[page] { return hit }
        let result = try await klipy.page(.trending, page: page)
        pages[page] = result
        at = Date()
        return result
    }
}

/// Masonry: each tile goes into the shortest column, so the order still
/// reads left→right, top→bottom (KLIPY's order).
struct MasonryGrid: View {
    var gifs: [Gif]
    var columns: Int
    var onTap: (Gif) -> Void

    var body: some View {
        let split = distribute()
        HStack(alignment: .top, spacing: 4) {
            ForEach(0..<columns, id: \.self) { column in
                LazyVStack(spacing: 4) {
                    ForEach(split[column], id: \.offset) { item in
                        Button { onTap(item.gif) } label: {
                            GifTile(gif: item.gif)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(item.gif.title.isEmpty ? "GIF" : item.gif.title)
                    }
                }
            }
        }
    }

    private func distribute() -> [[(offset: Int, gif: Gif)]] {
        var out = Array(repeating: [(offset: Int, gif: Gif)](), count: columns)
        var heights = Array(repeating: 0.0, count: columns)
        for (i, gif) in gifs.enumerated() {
            let column = heights.indices.min { heights[$0] < heights[$1] } ?? 0
            out[column].append((i, gif))
            heights[column] += 1 / gif.aspectRatio
        }
        return out
    }
}

struct GifTile: View {
    var gif: Gif

    var body: some View {
        Color.clear
            .aspectRatio(gif.aspectRatio, contentMode: .fit)
            .overlay {
                if let url = gif.imageURL {
                    AnimatedRemoteImage(url: url, placeholder: Theme.window)
                } else if let still = gif.stillURL {
                    AsyncImage(url: still) { $0.resizable().scaledToFill() } placeholder: { Theme.window }
                }
            }
            .clipped()
            .overlay(Rectangle().stroke(Theme.shadow, lineWidth: 1))
    }
}
