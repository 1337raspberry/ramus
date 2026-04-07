import SwiftUI
import Search
import Cache
import GenreTree
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "Search")

@Observable
final class SearchViewModel {

    // MARK: - Dependencies

    private var engine: SearchEngine?

    // MARK: - State

    var query: String = ""
    var results: [SearchResult] = []
    var selectedIndex: Int = 0
    var isVisible: Bool = false
    var isSearching: Bool = false

    private var searchTask: Task<Void, Never>?

    // MARK: - Setup

    func setup(cache: CacheDatabase?, genreMapper: GenreMapper?) {
        guard let cache else { return }
        engine = SearchEngine(db: cache, genreMapper: genreMapper)
    }

    // MARK: - Actions

    func show(withPrefix prefix: String = "") {
        let t = ContinuousClock.now
        isVisible = true
        results = []
        selectedIndex = 0
        query = prefix
        log.info("show(prefix: \"\(prefix, privacy: .public)\") — \(ContinuousClock.now - t, privacy: .public)")
    }

    func dismiss() {
        isVisible = false
        query = ""
        results = []
        searchTask?.cancel()
    }

    func moveUp() {
        if selectedIndex > 0 {
            selectedIndex -= 1
        }
    }

    func moveDown() {
        if selectedIndex < results.count - 1 {
            selectedIndex += 1
        }
    }

    func confirm() -> SearchResult? {
        guard selectedIndex >= 0, selectedIndex < results.count else { return nil }
        return results[selectedIndex]
    }

    // MARK: - Debounced Search

    func queryChanged() {
        searchTask?.cancel()

        guard !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            results = []
            selectedIndex = 0
            return
        }

        searchTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }

            guard let engine else { return }
            let t0 = ContinuousClock.now
            let parsed = QueryParser.parse(query)
            let tParse = ContinuousClock.now

            isSearching = true
            defer { isSearching = false }
            do {
                let r = try engine.search(parsed)
                let tSearch = ContinuousClock.now
                guard !Task.isCancelled else { return }
                results = r
                let tAssign = ContinuousClock.now
                selectedIndex = 0
                log.info("queryChanged \"\(self.query, privacy: .public)\": parse \(tParse - t0, privacy: .public), search \(tSearch - tParse, privacy: .public) (\(r.count, privacy: .public) results), assign \(tAssign - tSearch, privacy: .public)")
            } catch {
                results = []
                log.warning("queryChanged error: \(error.localizedDescription, privacy: .public)")
            }
        }
    }
}
