// Package engine is the Road_C hybrid search orchestrator: it queries the
// self-built binary index first (fast, fuzzy) and transparently degrades to the
// real-time searchfs() syscall when the index is missing or corrupt, per the
// recommended hybrid architecture (open-source-analysis.md §5.4).
package engine

import (
	"macfind/roadc/internal/index"
	"macfind/roadc/internal/searchfs"
)

// Source labels which engine produced a result set, so the UI can show the user
// whether they're seeing indexed or live results.
type Source int

const (
	SourceNone     Source = iota
	SourceIndex           // primary: self-built binary index
	SourceSearchFS        // fallback: live searchfs() scan
)

func (s Source) String() string {
	switch s {
	case SourceIndex:
		return "index"
	case SourceSearchFS:
		return "searchfs (fallback)"
	default:
		return "none"
	}
}

// Result is one hit surfaced to callers, normalized across both engines.
type Result struct {
	Path  string
	IsDir bool
	Score int
}

// Engine holds the loaded index (if any). It is safe to call Search
// concurrently; Reload swaps the index pointer under the caller's control.
type Engine struct {
	idx *index.Index
}

// New attempts to open the index at path. A load failure is not fatal: the
// engine still works via the searchfs fallback, and Reload can attach an index
// once one has been built.
func New(indexPath string) *Engine {
	e := &Engine{}
	if ix, err := index.Open(indexPath); err == nil {
		e.idx = ix
	}
	return e
}

// HasIndex reports whether a usable index is loaded.
func (e *Engine) HasIndex() bool { return e.idx != nil }

// IndexCount returns the number of indexed entries, or 0 if none loaded.
func (e *Engine) IndexCount() int {
	if e.idx == nil {
		return 0
	}
	return e.idx.Count()
}

// Reload replaces the current index with the one at path. It returns an error
// if the file can't be opened/validated, leaving the previous index in place.
func (e *Engine) Reload(indexPath string) error {
	ix, err := index.Open(indexPath)
	if err != nil {
		return err
	}
	e.idx = ix
	return nil
}

// Search runs the hybrid query. When an index is loaded it is used and results
// are labeled SourceIndex; otherwise it falls back to a live searchfs() scan
// labeled SourceSearchFS. limit <= 0 means "engine default".
func (e *Engine) Search(query string, limit int) ([]Result, Source) {
	if e.idx != nil {
		hits := e.idx.Search(query, limit)
		if len(hits) > 0 {
			out := make([]Result, len(hits))
			for i, h := range hits {
				out[i] = Result{Path: h.Path, IsDir: h.IsDir, Score: h.Score}
			}
			return out, SourceIndex
		}
		// Index loaded but empty result: still report SourceIndex (a genuine
		// "no match"), rather than pounding the catalog with a live scan.
		return nil, SourceIndex
	}
	return e.fallback(query, limit), SourceSearchFS
}

// fallback performs a live searchfs() scan.
func (e *Engine) fallback(query string, limit int) []Result {
	hits := searchfs.Search(query, searchfs.Options{Limit: limit})
	out := make([]Result, len(hits))
	for i, h := range hits {
		out[i] = Result{Path: h.Path, IsDir: h.IsDir}
	}
	return out
}
