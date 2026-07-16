import Foundation
import Combine
import HybridEngine

/// Observable view-model that drives the SwiftUI GUI on top of `HybridEngine`.
///
/// It owns the debounced query pipeline, background search dispatch, and the
/// index-build lifecycle, exposing plain @Published state the views bind to.
@MainActor
final class AppModel: ObservableObject {

    // Query + results.
    @Published var query: String = ""
    @Published var results: [SearchHit] = []
    @Published var selection: SearchHit.ID? = nil

    // Filters (mirror the CLI/searchfs options).
    @Published var filesOnly = false
    @Published var dirsOnly = false
    @Published var forceFallback = false

    // Status surfaced in the footer.
    @Published var statusLine: String = "starting…"
    @Published var backendLabel: String = "-"
    @Published var isBuilding = false
    @Published var elapsedMS: Double = 0

    private let engine = HybridEngine()
    private var cancellables = Set<AnyCancellable>()
    private let searchQueue = DispatchQueue(label: "com.machaifind.c.search", qos: .userInitiated)
    private var generation = 0

    init() {
        // Debounce keystrokes so we search at most ~15×/s while typing.
        $query
            .removeDuplicates()
            .debounce(for: .milliseconds(60), scheduler: RunLoop.main)
            .sink { [weak self] _ in self?.runSearch() }
            .store(in: &cancellables)

        // Re-run when a filter toggles.
        Publishers.CombineLatest3($filesOnly, $dirsOnly, $forceFallback)
            .dropFirst()
            .sink { [weak self] _ in self?.runSearch() }
            .store(in: &cancellables)

        // The AppKit menu-bar "Rebuild Index" item posts this notification.
        NotificationCenter.default.publisher(for: .mhfcRebuildIndex)
            .sink { [weak self] _ in self?.rebuildIndex() }
            .store(in: &cancellables)

        refreshStatus()
        ensureIndex()
        engine.startWatching { [weak self] in
            Task { @MainActor in self?.refreshStatus() }
        }
    }

    // MARK: - Index lifecycle

    /// Build the index on first launch if none exists yet (off the main thread).
    func ensureIndex() {
        if engine.isIndexed { return }
        rebuildIndex()
    }

    func rebuildIndex() {
        guard !isBuilding else { return }
        isBuilding = true
        statusLine = "building index…"
        searchQueue.async { [weak self] in
            guard let self else { return }
            let count = (try? self.engine.buildIndex()) ?? 0
            Task { @MainActor in
                self.isBuilding = false
                self.statusLine = "indexed \(count) items"
                self.refreshStatus()
                self.runSearch()
            }
        }
    }

    // MARK: - Search

    func runSearch() {
        let q = query
        let opts = SearchOptions(filesOnly: filesOnly, dirsOnly: dirsOnly, limit: 500)
        let fallback = forceFallback
        generation += 1
        let gen = generation

        searchQueue.async { [weak self] in
            guard let self else { return }
            let start = Date()
            let (hits, backend) = self.engine.search(q, options: opts, forceFallback: fallback)
            let ms = Date().timeIntervalSince(start) * 1000
            Task { @MainActor in
                guard gen == self.generation else { return }  // drop stale results
                self.results = hits
                self.backendLabel = backend.rawValue
                self.elapsedMS = ms
                self.statusLine = "\(hits.count) results · \(String(format: "%.1f", ms)) ms · \(backend.rawValue)"
            }
        }
    }

    func refreshStatus() {
        // Don't clobber the "building index…" message mid-build.
        if isBuilding { return }
        let s = engine.status
        backendLabel = s.backend.rawValue
        if s.backend == .index {
            statusLine = "index: \(s.entryCount) items\(s.stale ? " (stale)" : "")"
        } else if s.backend == .searchfs {
            statusLine = "no index — using searchfs() fallback"
        } else {
            statusLine = "no search backend available"
        }
    }

    // MARK: - Actions on the selected / a given hit

    func revealInFinder(_ hit: SearchHit) {
        Actions.revealInFinder(path: hit.path)
    }

    func open(_ hit: SearchHit) {
        Actions.open(path: hit.path)
    }

    func copyPath(_ hit: SearchHit) {
        Actions.copyToPasteboard(hit.path)
    }
}
