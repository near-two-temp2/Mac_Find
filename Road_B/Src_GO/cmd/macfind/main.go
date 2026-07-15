// Command macfind is the Road_B (self-built binary index + fzf) Go
// implementation of the macOS fast file search matrix.
//
// It is a Fyne desktop GUI by default. Two CLI subcommands are provided for
// scripting and CI smoke tests:
//
//	macfind index  [-o path] [-root dir ...]   build the binary index
//	macfind search [-i path] [-n N] <pattern>  query the index
//
// With no arguments it launches the GUI (search box + result list).
package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "index":
			os.Exit(runIndexCLI(os.Args[2:]))
		case "search":
			os.Exit(runSearchCLI(os.Args[2:]))
		case "-h", "--help", "help":
			usage()
			os.Exit(0)
		case "gui":
			// Explicit GUI request; fall through.
		default:
			fmt.Fprintf(os.Stderr, "unknown command %q\n\n", os.Args[1])
			usage()
			os.Exit(2)
		}
	}
	runGUI()
}

func usage() {
	fmt.Fprint(os.Stderr, `macfind — Road_B (index + fzf) file search

usage:
  macfind                 launch the GUI
  macfind gui             launch the GUI (explicit)
  macfind index  [flags]  build the binary index
  macfind search [flags] <pattern>
                          query the index

run "macfind index -h" or "macfind search -h" for subcommand flags.
`)
}
