// VolumeFilter.h — decide which filesystem paths the indexer is allowed to walk.
//
// The single most important rule for Road_C (see ../../SEARCH_TEST_BASELINE.md
// §"索引构建硬性要求"): the index build must NEVER descend into a network /
// FUSE mount. On this machine there are rclone→Backblaze B2 mounts under
// /Volumes/Disk/h2-*; walking them is slow, can hang, and burns paid B2 API
// quota. So we prune at every mount boundary that isn't a plain local volume.
//
// Three complementary defenses, cheapest first:
//   1. Don't cross device boundaries during the walk (compare st_dev, like
//      FTS_XDEV) — a mount is a different device, so we naturally stop at it and
//      only re-enter if the child mount passes the checks below.
//   2. statfs() the mount point: keep only local catalog filesystems
//      (apfs / hfs); reject macfuse / nfs / smbfs / afpfs / webdav / etc., and
//      require the MNT_LOCAL flag.
//   3. An explicit blocklist of known-bad paths as a belt-and-suspenders guard
//      (the h2-* rclone mounts and ~/Library/CloudStorage/*), in case a mount's
//      advertised type ever looks local.
#pragma once

#include <string>

namespace macfind {

// True if `path` is a mount point we are allowed to index (local apfs/hfs, not
// on any blocklist). Used to decide whether to descend into a newly crossed
// mount during the fts walk. Non-mount paths never reach this — they inherit
// their parent volume's decision by staying on the same device.
bool mountIsIndexable(const std::string& mountPath);

// True if `path` matches a hard-coded exclusion (rclone→B2 mounts, CloudStorage
// FileProvider dirs). Checked independently of statfs so a mislabeled mount
// still can't leak. Prefix-matched, so it also prunes anything *under* them.
bool isExplicitlyExcluded(const std::string& path);

}  // namespace macfind
