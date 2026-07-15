import SwiftUI
import AppKit
import SearchFSKit

/// Main window: search bar + option controls on top, results table below.
struct ContentView: View {
    @StateObject private var vm = SearchViewModel()
    @State private var selection: SearchResult.ID?

    var body: some View {
        VStack(spacing: 0) {
            searchBar
            Divider()
            optionsBar
            Divider()
            resultsList
            Divider()
            statusBar
        }
        .frame(minWidth: 720, minHeight: 480)
    }

    // MARK: - Search bar

    private var searchBar: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundColor(.secondary)
            TextField("搜索文件名…（searchfs 实时，无索引）", text: $vm.query)
                .textFieldStyle(.plain)
                .font(.system(size: 15))
                .onChange(of: vm.query) { _ in vm.scheduleSearch() }
                .onSubmit { vm.scheduleSearch() }
            if vm.isSearching {
                ProgressView()
                    .controlSize(.small)
            }
            if !vm.query.isEmpty {
                Button {
                    vm.query = ""
                    vm.scheduleSearch()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    // MARK: - Options

    private var optionsBar: some View {
        HStack(spacing: 16) {
            Picker("", selection: $vm.scope) {
                Text("文件+目录").tag(SearchOptions.Scope.filesAndDirs)
                Text("仅文件").tag(SearchOptions.Scope.filesOnly)
                Text("仅目录").tag(SearchOptions.Scope.dirsOnly)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .onChange(of: vm.scope) { _ in vm.scheduleSearch() }

            Toggle("子串", isOn: $vm.substring)
                .toggleStyle(.checkbox)
                .onChange(of: vm.substring) { _ in vm.scheduleSearch() }

            Toggle("区分大小写", isOn: $vm.caseSensitive)
                .toggleStyle(.checkbox)
                .onChange(of: vm.caseSensitive) { _ in vm.scheduleSearch() }

            Spacer()

            HStack(spacing: 4) {
                Text("上限")
                    .foregroundColor(.secondary)
                TextField("", value: $vm.limit, format: .number)
                    .frame(width: 60)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { vm.scheduleSearch() }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    // MARK: - Results

    private var resultsList: some View {
        // A List of rows (rather than a Table) so we can attach a per-row
        // context menu — Table's contextMenu(forSelectionType:) needs macOS 13,
        // and we keep the deployment target at macOS 12.
        List(vm.results, selection: $selection) { r in
            HStack(spacing: 8) {
                Image(nsImage: NSWorkspace.shared.icon(forFile: r.path))
                    .resizable()
                    .frame(width: 16, height: 16)
                VStack(alignment: .leading, spacing: 1) {
                    Text(r.name)
                    Text(r.path)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            .contentShape(Rectangle())
            .contextMenu {
                Button("在访达中显示") { revealInFinder(r.path) }
                Button("拷贝路径") { copyPath(r.path) }
            }
            .onTapGesture(count: 2) { revealInFinder(r.path) }
            .tag(r.id)
        }
    }

    // MARK: - Status

    private var statusBar: some View {
        HStack {
            Text(vm.statusMessage)
                .font(.caption)
                .foregroundColor(.secondary)
            Spacer()
            Text("引擎: searchfs() · Road_A")
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
    }

    // MARK: - Actions

    private func revealInFinder(_ path: String) {
        NSWorkspace.shared.selectFile(path, inFileViewerRootedAtPath: "")
    }

    private func copyPath(_ path: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(path, forType: .string)
    }
}
