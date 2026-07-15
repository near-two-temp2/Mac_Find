// scanner.hpp — filesystem traversal that feeds the index builder.
//
// Uses fts(3) with FTS_NOSTAT (Cling does the same, §3.3 step 2) so the walk
// leans on getattrlistbulk internally and avoids a stat() per entry. We report
// each path plus whether it is a directory to a caller-supplied sink.

#pragma once

#include <functional>
#include <string>
#include <vector>

namespace mff {

// Called once per discovered path. `path` is the absolute path, `isDir` marks
// directories.
using ScanSink = std::function<void(const std::string& path, bool isDir)>;

// Walk `roots` and invoke `sink` for every entry. Skips a small set of noisy
// system caches by default so the demo index stays useful and fast. Returns the
// number of entries reported.
size_t scanRoots(const std::vector<std::string>& roots, const ScanSink& sink);

// Convenience: the default roots to index for a user-scope build. Falls back to
// $HOME; callers may override.
std::vector<std::string> defaultRoots();

} // namespace mff
