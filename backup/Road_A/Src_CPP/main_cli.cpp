// main_cli.cpp — headless CLI entry point for Road_A (C++) searchfs engine.
//
// Exists mainly so CI can smoke-test the search engine without a display server.
// Mirrors the flag surface of Open_Ref/searchfs/main.m.
#include "SearchEngine.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <getopt.h>

using namespace macfind;

static void printUsage(const char* prog) {
    std::fprintf(stderr,
        "usage: %s [-dfesl] [-m limit] [-v volume] search_term\n"
        "  -d  dirs only        -f  files only\n"
        "  -e  exact match      -s  case sensitive\n"
        "  -m  limit N          -v  volume mount path\n"
        "  -l  list searchable volumes and exit\n",
        prog);
}

int main(int argc, char* argv[]) {
    SearchOptions opts;
    std::string volume;

    static struct option longOpts[] = {
        {"dirs-only",      no_argument,       nullptr, 'd'},
        {"files-only",     no_argument,       nullptr, 'f'},
        {"exact-match",    no_argument,       nullptr, 'e'},
        {"case-sensitive", no_argument,       nullptr, 's'},
        {"limit",          required_argument, nullptr, 'm'},
        {"volume",         required_argument, nullptr, 'v'},
        {"list",           no_argument,       nullptr, 'l'},
        {"help",           no_argument,       nullptr, 'h'},
        {nullptr, 0, nullptr, 0},
    };

    int ch;
    while ((ch = getopt_long(argc, argv, "dfesm:v:lh", longOpts, nullptr)) != -1) {
        switch (ch) {
            case 'd': opts.dirsOnly = true; break;
            case 'f': opts.filesOnly = true; break;
            case 'e': opts.exactMatch = true; break;
            case 's': opts.caseSensitive = true; break;
            case 'm': opts.limit = std::strtoul(optarg, nullptr, 10); break;
            case 'v': volume = optarg; break;
            case 'l': {
                for (const auto& v : listSearchableVolumes()) {
                    std::printf("%s\n", v.c_str());
                }
                return 0;
            }
            case 'h':
            default:
                printUsage(argv[0]);
                return ch == 'h' ? 0 : 2;
        }
    }

    if (opts.dirsOnly && opts.filesOnly) {
        std::fprintf(stderr, "-d and -f are mutually exclusive.\n");
        return 2;
    }
    if (optind >= argc) {
        printUsage(argv[0]);
        return 2;
    }

    std::string term = argv[optind];

    SearchEngine engine;
    auto outcome = engine.search(term, opts, volume,
                                 [](const SearchResult& r) {
                                     std::printf("%s\n", r.path.c_str());
                                     return true;  // keep streaming
                                 });

    if (!outcome.ok) {
        std::fprintf(stderr, "Error: %s\n", outcome.error.c_str());
        return 1;
    }
    std::fprintf(stderr, "%zu match(es).\n", outcome.results.size());
    return 0;
}
