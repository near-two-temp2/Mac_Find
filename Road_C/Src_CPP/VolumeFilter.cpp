// VolumeFilter.cpp — network/FUSE-mount exclusion for the index walk.
#include "VolumeFilter.h"

#include <array>
#include <cstring>

#include <sys/param.h>
#include <sys/mount.h>   // statfs, MNT_LOCAL, f_fstypename
#include <unistd.h>
#include <pwd.h>

namespace macfind {

namespace {

std::string homeDir() {
    if (const char* h = getenv("HOME")) return h;
    if (const passwd* pw = getpwuid(getuid())) return pw->pw_dir;
    return "/";
}

// Case-insensitive equality for short filesystem type names.
bool eqIgnoreCase(const char* a, const char* b) {
    return ::strcasecmp(a, b) == 0;
}

// Local catalog filesystems we're happy to index. Everything else — network
// (nfs/smbfs/afpfs/webdav/cifs), FUSE (macfuse/osxfuse), FileProvider, etc. —
// is rejected. We whitelist rather than blacklist so an unfamiliar network fs
// type can never slip through.
bool isLocalCatalogFsType(const char* fstype) {
    return eqIgnoreCase(fstype, "apfs") || eqIgnoreCase(fstype, "hfs");
}

// Does `path` start with `prefix`, at a path-component boundary? So "/a/b"
// matches prefix "/a" (and "/a" itself) but "/ab" does not.
bool hasPathPrefix(const std::string& path, const std::string& prefix) {
    if (prefix.empty()) return false;
    if (path.compare(0, prefix.size(), prefix) != 0) return false;
    return path.size() == prefix.size() || path[prefix.size()] == '/';
}

}  // namespace

bool isExplicitlyExcluded(const std::string& path) {
    // Known rclone→Backblaze B2 mounts on this machine (project CLAUDE.md +
    // SEARCH_TEST_BASELINE.md). Pruning these is what keeps us from burning B2
    // Class-C transaction quota.
    static const std::array<const char*, 3> kBadMounts = {
        "/Volumes/Disk/h2-bu-01",
        "/Volumes/Disk/h2_bu_01_b2",
        "/Volumes/Disk/h2_open_rsh",
    };
    for (const char* bad : kBadMounts) {
        if (hasPathPrefix(path, bad)) return true;
    }

    // Any h2-* sibling that might appear under /Volumes/Disk in the future:
    // treat the whole rclone parent conservatively by name prefix.
    static const std::string kDiskH2 = "/Volumes/Disk/h2-";
    static const std::string kDiskH2u = "/Volumes/Disk/h2_";
    if (path.compare(0, kDiskH2.size(), kDiskH2) == 0 ||
        path.compare(0, kDiskH2u.size(), kDiskH2u) == 0) {
        return true;
    }

    // Cloud FileProvider dirs — deep-walking these is slow and pointless.
    const std::string cloud = homeDir() + "/Library/CloudStorage";
    if (hasPathPrefix(path, cloud)) return true;

    return false;
}

bool mountIsIndexable(const std::string& mountPath) {
    if (isExplicitlyExcluded(mountPath)) return false;

    struct statfs sb;
    if (::statfs(mountPath.c_str(), &sb) != 0) {
        // If we can't even stat the mount, don't risk descending into it.
        return false;
    }

    // Must be a locally-attached volume …
    if ((sb.f_flags & MNT_LOCAL) == 0) return false;
    // … and one of our known-safe catalog filesystems.
    if (!isLocalCatalogFsType(sb.f_fstypename)) return false;

    return true;
}

}  // namespace macfind
