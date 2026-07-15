// Command macfind is the Road_C (Go) hybrid file-search desktop app.
//
// GUI (default): a Fyne window with a search box, a results list and a
// "Show in Finder" action. The backend is the hybrid engine — primary
// self-built binary index (parallel bitmask prefilter + fzf scoring), with a
// live searchfs() fallback when the index is missing/corrupt.
//
// A CLI entry (index / search / selftest) is retained for scripting and CI
// smoke tests; see cli.go.
package main

import "os"

func main() {
	if handled, code := runCLI(os.Args[1:]); handled {
		os.Exit(code)
	}
	runGUI()
}
