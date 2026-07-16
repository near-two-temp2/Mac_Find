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
	cjktheme "macfind/roadc/internal/theme"
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
	building bool   // true while an index build is in flight (auto or manual)
}

// runGUI builds and shows the main window. The hybrid engine loads whatever
// index exists; if none does, searches transparently use the searchfs fallback
// and the user can trigger a background "Build Index" from the toolbar.
func runGUI() {
	a := app.NewWithID("com.macfind.roadc.go")
	// Use a CJK-capable font so Chinese UI text and Chinese file paths render as
	// glyphs instead of tofu (□) boxes.
	a.Settings().SetTheme(cjktheme.New())
	w := a.NewWindow("MacFind · Go/Fyne · Road_C")
	w.Resize(fyne.NewSize(820, 560))

	st := &guiState{
		eng:      engine.New(index.DefaultPath()),
		win:      w,
		selected: -1,
	}

	st.entry = widget.NewEntry()
	st.entry.SetPlaceHolder("输入以搜索文件… / Type to search (index-first, searchfs fallback)")

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
			// Use text markers rather than emoji: the embedded CJK font has no
			// emoji glyphs, and the custom theme routes every text style through
			// it, so 📄/📁 would render as tofu boxes.
			prefix := "[文件] "
			if r.IsDir {
				prefix = "[目录] "
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

	showBtn := widget.NewButton("在访达中显示 / Show in Finder", st.showSelectedInFinder)
	buildBtn := widget.NewButton("建立 / 重建索引 / Rebuild Index", st.buildIndexAsync)

	toolbar := container.NewHBox(showBtn, buildBtn)
	top := container.NewVBox(st.entry, toolbar, st.status)
	content := container.NewBorder(top, nil, nil, nil, st.list)
	w.SetContent(content)

	w.Canvas().Focus(st.entry)

	// First-launch UX: if no index exists yet, build one automatically in the
	// background instead of leaving the user on the slow searchfs() path (a full
	// live scan is ~86s per query). The build itself avoids network volumes.
	if !st.eng.HasIndex() {
		st.buildIndexAsync()
	}

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
		st.status.SetText(fmt.Sprintf("就绪 / Ready · index loaded (%d entries) · fuzzy search",
			st.eng.IndexCount()))
	} else {
		st.status.SetText("就绪 / Ready · no index · using live searchfs() fallback (building one…)")
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
// background goroutine, then reloads it into the engine. It is safe to call from
// either the toolbar button (first launch also triggers it automatically) and
// is a no-op if a build is already in flight, so the auto-build and a manual tap
// can't collide. The scan skips all network / FUSE volumes (see index.Build).
func (st *guiState) buildIndexAsync() {
	st.mu.Lock()
	if st.building {
		st.mu.Unlock()
		return
	}
	st.building = true
	st.mu.Unlock()

	// status.SetText is a UI-thread operation; wrap it in fyne.Do since this
	// method is also invoked from the pre-ShowAndRun auto-build path.
	fyne.Do(func() {
		st.status.SetText("正在建立索引… / Building index (skipping network volumes)…")
	})
	go func() {
		roots := index.DefaultRoots()
		out := index.DefaultPath()
		start := time.Now()
		n, err := index.Build(roots, out)
		reloadErr := error(nil)
		if err == nil {
			reloadErr = st.eng.Reload(out)
		}

		st.mu.Lock()
		st.building = false
		st.mu.Unlock()

		fyne.Do(func() {
			switch {
			case err != nil:
				st.status.SetText("索引建立失败 / Index build failed: " + err.Error())
			case reloadErr != nil:
				st.status.SetText("已建立索引但重载失败 / Built index but reload failed: " + reloadErr.Error())
			default:
				st.status.SetText(fmt.Sprintf("索引就绪 / Index ready · %d entries · %s",
					n, time.Since(start).Round(time.Millisecond)))
			}
		})
	}()
}
