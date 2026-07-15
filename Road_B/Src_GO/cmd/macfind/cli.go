package main

import (
	"flag"
	"fmt"
	"os"
	"time"

	"macfind/internal/index"
	"macfind/internal/search"
)

// runIndexCLI implements `macfind index [-o path] [-root dir ...]`. It walks
// the roots, builds the binary index and writes it to disk. This is the CI
// smoke-test entry for the indexing half of Road_B.
func runIndexCLI(args []string) int {
	fs := flag.NewFlagSet("index", flag.ContinueOnError)
	out := fs.String("o", "", "output index path (default: cache location)")
	skipHidden := fs.Bool("skip-hidden", false, "skip dot-files and dot-dirs")
	var roots multiFlag
	fs.Var(&roots, "root", "root directory to scan (repeatable; default: home + /Applications)")
	if err := fs.Parse(args); err != nil {
		return 2
	}

	outPath := *out
	if outPath == "" {
		p, err := index.DefaultPath()
		if err != nil {
			fmt.Fprintln(os.Stderr, "index: cannot resolve default path:", err)
			return 1
		}
		outPath = p
	}

	scanRoots := []string(roots)
	if len(scanRoots) == 0 {
		scanRoots = index.DefaultRoots()
	}

	fmt.Printf("indexing %v ...\n", scanRoots)
	start := time.Now()
	ix, err := index.Build(index.BuildOptions{
		Roots:      scanRoots,
		SkipHidden: *skipHidden,
		Progress: func(n int) {
			fmt.Printf("\r  %d entries", n)
		},
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, "\nindex: build failed:", err)
		return 1
	}
	fmt.Printf("\r  %d entries scanned in %s\n", ix.Count, time.Since(start).Round(time.Millisecond))

	if err := ix.Save(outPath); err != nil {
		fmt.Fprintln(os.Stderr, "index: save failed:", err)
		return 1
	}
	fmt.Printf("wrote index to %s\n", outPath)
	return 0
}

// runSearchCLI implements `macfind search [-i path] [-n limit] <pattern>`. It
// mmaps the index and runs the two-phase query, printing scored results. This
// is the CI smoke-test entry for the search half of Road_B.
func runSearchCLI(args []string) int {
	fs := flag.NewFlagSet("search", flag.ContinueOnError)
	in := fs.String("i", "", "index path (default: cache location)")
	limit := fs.Int("n", 50, "max results")
	ext := fs.String("ext", "", "restrict to this extension (no dot)")
	dirsOnly := fs.Bool("d", false, "directories only")
	filesOnly := fs.Bool("f", false, "files only")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if fs.NArg() < 1 {
		fmt.Fprintln(os.Stderr, "usage: macfind search [flags] <pattern>")
		return 2
	}
	pattern := fs.Arg(0)

	inPath := *in
	if inPath == "" {
		p, err := index.DefaultPath()
		if err != nil {
			fmt.Fprintln(os.Stderr, "search: cannot resolve default path:", err)
			return 1
		}
		inPath = p
	}

	ix, err := index.Open(inPath)
	if err != nil {
		fmt.Fprintln(os.Stderr, "search: cannot open index:", err)
		fmt.Fprintln(os.Stderr, "  (run `macfind index` first)")
		return 1
	}
	defer ix.Close()

	start := time.Now()
	results := search.Query(ix, pattern, search.Options{
		Limit:     *limit,
		Ext:       *ext,
		DirsOnly:  *dirsOnly,
		FilesOnly: *filesOnly,
	})
	elapsed := time.Since(start)

	for _, r := range results {
		kind := "f"
		if r.IsDir {
			kind = "d"
		}
		fmt.Printf("%5d  %s  %s\n", r.Score, kind, r.Path)
	}
	fmt.Fprintf(os.Stderr, "%d results in %s (%d entries indexed)\n",
		len(results), elapsed.Round(time.Microsecond), ix.Count)
	return 0
}

// multiFlag collects a repeatable string flag.
type multiFlag []string

func (m *multiFlag) String() string { return fmt.Sprint([]string(*m)) }
func (m *multiFlag) Set(v string) error {
	*m = append(*m, v)
	return nil
}
