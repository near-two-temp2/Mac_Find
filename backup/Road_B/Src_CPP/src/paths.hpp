// paths.hpp — where the default index lives, shared by CLI and GUI.
#pragma once

#include <cstdlib>
#include <string>
#include <sys/stat.h>

namespace mff {

// Default .idx path: ~/Library/Caches/com.mff.roadb-cpp/index.idx, mirroring
// Cling's cache location convention. Creates the directory if needed.
inline std::string defaultIndexPath() {
    const char* home = std::getenv("HOME");
    std::string base = (home && home[0]) ? std::string(home) : std::string("/tmp");
    std::string dir = base + "/Library/Caches/com.mff.roadb-cpp";
    // Best-effort mkdir chain (ignore EEXIST).
    ::mkdir((base + "/Library").c_str(), 0755);
    ::mkdir((base + "/Library/Caches").c_str(), 0755);
    ::mkdir(dir.c_str(), 0755);
    return dir + "/index.idx";
}

} // namespace mff
