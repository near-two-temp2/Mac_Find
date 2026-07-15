package main

import (
	"fmt"
	"os/exec"
	"sync"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/container"
	"fyne.io/fyne/v2/widget"

	"macfind/roadc/internal/engine"
	"macfind/roadc/internal/index"
)

// guiState holds the widgets and current result set shared between the search
// callback and the list data source.
type guiState struct {
	eng      *engine.Engine
	win      fyne.Window
	status   *widget.Label
	list     *widget.List
	entry    *widget.Entry

	mu       sync.Mutex
	results  []engine.Result
	selected int
	seq      uint64 // debounce/staleness guard for async searches
}

// runGUI builds and shows the main window. The hybrid engine loads whatever
// index exists; if none does, searches transparently use the searchfs fallback
// and the user can trigger a background "Build Index" from the toolbar.
func runGUI() {
	a := app.NewWithID("com.macfind.roadc.go")
	w := a.NewWindow("MacFind · Go/Fyne · Road_C")
	w.Resize(fyne.NewSize(820, 560))

	st := &guiState{
		eng:      engine.New(index.DefaultPath()),
		win:      w,
		selected: -1,
	}

	st.entry = widget.NewEntry()
	st.entry.SetPlaceHolder("Type to search files… (index-first, searchfs fallback)")

	st.status = widget.NewLabel("")
	st.updateStatusIdle()

	st.list = widget.NewList(
		func() int {
			st.mu.Lock()
			defer st.mu.Unlock()
			return len(st.results)
		},
		func() fyne.CanvasObject { return widget.NewLabel("") },
		func(id widget.ListItemID, o fyne.CanvasObject) {
			st.mu.Lock()
			defer st.mu.Unlock()
			if id < 0 || id >= len(st.results) {
				return
			}
			r := st.results[id]
			prefix := "📄 "
			if r.IsDir {
				prefix = "📁 "
			}
			o.(*widget.Label).SetText(prefix + r.Path)
		},
	)
	st.list.OnSelected = func(id widget.ListItemID) {
		st.mu.Lock()
		st.selected = int(id)
		st.mu.Unlock()
	}

	// Debounced live search on every keystroke.
	st.entry.OnChanged = func(q string) { st.search(q) }
	st.entry.OnSubmitted = func(q string) { st.search(q) }

	showBtn := widget.NewButton("Show in Finder", st.showSelectedInFinder)
	buildBtn := widget.NewButton("Build / Rebuild Index", st.buildIndexAsync)

	toolbar := container.NewHBox(showBtn, buildBtn)
	top := container.NewVBox(st.entry, toolbar, st.status)
	content := container.NewBorder(top, nil, nil, nil, st.list)
	w.SetContent(content)

	w.Canvas().Focus(st.entry)
	w.ShowAndRun()
}

// search runs a query on a background goroutine and marshals the result back to
// the UI thread. A monotonically increasing sequence number discards stale
// responses when the user keeps typing.
func (st *guiState) search(query string) {
	st.mu.Lock()
	st.seq++
	mySeq := st.seq
	st.mu.Unlock()

	if query == "" {
		st.mu.Lock()
		st.results = nil
		st.selected = -1
		st.mu.Unlock()
		// OnChanged runs on the UI goroutine, so update widgets directly here.
		st.list.UnselectAll()
		st.list.Refresh()
		st.updateStatusIdle()
		return
	}

	go func() {
		start := time.Now()
		results, src := st.eng.Search(query, 500)
		elapsed := time.Since(start)

		st.mu.Lock()
		if mySeq != st.seq { // a newer search superseded this one
			st.mu.Unlock()
			return
		}
		st.results = results
		st.selected = -1
		count := len(results)
		st.mu.Unlock()

		// Marshal widget updates back onto the UI goroutine (Fyne v2.6+).
		fyne.Do(func() {
			st.list.UnselectAll()
			st.list.Refresh()
			st.status.SetText(fmt.Sprintf("%d results · engine: %s · %s",
				count, src, elapsed.Round(time.Millisecond)))
		})
	}()
}

func (st *guiState) updateStatusIdle() {
	if st.eng.HasIndex() {
		st.status.SetText(fmt.Sprintf("Ready · index loaded (%d entries) · fuzzy search",
			st.eng.IndexCount()))
	} else {
		st.status.SetText("Ready · no index · using live searchfs() fallback (build an index for fuzzy search)")
	}
}

// showSelectedInFinder reveals the highlighted result in Finder via
// `open -R <path>`.
func (st *guiState) showSelectedInFinder() {
	st.mu.Lock()
	sel := st.selected
	var path string
	if sel >= 0 && sel < len(st.results) {
		path = st.results[sel].Path
	}
	st.mu.Unlock()

	if path == "" {
		st.status.SetText("Select a result first, then click Show in Finder.")
		return
	}
	if err := exec.Command("open", "-R", path).Start(); err != nil {
		st.status.SetText("Failed to reveal in Finder: " + err.Error())
	}
}

// buildIndexAsync rebuilds the binary index over the default roots on a
// background goroutine, then reloads it into the engine. The button disables
// itself while running.
func (st *guiState) buildIndexAsync() {
	// Called from the button tap on the UI goroutine — safe to set directly.
	st.status.SetText("Building index… (scanning filesystem, this may take a while)")
	go func() {
		roots := index.DefaultRoots()
		out := index.DefaultPath()
		start := time.Now()
		n, err := index.Build(roots, out)
		if err != nil {
			fyne.Do(func() { st.status.SetText("Index build failed: " + err.Error()) })
			return
		}
		reloadErr := st.eng.Reload(out)
		fyne.Do(func() {
			if reloadErr != nil {
				st.status.SetText("Built index but reload failed: " + reloadErr.Error())
				return
			}
			st.status.SetText(fmt.Sprintf("Index ready · %d entries · %s",
				n, time.Since(start).Round(time.Millisecond)))
		})
	}()
}
