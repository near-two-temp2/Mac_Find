package main

import (
	"fmt"
	"sync"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/widget"

	"macfind/internal/index"
	"macfind/internal/search"
)

// guiState holds the live index and guards it against the background builder.
type guiState struct {
	mu   sync.RWMutex
	ix   *index.Index
	path string
}

func (g *guiState) get() *index.Index {
	g.mu.RLock()
	defer g.mu.RUnlock()
	return g.ix
}

func (g *guiState) set(ix *index.Index) {
	g.mu.Lock()
	old := g.ix
	g.ix = ix
	g.mu.Unlock()
	if old != nil {
		_ = old.Close()
	}
}

// runGUI builds and shows the Fyne window: a search box on top, a result list
// below, a status bar and a "Build Index" button. Searches run on every
// keystroke against the in-memory index.
func runGUI() {
	a := app.NewWithID("com.macfind.roadb.go")
	w := a.NewWindow("MacFind · Go/Fyne · Road_B")
	w.Resize(fyne.NewSize(760, 520))

	state := &guiState{}

	// Try to load an existing index at startup so search works immediately.
	if p, err := index.DefaultPath(); err == nil {
		state.path = p
		if ix, err := index.Open(p); err == nil {
			state.set(ix)
		}
	}

	// Result data model backed by a slice the list widget reads from.
	var (
		resultsMu sync.RWMutex
		results   []search.Result
	)

	list := widget.NewList(
		func() int {
			resultsMu.RLock()
			defer resultsMu.RUnlock()
			return len(results)
		},
		func() fyne.CanvasObject {
			return widget.NewLabel("")
		},
		func(id widget.ListItemID, o fyne.CanvasObject) {
			resultsMu.RLock()
			defer resultsMu.RUnlock()
			if id < 0 || id >= len(results) {
				return
			}
			r := results[id]
			prefix := "📄 "
			if r.IsDir {
				prefix = "📁 "
			}
			o.(*widget.Label).SetText(prefix + r.Path)
		},
	)

	status := widget.NewLabel("")

	// setStatus updates the status label from any goroutine. fyne.Do queues
	// the change onto the main loop (Fyne v2.6+ single-goroutine model).
	setStatus := func(s string) {
		fyne.Do(func() { status.SetText(s) })
	}

	// Initial label text is set directly: we are on the main goroutine during
	// construction and the event loop is not running yet, so fyne.Do here
	// would queue work that never runs before ShowAndRun.
	if ix := state.get(); ix != nil {
		status.SetText(fmt.Sprintf("index ready: %d entries", ix.Count))
	} else {
		status.SetText("no index — click \"Build Index\" to start")
	}

	searchBox := widget.NewEntry()
	searchBox.SetPlaceHolder("Type to search…")

	// runQuery runs the two-phase search off the UI goroutine and publishes
	// the results, then marshals the list/status refresh onto the main loop.
	// It never calls itself, so there is no nested fyne.Do.
	runQuery := func(q string) {
		ix := state.get()
		if ix == nil {
			return
		}
		start := time.Now()
		res := search.Query(ix, q, search.Options{Limit: 500})
		elapsed := time.Since(start)

		resultsMu.Lock()
		results = res
		resultsMu.Unlock()

		fyne.Do(func() {
			list.Refresh()
			list.ScrollToTop()
			status.SetText(fmt.Sprintf("%d results in %s (%d indexed)",
				len(res), elapsed.Round(time.Microsecond), ix.Count))
		})
	}

	searchBox.OnChanged = func(q string) {
		go runQuery(q)
	}

	// Build Index button: scans the default roots in the background.
	var buildBtn *widget.Button
	buildBtn = widget.NewButton("Build Index", func() {
		buildBtn.Disable()
		setStatus("building index…")
		go func() {
			start := time.Now()
			ix, err := index.Build(index.BuildOptions{
				Roots: index.DefaultRoots(),
				Progress: func(n int) {
					setStatus(fmt.Sprintf("building index… %d entries", n))
				},
			})
			if err != nil {
				setStatus("index build failed: " + err.Error())
				fyne.Do(buildBtn.Enable)
				return
			}
			// Persist so the next launch loads instantly.
			if state.path != "" {
				_ = ix.Save(state.path)
			}
			state.set(ix)
			fyne.Do(func() {
				buildBtn.Enable()
				status.SetText(fmt.Sprintf("index ready: %d entries (%s)",
					ix.Count, time.Since(start).Round(time.Millisecond)))
			})
			// Re-run the current query against the fresh index (own goroutine,
			// its own fyne.Do — not nested inside the block above).
			go runQuery(searchBox.Text)
		}()
	})

	top := container.NewBorder(nil, nil, nil, buildBtn, searchBox)
	content := container.NewBorder(top, status, nil, nil, list)
	w.SetContent(content)

	w.ShowAndRun()
}
