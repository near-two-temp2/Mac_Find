import Foundation
#if canImport(SwiftUI) && canImport(AppKit)
import SwiftUI
import AppKit
import SearchEngine

/// Boots a SwiftUI window without relying on the `@main` App lifecycle (main.swift
/// owns process entry so the CLI path stays window-server-free). We drive an
/// NSApplication manually and host the SwiftUI view in an NSHostingView.
enum GUIApp {
    static func run() -> Never {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)

        let delegate = AppDelegate()
        app.delegate = delegate
        // Retain the delegate for the process lifetime.
        _ = Unmanaged.passRetained(delegate)

        app.run()
        exit(0)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let model = SearchModel()
        let view = ContentView().environmentObject(model)

        let hosting = NSHostingView(rootView: view)
        let win = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 820, height: 560),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        win.title = "MacFind · Swift · Road_B"
        win.center()
        win.contentView = hosting
        win.makeKeyAndOrderFront(nil)
        self.window = win

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

/// Observable view model bridging the SwiftUI UI to the background engine.
final class SearchModel: ObservableObject {
    @Published var query: String = ""
    @Published var results: [SearchResult] = []
    @Published var status: String = "No index. Click “Build Index” to scan your Home folder."
    @Published var isIndexing: Bool = false
    @Published var filesOnly: Bool = false
    @Published var dirsOnly: Bool = false

    private var engine: SearchEngine?
    private let indexURL = CLI.defaultIndexURL()
    private let workQueue = DispatchQueue(label: "com.machaifind.roadb.search", qos: .userInitiated)
    private var searchGeneration = 0

    init() {
        // Try to attach to a pre-existing index on launch.
        tryLoadIndex()
    }

    func tryLoadIndex() {
        workQueue.async { [weak self] in
            guard let self else { return }
            if let e = try? SearchEngine(indexURL: self.indexURL) {
                DispatchQueue.main.async {
                    self.engine = e
                    self.status = "Index ready: \(e.index.count) entries."
                    self.runSearch()
                }
            }
        }
    }

    func buildIndex() {
        guard !isIndexing else { return }
        isIndexing = true
        status = "Building index…"
        let url = indexURL
        let root = FileManager.default.homeDirectoryForCurrentUser

        workQueue.async { [weak self] in
            guard let self else { return }
            let builder = IndexBuilder()
            do {
                let count = try builder.build(root: root, outputURL: url) { p in
                    DispatchQueue.main.async {
                        self.status = "Indexing… \(p.scanned) files"
                    }
                }
                let engine = try SearchEngine(indexURL: url)
                DispatchQueue.main.async {
                    self.engine = engine
                    self.isIndexing = false
                    self.status = "Index ready: \(count) entries."
                    self.runSearch()
                }
            } catch {
                DispatchQueue.main.async {
                    self.isIndexing = false
                    self.status = "Index failed: \(error)"
                }
            }
        }
    }

    /// Debounced instant search on every keystroke.
    func runSearch() {
        searchGeneration += 1
        let gen = searchGeneration
        guard let engine else {
            results = []
            return
        }
        let q = query
        var opts = SearchOptions(maxResults: 500)
        opts.filesOnly = filesOnly
        opts.dirsOnly = dirsOnly

        workQueue.async { [weak self] in
            let hits = engine.search(q, options: opts)
            DispatchQueue.main.async {
                guard let self, gen == self.searchGeneration else { return }
                self.results = hits
                if !q.isEmpty {
                    self.status = "\(hits.count) hits for “\(q)” · \(engine.index.count) indexed"
                }
            }
        }
    }

    func reveal(_ result: SearchResult) {
        NSWorkspace.shared.selectFile(result.path, inFileViewerRootedAtPath: "")
    }
}

struct ContentView: View {
    @EnvironmentObject var model: SearchModel

    var body: some View {
        VStack(spacing: 0) {
            // Top toolbar: search box + controls.
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundColor(.secondary)
                TextField("Search files…", text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 15))
                    .onChange(of: model.query) { _ in model.runSearch() }

                Toggle("Files", isOn: $model.filesOnly)
                    .toggleStyle(.checkbox)
                    .onChange(of: model.filesOnly) { on in
                        if on { model.dirsOnly = false }
                        model.runSearch()
                    }
                Toggle("Dirs", isOn: $model.dirsOnly)
                    .toggleStyle(.checkbox)
                    .onChange(of: model.dirsOnly) { on in
                        if on { model.filesOnly = false }
                        model.runSearch()
                    }

                Button(action: { model.buildIndex() }) {
                    if model.isIndexing {
                        ProgressView().scaleEffect(0.6).frame(width: 16, height: 16)
                    } else {
                        Text("Build Index")
                    }
                }
                .disabled(model.isIndexing)
            }
            .padding(10)

            Divider()

            // Results list.
            if model.results.isEmpty {
                Spacer()
                Text(model.query.isEmpty ? "Type to search." : "No matches.")
                    .foregroundColor(.secondary)
                Spacer()
            } else {
                List(model.results) { r in
                    HStack {
                        Image(systemName: r.isDir ? "folder" : "doc")
                            .foregroundColor(r.isDir ? .accentColor : .secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text((r.path as NSString).lastPathComponent)
                                .font(.system(size: 13, weight: .medium))
                            Text(r.path)
                                .font(.system(size: 11))
                                .foregroundColor(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        Spacer()
                        Text("\(r.score)")
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                    }
                    .contentShape(Rectangle())
                    .onTapGesture(count: 2) { model.reveal(r) }
                }
                .listStyle(.inset)
            }

            Divider()
            HStack {
                Text(model.status).font(.system(size: 11)).foregroundColor(.secondary)
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
        }
        .frame(minWidth: 620, minHeight: 420)
    }
}
#else
import SearchEngine

/// Fallback when SwiftUI/AppKit aren't available (non-macOS build). The GUI is
/// macOS-only; on other platforms we exit with a message so CI still links.
enum GUIApp {
    static func run() -> Never {
        FileHandle.standardError.write(Data("GUI is only available on macOS. Use `index` / `search`.\n".utf8))
        exit(1)
    }
}
#endif
