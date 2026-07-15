// main_cli.cpp — index/search CLI entry for scripting and CI smoke tests.
//
//   mff-b index  [--root DIR]... [--out FILE]
//   mff-b search [--index FILE] [--limit N] [--files|--dirs] [--ext EXT] QUERY
//   mff-b selftest
//
// `selftest` builds a tiny in-memory index over a temp tree and asserts search
// works end-to-end, giving CI a runner-independent success signal.

#include "index_engine.hpp"
#include "paths.hpp"
#include "scanner.hpp"

#include <unistd.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

using namespace mff;

static int usage() {
    std::fprintf(stderr,
        "usage:\n"
        "  mff-b index  [--root DIR]... [--out FILE]\n"
        "  mff-b search [--index FILE] [--limit N] [--files|--dirs] [--ext EXT] QUERY\n"
        "  mff-b selftest\n");
    return 2;
}

static int cmdIndex(int argc, char** argv) {
    std::vector<std::string> roots;
    std::string out = defaultIndexPath();
    for (int i = 0; i < argc; ++i) {
        if (!std::strcmp(argv[i], "--root") && i + 1 < argc) roots.push_back(argv[++i]);
        else if (!std::strcmp(argv[i], "--out") && i + 1 < argc) out = argv[++i];
    }
    if (roots.empty()) roots = defaultRoots();

    IndexEngine eng;
    std::fprintf(stderr, "indexing %zu root(s)...\n", roots.size());
    size_t n = eng.buildFromRoots(roots);
    if (!eng.save(out)) {
        std::fprintf(stderr, "error: failed to write %s\n", out.c_str());
        return 1;
    }
    std::fprintf(stderr, "indexed %zu entries -> %s\n", n, out.c_str());
    return 0;
}

static int cmdSearch(int argc, char** argv) {
    std::string idx = defaultIndexPath();
    SearchOptions opts;
    std::string query;
    for (int i = 0; i < argc; ++i) {
        if (!std::strcmp(argv[i], "--index") && i + 1 < argc) idx = argv[++i];
        else if (!std::strcmp(argv[i], "--limit") && i + 1 < argc) opts.maxResults = std::atoi(argv[++i]);
        else if (!std::strcmp(argv[i], "--files")) opts.filesOnly = true;
        else if (!std::strcmp(argv[i], "--dirs")) opts.dirsOnly = true;
        else if (!std::strcmp(argv[i], "--ext") && i + 1 < argc) opts.extension = argv[++i];
        else query = argv[i];
    }
    IndexEngine eng;
    if (!eng.loadMmap(idx)) {
        std::fprintf(stderr, "error: cannot open index %s (run `index` first)\n", idx.c_str());
        return 1;
    }
    auto hits = eng.search(query, opts);
    for (const auto& h : hits)
        std::printf("%6d  %s%s\n", h.score, h.path.c_str(), h.isDir ? "/" : "");
    std::fprintf(stderr, "%zu result(s) from %zu entries\n", hits.size(), eng.entryCount());
    return 0;
}

// Build a throwaway directory tree so selftest never depends on runner layout.
static std::string makeSelftestTree() {
    char tmpl[] = "/tmp/mffb_selftest_XXXXXX";
    char* dir = mkdtemp(tmpl);
    if (!dir) return {};
    std::string d(dir);
    auto touch = [&](const std::string& rel) {
        std::string p = d + "/" + rel;
        FILE* f = std::fopen(p.c_str(), "w");
        if (f) { std::fputs("x", f); std::fclose(f); }
    };
    touch("ReadMe.md");
    touch("main_engine.cpp");
    touch("photo_2024.png");
    touch("notes.txt");
    return d;
}

static int cmdSelftest() {
    std::string tree = makeSelftestTree();
    if (tree.empty()) { std::fprintf(stderr, "selftest: mkdtemp failed\n"); return 1; }

    IndexEngine eng;
    size_t n = eng.buildFromRoots({tree});
    if (n < 4) { std::fprintf(stderr, "selftest: expected >=4 entries, got %zu\n", n); return 1; }

    // Fuzzy query "engcpp" should surface main_engine.cpp.
    auto hits = eng.search("engcpp", SearchOptions{});
    bool found = false;
    for (const auto& h : hits)
        if (h.path.find("main_engine.cpp") != std::string::npos) found = true;
    if (!found) { std::fprintf(stderr, "selftest: fuzzy match failed\n"); return 1; }

    // Extension filter.
    auto pngs = eng.search("photo", SearchOptions{200, false, false, "png"});
    if (pngs.empty()) { std::fprintf(stderr, "selftest: ext filter failed\n"); return 1; }

    // Round-trip through save + mmap load.
    std::string idxPath = tree + "/test.idx";
    if (!eng.save(idxPath)) { std::fprintf(stderr, "selftest: save failed\n"); return 1; }
    IndexEngine eng2;
    if (!eng2.loadMmap(idxPath)) { std::fprintf(stderr, "selftest: load failed\n"); return 1; }
    if (eng2.entryCount() != n) { std::fprintf(stderr, "selftest: count mismatch\n"); return 1; }
    auto hits2 = eng2.search("readme", SearchOptions{});
    if (hits2.empty()) { std::fprintf(stderr, "selftest: mmap search failed\n"); return 1; }

    std::printf("selftest OK: %zu entries, fuzzy+ext+mmap all pass\n", n);
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 2) return usage();
    std::string cmd = argv[1];
    if (cmd == "index")    return cmdIndex(argc - 2, argv + 2);
    if (cmd == "search")   return cmdSearch(argc - 2, argv + 2);
    if (cmd == "selftest") return cmdSelftest();
    return usage();
}
