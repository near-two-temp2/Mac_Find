package main

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"macfind/roadc/internal/engine"
	"macfind/roadc/internal/index"
)

// runCLI handles the non-GUI subcommands used for scripting and CI smoke tests:
//
//	index [root...]     build the binary index (defaults to $HOME + /Applications)
//	search <query>      query via the hybrid engine, print ranked paths
//	selftest            build a tiny throwaway index and query it (no GUI, no root)
//
// It returns true if it handled the arguments (so main should exit), along with
// the process exit code.
func runCLI(args []string) (handled bool, code int) {
	if len(args) == 0 {
		return false, 0
	}
	switch args[0] {
	case "index":
		return true, cmdIndex(args[1:])
	case "search":
		return true, cmdSearch(args[1:])
	case "selftest":
		return true, cmdSelftest()
	case "gui":
		// Explicit GUI request: let main fall through to launch it.
		return false, 0
	default:
		return false, 0
	}
}

func cmdIndex(roots []string) int {
	if len(roots) == 0 {
		roots = index.DefaultRoots()
	}
	out := index.DefaultPath()
	start := time.Now()
	n, err := index.Build(roots, out)
	if err != nil {
		fmt.Fprintf(os.Stderr, "index: build failed: %v\n", err)
		return 1
	}
	fmt.Printf("indexed %d entries from %v in %s -> %s\n", n, roots, time.Since(start).Round(time.Millisecond), out)
	return 0
}

func cmdSearch(args []string) int {
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, "usage: macfind search <query>")
		return 2
	}
	query := args[0]
	eng := engine.New(index.DefaultPath())
	results, src := eng.Search(query, 50)
	fmt.Printf("# source=%s hits=%d\n", src, len(results))
	for _, r := range results {
		mark := "f"
		if r.IsDir {
			mark = "d"
		}
		fmt.Printf("%s\t%d\t%s\n", mark, r.Score, r.Path)
	}
	return 0
}

// cmdSelftest is the CI smoke test: it never touches the real index or requires
// searchfs privileges. It seeds a temp tree with known files, indexes it, and
// confirms the hybrid engine returns results — exercising the index build,
// load, bitmask prefilter and fzf scoring in one pass. Being self-contained
// (rather than relying on the working directory) keeps it deterministic.
func cmdSelftest() int {
	tmp, err := os.MkdirTemp("", "macfind-selftest-*")
	if err != nil {
		fmt.Fprintf(os.Stderr, "selftest: %v\n", err)
		return 1
	}
	defer os.RemoveAll(tmp)

	// Seed known files so the query has a guaranteed match.
	seed := []string{"hello_world.txt", "sub/report.md", "sub/deep/alpha.go"}
	for _, f := range seed {
		p := filepath.Join(tmp, f)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			fmt.Fprintf(os.Stderr, "selftest: %v\n", err)
			return 1
		}
		if err := os.WriteFile(p, []byte("x"), 0o644); err != nil {
			fmt.Fprintf(os.Stderr, "selftest: %v\n", err)
			return 1
		}
	}

	idxPath := filepath.Join(tmp, "selftest.idx")
	n, err := index.Build([]string{tmp}, idxPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "selftest: build failed: %v\n", err)
		return 1
	}
	if n == 0 {
		fmt.Fprintln(os.Stderr, "selftest: indexed 0 entries")
		return 1
	}

	eng := engine.New(idxPath)
	if !eng.HasIndex() {
		fmt.Fprintln(os.Stderr, "selftest: index failed to load")
		return 1
	}
	results, src := eng.Search("report", 10)
	fmt.Printf("selftest OK: built %d entries, query 'report' -> %d hits via %s\n", n, len(results), src)
	if len(results) == 0 {
		fmt.Fprintln(os.Stderr, "selftest: expected at least one hit for 'report'")
		return 1
	}
	return 0
}
