// main_cli.cpp — headless CLI for Road_C (C++) hybrid engine.
//
// Purpose: give CI something to run without a display server, and let a human
// exercise the hybrid behaviour from a shell. Subcommands:
//
//   macfind-c-cli index [root ...]          build + persist the index
//   macfind-c-cli search [-dfs] [-m N] TERM query (index-first, searchfs fallback)
//   macfind-c-cli info                      show whether an index is loaded
//
// The `search` path prints which backend served the query, which is exactly the
// hybrid property Road_C is meant to demonstrate.
#include "HybridEngine.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include <getopt.h>

using namespace macfind;

static void usage(const char* prog) {
    std::fprintf(stderr,
        "usage:\n"
        "  %s index [root ...]            build and persist the binary index\n"
        "  %s search [-d|-f] [-s] [-m N] TERM   query (index-first, searchfs fallback)\n"
        "  %s info                        report index status\n"
        "\nsearch flags: -d dirs only  -f files only  -s case sensitive  -m limit\n",
        prog, prog, prog);
}

static int doIndex(int argc, char** argv) {
    std::vector<std::string> roots;
    for (int i = 0; i < argc; ++i) roots.emplace_back(argv[i]);

    HybridEngine eng;
    std::fprintf(stderr, "Building index%s…\n",
                 roots.empty() ? " over $HOME" : "");
    bool ok = eng.buildIndex(roots, std::string(),
        [](std::size_t n) {
            std::fprintf(stderr, "\r  %zu entries…", n);
            std::fflush(stderr);
        });
    std::fprintf(stderr, "\n");
    if (!ok) {
        std::fprintf(stderr, "Index build failed.\n");
        return 1;
    }
    std::fprintf(stderr, "Indexed %zu entries → %s\n",
                 eng.indexEntryCount(), defaultIndexPath().c_str());
    return 0;
}

static int doSearch(int argc, char** argv) {
    SearchOptions opts;
    // getopt over the sub-args (argv[0] is "search").
    optind = 1;
    int ch;
    while ((ch = getopt(argc, argv, "dfsm:")) != -1) {
        switch (ch) {
            case 'd': opts.dirsOnly = true; break;
            case 'f': opts.filesOnly = true; break;
            case 's': opts.caseSensitive = true; break;
            case 'm': opts.limit = std::strtoul(optarg, nullptr, 10); break;
            default:  return 2;
        }
    }
    if (opts.dirsOnly && opts.filesOnly) {
        std::fprintf(stderr, "-d and -f are mutually exclusive.\n");
        return 2;
    }
    if (optind >= argc) {
        std::fprintf(stderr, "search: missing TERM\n");
        return 2;
    }
    std::string term = argv[optind];

    HybridEngine eng;  // ctor auto-loads any existing index
    SearchOutcome out = eng.search(term, opts, /*allowFallback=*/true);
    if (!out.ok) {
        std::fprintf(stderr, "Error: %s\n", out.error.c_str());
        return 1;
    }
    for (const auto& r : out.results) {
        std::printf("%s\n", r.path.c_str());
    }
    std::fprintf(stderr, "%zu match(es) via %s.\n",
                 out.results.size(), backendName(out.backend));
    return 0;
}

static int doInfo() {
    HybridEngine eng;
    if (eng.indexAvailable()) {
        std::fprintf(stderr, "Index loaded: %zu entries at %s\n",
                     eng.indexEntryCount(), defaultIndexPath().c_str());
    } else {
        std::fprintf(stderr, "No index loaded — queries would use searchfs() fallback.\n");
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 2) { usage(argv[0]); return 2; }

    std::string cmd = argv[1];
    if (cmd == "index")  return doIndex(argc - 2, argv + 2);
    if (cmd == "search") return doSearch(argc - 1, argv + 1);  // keep "search" as argv[0]
    if (cmd == "info")   return doInfo();

    if (cmd == "-h" || cmd == "--help") { usage(argv[0]); return 0; }
    std::fprintf(stderr, "Unknown command: %s\n", cmd.c_str());
    usage(argv[0]);
    return 2;
}
