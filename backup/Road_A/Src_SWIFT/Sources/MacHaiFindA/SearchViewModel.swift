import Foundation
import SwiftUI
import SearchFSKit

/// Drives the GUI: debounces input, runs searchfs off the main thread, and
/// publishes results back for the list to render.
@MainActor
final class SearchViewModel: ObservableObject {
    @Published var query: String = ""
    @Published var scope: SearchOptions.Scope = .filesAndDirs
    @Published var substring: Bool = true
    @Published var caseSensitive: Bool = false
    @Published var limit: Int = 1000

    @Published private(set) var results: [SearchResult] = []
    @Published private(set) var isSearching: Bool = false
    @Published private(set) var elapsed: TimeInterval = 0
    @Published private(set) var statusMessage: String = "输入关键字开始搜索"

    private var searchTask: Task<Void, Never>?
    /// Monotonic token: only the newest search is allowed to publish.
    private var generation = 0

    /// Debounced trigger — call on every keystroke / option change.
    func scheduleSearch() {
        searchTask?.cancel()
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else {
            results = []
            isSearching = false
            statusMessage = "输入关键字开始搜索"
            return
        }

        generation += 1
        let myGen = generation
        let options = SearchOptions(scope: scope,
                                    substring: substring,
                                    caseSensitive: caseSensitive,
                                    limit: limit)

        searchTask = Task { [weak self] in
            // 200ms debounce so we don't fire a searchfs scan on every keystroke.
            try? await Task.sleep(nanoseconds: 200_000_000)
            if Task.isCancelled { return }
            await self?.performSearch(term: q, options: options, gen: myGen)
        }
    }

    private func performSearch(term: String, options: SearchOptions, gen: Int) async {
        guard gen == generation else { return }
        isSearching = true
        statusMessage = "搜索中…"
        let start = Date()

        // searchfs() is a blocking syscall loop — run it off the main actor.
        let found: [SearchResult] = await Task.detached(priority: .userInitiated) {
            SearchEngine.search(term: term, options: options) {
                // Cooperative cancellation: stop the C loop if a newer search started.
                Task.isCancelled
            }
        }.value

        // Drop stale results.
        guard gen == generation else { return }
        results = found
        elapsed = Date().timeIntervalSince(start)
        isSearching = false
        statusMessage = found.isEmpty
            ? "无匹配结果（用时 \(String(format: "%.0f", elapsed * 1000)) ms）"
            : "\(found.count) 个结果（用时 \(String(format: "%.0f", elapsed * 1000)) ms）"
    }
}
