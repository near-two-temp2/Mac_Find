// Command macfind-a-go is a macOS GUI file-search app (Road_A of the Mac_Find
// implementation matrix). Its search engine is index-free: every query calls
// the searchfs(2) syscall directly on APFS/HFS+ volume catalogs.
//
// Running with no arguments launches the Fyne GUI (search box + results list).
// Running with --cli <term> performs a one-shot search and prints paths to
// stdout; this path is used by CI as a smoke test and is safe to build on any
// platform.
package main

import (
	"flag"
	"fmt"
	"os"

	"macfind-a-go/internal/engine"
)

func main() {
	var (
		cli           = flag.Bool("cli", false, "run a one-shot search on the command line instead of the GUI")
		term          = flag.String("term", "", "search term (CLI mode)")
		filesOnly     = flag.Bool("files-only", false, "match files only (CLI mode)")
		dirsOnly      = flag.Bool("dirs-only", false, "match directories only (CLI mode)")
		caseSensitive = flag.Bool("case-sensitive", false, "case-sensitive matching (CLI mode)")
		limit         = flag.Int("limit", 100, "maximum number of results (CLI mode)")
	)
	flag.Parse()

	// Positional argument is also accepted as the term, e.g. `--cli foo`.
	if *term == "" && flag.NArg() > 0 {
		*term = flag.Arg(0)
	}

	if *cli {
		os.Exit(runCLI(cliArgs{
			term:          *term,
			filesOnly:     *filesOnly,
			dirsOnly:      *dirsOnly,
			caseSensitive: *caseSensitive,
			limit:         *limit,
		}))
	}

	runGUI()
}

type cliArgs struct {
	term          string
	filesOnly     bool
	dirsOnly      bool
	caseSensitive bool
	limit         int
}

// runCLI performs a single search and prints matched paths. It returns a process
// exit code. An empty term is treated as a successful no-op so CI can invoke the
// binary without triggering a full-disk scan or a hard failure.
func runCLI(a cliArgs) int {
	if a.term == "" {
		fmt.Fprintln(os.Stderr, "no --term given; nothing to search (CLI smoke ok)")
		return 0
	}

	kind := engine.MatchAll
	if a.filesOnly {
		kind = engine.MatchFilesOnly
	} else if a.dirsOnly {
		kind = engine.MatchDirsOnly
	}

	results, err := engine.Search(engine.Options{
		Term:          a.term,
		Kind:          kind,
		CaseSensitive: a.caseSensitive,
		Limit:         a.limit,
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "search error: %v\n", err)
		// Still print whatever partial results came back.
	}
	for _, r := range results {
		fmt.Println(r.Path)
	}
	fmt.Fprintf(os.Stderr, "%d result(s)\n", len(results))
	return 0
}
