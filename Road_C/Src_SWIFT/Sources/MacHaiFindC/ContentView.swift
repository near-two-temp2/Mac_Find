import SwiftUI
import AppKit
import HybridEngine

/// The flagship UI shell: search field + filter toggles + results list + footer.
/// Shared "search box + results list" skeleton across the 18 implementations;
/// what's distinctive here is the hybrid backend badge and the fallback toggle.
struct ContentView: View {
    @StateObject private var model = AppModel()
    @FocusState private var searchFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            searchBar
            Divider()
            resultsList
            Divider()
            footer
        }
        .frame(minWidth: 620, minHeight: 400)
        .onAppear { searchFocused = true }
    }

    // MARK: Search bar + filters

    private var searchBar: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Search files and folders…", text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 18))
                    .focused($searchFocused)
                    .onSubmit { openSelected() }
                if !model.query.isEmpty {
                    Button {
                        model.query = ""
                        searchFocused = true
                    } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary) }
                    .buttonStyle(.plain)
                }
            }
            HStack(spacing: 12) {
                Toggle("Files only", isOn: $model.filesOnly)
                    .toggleStyle(.checkbox)
                    .onChange(of: model.filesOnly) { v in if v { model.dirsOnly = false } }
                Toggle("Folders only", isOn: $model.dirsOnly)
                    .toggleStyle(.checkbox)
                    .onChange(of: model.dirsOnly) { v in if v { model.filesOnly = false } }
                Toggle("Force searchfs()", isOn: $model.forceFallback)
                    .toggleStyle(.checkbox)
                    .help("Bypass the binary index and query the volume catalog directly")
                Spacer()
                Button {
                    model.rebuildIndex()
                } label: {
                    Label("Rebuild index", systemImage: "arrow.clockwise")
                }
                .disabled(model.isBuilding)
            }
            .font(.system(size: 12))
            .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    // MARK: Results

    private var resultsList: some View {
        Group {
            if model.results.isEmpty {
                emptyState
            } else {
                List(selection: $model.selection) {
                    ForEach(model.results, id: \.id) { hit in
                        ResultRow(hit: hit)
                            .tag(hit.id)
                            .contextMenu { rowMenu(hit) }
                            .onTapGesture(count: 2) { model.open(hit) }
                    }
                }
                .listStyle(.inset)
            }
        }
        .frame(maxHeight: .infinity)
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            if model.isBuilding {
                ProgressView()
                Text("Building index…").foregroundStyle(.secondary)
            } else if model.query.isEmpty {
                Image(systemName: "magnifyingglass").font(.system(size: 34)).foregroundStyle(.tertiary)
                Text("Type to search").foregroundStyle(.secondary)
            } else {
                Image(systemName: "tray").font(.system(size: 34)).foregroundStyle(.tertiary)
                Text("No results").foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    @ViewBuilder
    private func rowMenu(_ hit: SearchHit) -> some View {
        Button("Open") { model.open(hit) }
        Button("Reveal in Finder") { model.revealInFinder(hit) }
        Divider()
        Button("Copy Path") { model.copyPath(hit) }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 10) {
            backendBadge
            Text(model.statusLine)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer()
            if let sel = selectedHit {
                Button {
                    model.revealInFinder(sel)
                } label: { Label("Reveal", systemImage: "folder") }
                .controlSize(.small)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
    }

    private var backendBadge: some View {
        Text(model.backendLabel.uppercased())
            .font(.system(size: 10, weight: .semibold))
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(badgeColor.opacity(0.18), in: Capsule())
            .foregroundStyle(badgeColor)
    }

    private var badgeColor: Color {
        switch model.backendLabel {
        case "index": return .green
        case "searchfs": return .orange
        default: return .gray
        }
    }

    // MARK: Helpers

    private var selectedHit: SearchHit? {
        guard let id = model.selection else { return model.results.first }
        return model.results.first { $0.id == id }
    }

    private func openSelected() {
        if let hit = selectedHit { model.open(hit) }
    }
}

/// One result row: icon + basename + dimmed parent path.
private struct ResultRow: View {
    let hit: SearchHit

    var body: some View {
        HStack(spacing: 8) {
            Image(nsImage: Actions.icon(for: hit.path, isDir: hit.isDir))
                .resizable().frame(width: 18, height: 18)
            VStack(alignment: .leading, spacing: 1) {
                Text(basename).font(.system(size: 13))
                Text(parent).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if hit.score > 0 {
                Text("\(hit.score)")
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.vertical, 1)
    }

    private var basename: String { (hit.path as NSString).lastPathComponent }
    private var parent: String { (hit.path as NSString).deletingLastPathComponent }
}

// SearchHit is Hashable; expose an Identifiable id (the path) for List/ForEach.
extension SearchHit: Identifiable {
    public var id: String { path }
}
