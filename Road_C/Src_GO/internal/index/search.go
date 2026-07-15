package index

import (
	"encoding/binary"
	"errors"
	"os"
	"runtime"
	"sort"
	"strings"
	"sync"

	"macfind/roadc/internal/bitmask"
	"macfind/roadc/internal/fuzzy"
)

// ErrBadIndex means the file is missing, truncated or has a wrong magic. The
// caller should fall back to searchfs().
var ErrBadIndex = errors.New("index: missing or corrupt")

// Match is one search hit surfaced to the UI/CLI.
type Match struct {
	Path  string
	IsDir bool
	Score int
}

// Index is a loaded, read-only binary index. The whole file lives in `data`;
// entries are decoded on demand as sub-slices, so per-entry access is
// allocation-free (Go's stdlib has no mmap, but an in-memory []byte gives the
// same random-access, zero-copy behavior for our lookups).
type Index struct {
	data  []byte
	count int
	blob  []byte
}

// Open loads (and validates) the index at path. Go doesn't expose mmap in the
// stdlib; reading the file into memory yields the same random-access, zero-copy
// sub-slice behavior for our purposes.
func Open(path string) (*Index, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, ErrBadIndex
	}
	if len(data) < headerSize {
		return nil, ErrBadIndex
	}
	if string(data[0:8]) != string(magicBytes) {
		return nil, ErrBadIndex
	}
	if binary.LittleEndian.Uint32(data[8:12]) != formatVer {
		return nil, ErrBadIndex
	}
	count := int(binary.LittleEndian.Uint32(data[12:16]))
	bytesLen := int(binary.LittleEndian.Uint64(data[16:24]))

	arraysEnd := headerSize + count*entryStride
	if arraysEnd < 0 || arraysEnd+bytesLen != len(data) {
		return nil, ErrBadIndex // size mismatch => corrupt/truncated
	}

	ix := &Index{data: data, count: count}
	// Views are computed lazily per-entry via helpers below; we only need the
	// blob boundary here.
	ix.blob = data[arraysEnd : arraysEnd+bytesLen]
	return ix, nil
}

// Count returns the number of indexed entries.
func (ix *Index) Count() int { return ix.count }

// entryAt decodes the i-th parallel-array record without allocating.
func (ix *Index) entryAt(i int) (mask, bnMask uint64, path string, bnStart uint32, isDir bool) {
	p := headerSize + i*entryStride
	mask = binary.LittleEndian.Uint64(ix.data[p:])
	bnMask = binary.LittleEndian.Uint64(ix.data[p+8:])
	off := binary.LittleEndian.Uint32(ix.data[p+16:])
	length := binary.LittleEndian.Uint32(ix.data[p+20:])
	bnStart = binary.LittleEndian.Uint32(ix.data[p+24:])
	isDir = ix.data[p+28] == 1
	path = ix.blobString(off, length)
	return
}

func (ix *Index) blobString(off, length uint32) string {
	return string(ix.blob[off : off+length])
}

// Search runs the two-phase query (open-source-analysis.md §3.4):
//   - Phase 1: parallel O(n) bitmask pre-filter across goroutines.
//   - Phase 2: fzf scoring of survivors, sorted by score.
//
// The query is matched against the lowercased basename first (the common case),
// falling back to the full path so that path-fragment queries still work.
func (ix *Index) Search(query string, limit int) []Match {
	q := strings.ToLower(strings.TrimSpace(query))
	if q == "" {
		return nil
	}
	qMask := bitmask.Of(q)

	workers := runtime.NumCPU()
	if workers < 1 {
		workers = 1
	}
	chunk := (ix.count + workers - 1) / workers

	var (
		mu      sync.Mutex
		results []Match
		wg      sync.WaitGroup
	)

	for w := 0; w < workers; w++ {
		start := w * chunk
		end := start + chunk
		if start >= ix.count {
			break
		}
		if end > ix.count {
			end = ix.count
		}
		wg.Add(1)
		go func(start, end int) {
			defer wg.Done()
			local := make([]Match, 0, 64)
			for i := start; i < end; i++ {
				mask, bnMask, path, bnStart, isDir := ix.entryAt(i)
				// Phase 1: cheap bitmask rejection against the whole path.
				if !bitmask.Matches(mask, qMask) {
					continue
				}
				// Phase 2: prefer a basename match; fall back to full path.
				bnPath := path[bnStart:]
				var res fuzzy.Result
				var ok bool
				if bitmask.Matches(bnMask, qMask) {
					res, ok = fuzzy.Match(q, bnPath)
					if ok {
						res.Score += 20 // basename hits rank above deep-path hits
					}
				}
				if !ok {
					res, ok = fuzzy.Match(q, path)
				}
				if ok {
					local = append(local, Match{Path: path, IsDir: isDir, Score: res.Score})
				}
			}
			if len(local) > 0 {
				mu.Lock()
				results = append(results, local...)
				mu.Unlock()
			}
		}(start, end)
	}
	wg.Wait()

	sort.SliceStable(results, func(a, b int) bool {
		if results[a].Score != results[b].Score {
			return results[a].Score > results[b].Score
		}
		return len(results[a].Path) < len(results[b].Path)
	})
	if limit > 0 && len(results) > limit {
		results = results[:limit]
	}
	return results
}
