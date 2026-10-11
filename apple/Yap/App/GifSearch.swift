import SwiftUI
import YapCore

/// The GIF picker's state: trending / recent /
/// search, KLIPY's pages, and what to say when there's nothing. Testing keys
/// get 100 requests an hour for the whole server, so search is debounced and
/// trending is cached (`TrendingCache`).
@MainActor
@Observable
final class GifSearch {
    var mode: KlipyClient.Mode = .trending
    var query = ""
    private(set) var gifs: [Gif] = []
    private(set) var hasNext = false
    private(set) var loading = false
    private(set) var status: String?
    @ObservationIgnored private var page = 1
    /// The next page being fetched; a new search cancels it, so an old
    /// page can't land under the new results.
    @ObservationIgnored private var moreTask: Task<Void, Never>?

    /// What `.task(id:)` follows: the tab, the words and the server's key.
    func key(_ model: AppModel) -> String {
        "\(mode.rawValue)|\(query)|\(model.config?.gifs?.appKey ?? "-")"
    }

    /// The first page (call from `.task(id: key)`).
    func reload(_ model: AppModel) async {
        moreTask?.cancel()
        moreTask = nil
        // Config (with the KLIPY key) may still be on its way.
        guard model.config != nil else {
            loading = true
            return
        }
        if !query.isEmpty { try? await Task.sleep(for: .milliseconds(400)) }
        guard !Task.isCancelled else { return }
        await load(more: false, model: model)
    }

    /// The next page (the end of the grid came on screen).
    func loadMore(_ model: AppModel) {
        guard hasNext, !loading, moreTask == nil else { return }
        moreTask = Task {
            await load(more: true, model: model)
            moreTask = nil
        }
    }

    /// The search the picked GIF came from (for KLIPY's share report).
    var pickedQuery: String? { query.isEmpty ? nil : query }

    private func load(more: Bool, model: AppModel) async {
        let klipy = model.klipy
        let activeMode: KlipyClient.Mode = query.isEmpty ? mode : .search
        let nextPage = more ? page + 1 : 1
        loading = true
        defer { loading = false }
        if !more {
            gifs = []
            status = nil
        }
        do {
            let result = try await TrendingCache.shared.page(klipy: klipy, mode: activeMode, query: query, page: nextPage)
            guard !Task.isCancelled else { return }
            gifs += result.gifs
            page = nextPage
            hasNext = result.hasNext
            if gifs.isEmpty { status = activeMode == .recent ? "GIFs you send show up here." : "No GIFs found." }
        } catch let failure as KlipyClient.Failure {
            hasNext = false
            switch failure {
            case .notConfigured: status = "GIFs aren't set up on this server yet."
            case .rateLimited: status = "GIF search is resting — the free KLIPY key allows 100 searches an hour. Try again soon."
            case .http: status = "Couldn't reach KLIPY. Try again in a moment."
            }
        } catch {
            hasNext = false
            if !Task.isCancelled { status = "Couldn't reach KLIPY. Try again in a moment." }
        }
    }
}
