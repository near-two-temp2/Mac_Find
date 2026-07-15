package search

import (
	"runtime"
	"sort"
	"strings"
	"sync"

	"macfind/internal/index"
)

// Result is one scored match, ready for display.
type Result struct {
	Path  string // original-case path is not stored; this is the lowercase path
	Score int
	IsDir bool
}

// Options tunes a query.
type Options struct {
	Limit    int    // max results returned (0 = 200)
	Ext      string // if non-empty, require this extension (without dot)
	DirsOnly bool
	FilesOnly bool
}

// Query runs the two-phase search over ix and returns up to Limit results
// sorted by descending score (ties broken by shorter path, then path bytes).
func Query(ix *index.Index, pattern string, opts Options) []Result {
	limit := opts.Limit
	if limit <= 0 {
		limit = 200
	}

	pat := strings.ToLower(strings.TrimSpace(pattern))
	patBytes := []byte(pat)
	combinedMask := index.MaskForString(pat)

	// Resolve the extension filter to an interned id if the index knows it.
	// An unknown extension id means "no entry can match" — handled below.
	var extID uint32
	extFilter := opts.Ext != ""
	extKnown := false
	if extFilter {
		want := strings.ToLower(strings.TrimPrefix(opts.Ext, "."))
		for id := uint32(0); int(id) < ix.ExtCount(); id++ {
			if ix.ExtName(id) == want {
				extID = id
				extKnown = true
				break
			}
		}
		if !extKnown {
			return nil
		}
	}

	// Phase 1 + 2 fused across goroutines: each worker filters its shard by
	// bitmask/extension and scores survivors, keeping a local result slice.
	workers := runtime.NumCPU()
	if workers < 1 {
		workers = 1
	}
	if ix.Count < workers*256 {
		workers = 1 // small index: parallelism isn't worth the overhead
	}

	shards := make([][]Result, workers)
	var wg sync.WaitGroup
	chunk := (ix.Count + workers - 1) / workers

	for w := 0; w < workers; w++ {
		lo := w * chunk
		hi := lo + chunk
		if hi > ix.Count {
			hi = ix.Count
		}
		if lo >= hi {
			continue
		}
		wg.Add(1)
		go func(w, lo, hi int) {
			defer wg.Done()
			shards[w] = scanShard(ix, lo, hi, patBytes, combinedMask,
				extFilter, extID, opts)
		}(w, lo, hi)
	}
	wg.Wait()

	// Merge shard results.
	var all []Result
	for _, s := range shards {
		all = append(all, s...)
	}

	sort.Slice(all, func(i, j int) bool {
		if all[i].Score != all[j].Score {
			return all[i].Score > all[j].Score
		}
		if len(all[i].Path) != len(all[j].Path) {
			return len(all[i].Path) < len(all[j].Path)
		}
		return all[i].Path < all[j].Path
	})

	if len(all) > limit {
		all = all[:limit]
	}
	return all
}

// scanShard runs Phase 1 (cheap rejects) and Phase 2 (fzf score) over entries
// [lo,hi) and returns the survivors it scored.
func scanShard(ix *index.Index, lo, hi int, patBytes []byte, combinedMask uint64,
	extFilter bool, extID uint32, opts Options) []Result {

	var out []Result
	emptyPattern := len(patBytes) == 0

	for i := lo; i < hi; i++ {
		isDir := ix.IsDirs[i] == 1
		if opts.DirsOnly && !isDir {
			continue
		}
		if opts.FilesOnly && isDir {
			continue
		}
		if extFilter && ix.ExtIDs[i] != extID {
			continue
		}

		// Phase 1: bitmask pre-filter. If the entry's path mask is missing any
		// bit the query needs, it cannot contain the pattern — reject in O(1).
		if !emptyPattern && ix.Masks[i]&combinedMask != combinedMask {
			continue
		}

		// Phase 2: fzf scoring. Score against the basename first (matches
		// there are what users usually mean), falling back to the full path.
		path := ix.Path(i)
		var score int
		if emptyPattern {
			score = 1
		} else {
			bn := ix.Basename(i)
			s, ok := fuzzyScore(patBytes, bn)
			if !ok {
				s, ok = fuzzyScore(patBytes, path)
				if !ok {
					continue
				}
			} else {
				// Basename hits get a bump so file matches outrank incidental
				// path-component matches.
				s += 12
			}
			score = s
		}

		out = append(out, Result{
			Path:  string(path),
			Score: score,
			IsDir: isDir,
		})
	}
	return out
}
