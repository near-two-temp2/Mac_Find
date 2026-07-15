package main

import (
	"fmt"
	"strconv"
	"sync/atomic"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/data/binding"
	"fyne.io/fyne/v2/layout"
	"fyne.io/fyne/v2/widget"

	"macfind-a-go/internal/engine"
)

// runGUI builds and shows the Fyne window. The layout is the shared Mac_Find
// shell: a search box + option row on top, a scrollable results list below,
// and a status line at the bottom.
func runGUI() {
	a := app.New()
	w := a.NewWindow("MacFind · Go/Fyne · Road_A")
	w.Resize(fyne.NewSize(760, 560))

	// --- widgets -----------------------------------------------------------
	searchEntry := widget.NewEntry()
	searchEntry.SetPlaceHolder("Type a filename substring and press Enter…")

	filesOnly := widget.NewCheck("Files only", nil)
	dirsOnly := widget.NewCheck("Dirs only", nil)
	// Files-only and dirs-only are mutually exclusive.
	filesOnly.OnChanged = func(on bool) {
		if on && dirsOnly.Checked {
			dirsOnly.SetChecked(false)
		}
	}
	dirsOnly.OnChanged = func(on bool) {
		if on && filesOnly.Checked {
			filesOnly.SetChecked(false)
		}
	}
	caseSensitive := widget.NewCheck("Case sensitive", nil)

	limitEntry := widget.NewEntry()
	limitEntry.SetText("500")

	// Results are held in a bound string slice so the List stays in sync.
	resultsData := binding.NewStringList()
	resultList := widget.NewListWithData(
		resultsData,
		func() fyne.CanvasObject {
			return widget.NewLabel("")
		},
		func(item binding.DataItem, obj fyne.CanvasObject) {
			s, _ := item.(binding.String).Get()
			lbl := obj.(*widget.Label)
			lbl.SetText(s)
			lbl.Truncation = fyne.TextTruncateEllipsis
		},
	)

	// The status line is a bound string. In Fyne v2.5 data bindings are the
	// supported way to mutate UI state from a background goroutine: the binding
	// framework marshals the change onto the main event loop for us (there is
	// no fyne.Do until v2.6).
	statusData := binding.NewString()
	statusData.Set("Ready.")
	status := widget.NewLabelWithData(statusData)

	// searchGen guards against out-of-order results: each search bumps the
	// generation, and a stale goroutine's results are dropped.
	var searchGen int64

	doSearch := func() {
		term := searchEntry.Text
		if term == "" {
			resultsData.Set(nil)
			statusData.Set("Enter a search term.")
			return
		}

		kind := engine.MatchAll
		if filesOnly.Checked {
			kind = engine.MatchFilesOnly
		} else if dirsOnly.Checked {
			kind = engine.MatchDirsOnly
		}

		limit := 500
		if v, err := strconv.Atoi(limitEntry.Text); err == nil && v > 0 {
			limit = v
		}

		gen := atomic.AddInt64(&searchGen, 1)
		statusData.Set("Searching…")

		go func() {
			results, err := engine.Search(engine.Options{
				Term:          term,
				Kind:          kind,
				CaseSensitive: caseSensitive.Checked,
				Limit:         limit,
			})

			// Drop results from a superseded search.
			if atomic.LoadInt64(&searchGen) != gen {
				return
			}

			paths := make([]string, 0, len(results))
			for _, r := range results {
				if r.IsDir {
					paths = append(paths, "📁 "+r.Path)
				} else {
					paths = append(paths, "📄 "+r.Path)
				}
			}

			// Bindings are goroutine-safe: they queue the update onto the UI loop.
			resultsData.Set(paths)
			if err != nil {
				statusData.Set(fmt.Sprintf("%d result(s) — %v", len(paths), err))
			} else {
				statusData.Set(fmt.Sprintf("%d result(s).", len(paths)))
			}
		}()
	}

	searchEntry.OnSubmitted = func(string) { doSearch() }
	searchButton := widget.NewButton("Search", doSearch)

	// --- layout ------------------------------------------------------------
	optionsRow := container.NewHBox(
		filesOnly,
		dirsOnly,
		caseSensitive,
		widget.NewLabel("Limit:"),
		container.NewGridWrap(fyne.NewSize(80, 36), limitEntry),
	)

	top := container.NewVBox(
		container.NewBorder(nil, nil, nil, searchButton, searchEntry),
		optionsRow,
		widget.NewSeparator(),
	)

	content := container.NewBorder(
		top,
		container.NewVBox(widget.NewSeparator(), status),
		nil, nil,
		container.New(layout.NewMaxLayout(), resultList),
	)

	w.SetContent(content)
	w.Canvas().Focus(searchEntry)
	w.ShowAndRun()
}
