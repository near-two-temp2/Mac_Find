"""PyQt6 desktop GUI for MacFind Road_C.

The window is intentionally the shared "search box + result list" shell used by
every implementation in this repo; only the backend differs. Here it is the
:class:`~macfind_c.engine.HybridEngine` (index primary, searchfs fallback).

Layout::

    ┌──────────────────────────────────────────────┐
    │  [ search field.......................... ]   │
    │  [ Files ] [ Dirs ] [ Build Index ]           │
    ├──────────────────────────────────────────────┤
    │  results (double-click = reveal in Finder)    │
    │  ...                                           │
    ├──────────────────────────────────────────────┤
    │  status: Index: N entries · 12 hits · 4.1 ms  │
    └──────────────────────────────────────────────┘

Search runs on a background ``QThread`` so keystrokes never block the UI. A
short debounce coalesces rapid typing.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import Optional

from PyQt6.QtCore import (
    Qt,
    QThread,
    QTimer,
    pyqtSignal,
)
from PyQt6.QtGui import QAction
from PyQt6.QtWidgets import (
    QApplication,
    QCheckBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMenu,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from .engine import HybridEngine, Result, SearchOutcome, Source

_DEBOUNCE_MS = 120
_RESULT_LIMIT = 500


class SearchWorker(QThread):
    """Runs one search on a background thread and emits the outcome."""

    finished_search = pyqtSignal(object)  # SearchOutcome

    def __init__(
        self,
        engine: HybridEngine,
        query: str,
        dirs_only: bool,
        files_only: bool,
    ):
        super().__init__()
        self._engine = engine
        self._query = query
        self._dirs_only = dirs_only
        self._files_only = files_only

    def run(self) -> None:
        try:
            outcome = self._engine.search(
                self._query,
                limit=_RESULT_LIMIT,
                dirs_only=self._dirs_only,
                files_only=self._files_only,
            )
        except Exception:  # never let a worker crash the app
            outcome = SearchOutcome([], Source.NONE, 0.0, 0)
        self.finished_search.emit(outcome)


class IndexWorker(QThread):
    """Builds the binary index in the background over $HOME."""

    finished_index = pyqtSignal(str)  # message

    def __init__(self, index_path: Path):
        super().__init__()
        self._index_path = index_path

    def run(self) -> None:
        from . import index as index_mod

        try:
            written = index_mod.build(
                [str(Path.home())],
                out_path=self._index_path,
                max_entries=300_000,
            )
            view = index_mod.load(written)
            self.finished_index.emit(
                f"Indexed {view.entry_count:,} entries → {written}"
            )
        except Exception as exc:
            self.finished_index.emit(f"Index build failed: {exc}")


class MainWindow(QMainWindow):
    def __init__(self, engine: Optional[HybridEngine] = None):
        super().__init__()
        self.engine = engine or HybridEngine()
        self._worker: Optional[SearchWorker] = None
        self._index_worker: Optional[IndexWorker] = None
        # Retired-but-still-running search workers, kept alive until they finish.
        self._retired: list[SearchWorker] = []

        self.setWindowTitle("MacFind · Python/PyQt6 · Road_C")
        self.resize(820, 560)

        central = QWidget()
        self.setCentralWidget(central)
        root = QVBoxLayout(central)
        root.setContentsMargins(12, 12, 12, 12)
        root.setSpacing(8)

        # Search field.
        self.search_field = QLineEdit()
        self.search_field.setPlaceholderText("Search files and folders…")
        self.search_field.setClearButtonEnabled(True)
        self.search_field.textChanged.connect(self._on_text_changed)
        self.search_field.returnPressed.connect(self._run_search_now)
        root.addWidget(self.search_field)

        # Options row.
        opts = QHBoxLayout()
        self.chk_files = QCheckBox("Files only")
        self.chk_dirs = QCheckBox("Folders only")
        self.chk_files.stateChanged.connect(self._on_option_changed)
        self.chk_dirs.stateChanged.connect(self._on_option_changed)
        self.btn_index = QPushButton("Build / Rebuild Index")
        self.btn_index.clicked.connect(self._build_index)
        opts.addWidget(self.chk_files)
        opts.addWidget(self.chk_dirs)
        opts.addStretch(1)
        opts.addWidget(self.btn_index)
        root.addLayout(opts)

        # Result list.
        self.results = QListWidget()
        self.results.setAlternatingRowColors(True)
        self.results.itemActivated.connect(self._reveal_item)
        self.results.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self.results.customContextMenuRequested.connect(self._show_context_menu)
        root.addWidget(self.results, 1)

        # Status bar.
        self.status = QLabel()
        self.status.setTextInteractionFlags(
            Qt.TextInteractionFlag.TextSelectableByMouse
        )
        root.addWidget(self.status)
        self._set_status(self.engine.status_line())

        # Debounce timer.
        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.setInterval(_DEBOUNCE_MS)
        self._debounce.timeout.connect(self._run_search_now)

        self.search_field.setFocus()
        # Populate with an initial (empty-query) listing when an index exists.
        if self.engine.has_index:
            self._run_search_now()

    # -- events ------------------------------------------------------------- #
    def _on_text_changed(self, _text: str) -> None:
        self._debounce.start()

    def _on_option_changed(self, _state: int) -> None:
        # Keep "files only" and "folders only" mutually exclusive.
        sender = self.sender()
        if sender is self.chk_files and self.chk_files.isChecked():
            self.chk_dirs.setChecked(False)
        elif sender is self.chk_dirs and self.chk_dirs.isChecked():
            self.chk_files.setChecked(False)
        self._run_search_now()

    def _run_search_now(self) -> None:
        self._debounce.stop()
        query = self.search_field.text()

        # Detach any in-flight search so its (now stale) results are ignored.
        # We keep the old worker referenced in _retired until it finishes so Qt
        # never garbage-collects a running QThread out from under itself.
        if self._worker is not None and self._worker.isRunning():
            try:
                self._worker.finished_search.disconnect(self._on_results)
            except TypeError:
                pass
            retired = self._worker
            self._retired.append(retired)
            retired.finished.connect(lambda w=retired: self._reap(w))

        self._worker = SearchWorker(
            self.engine,
            query,
            dirs_only=self.chk_dirs.isChecked(),
            files_only=self.chk_files.isChecked(),
        )
        self._worker.finished_search.connect(self._on_results)
        self._worker.start()

    def _reap(self, worker: "SearchWorker") -> None:
        """Drop a finished retired worker so it can be freed."""
        try:
            self._retired.remove(worker)
        except ValueError:
            pass

    def _on_results(self, outcome: SearchOutcome) -> None:
        self.results.clear()
        for r in outcome.results:
            item = QListWidgetItem(self._format_result(r))
            item.setData(Qt.ItemDataRole.UserRole, r.path)
            item.setToolTip(r.path)
            self.results.addItem(item)

        src = {
            Source.INDEX: "index",
            Source.SEARCHFS: "searchfs() fallback",
            Source.NONE: "no backend",
        }[outcome.source]
        self._set_status(
            f"{self.engine.status_line()}  ·  "
            f"{len(outcome.results)} hits via {src}  ·  "
            f"{outcome.elapsed_ms:.1f} ms"
        )

    @staticmethod
    def _format_result(r: Result) -> str:
        icon = "📁" if r.is_dir else "📄"
        return f"{icon}  {r.name}    —    {r.path}"

    # -- actions ------------------------------------------------------------ #
    def _selected_path(self) -> Optional[str]:
        item = self.results.currentItem()
        if item is None:
            return None
        return item.data(Qt.ItemDataRole.UserRole)

    def _reveal_item(self, item: QListWidgetItem) -> None:
        path = item.data(Qt.ItemDataRole.UserRole)
        if path:
            self._reveal_in_finder(path)

    def _reveal_in_finder(self, path: str) -> None:
        if sys.platform == "darwin" and Path(path).exists():
            subprocess.run(["open", "-R", path], check=False)

    def _open_path(self, path: str) -> None:
        if sys.platform == "darwin" and Path(path).exists():
            subprocess.run(["open", path], check=False)

    def _show_context_menu(self, pos) -> None:
        path = self._selected_path()
        if not path:
            return
        menu = QMenu(self)
        act_reveal = QAction("Reveal in Finder", self)
        act_reveal.triggered.connect(lambda: self._reveal_in_finder(path))
        act_open = QAction("Open", self)
        act_open.triggered.connect(lambda: self._open_path(path))
        act_copy = QAction("Copy Path", self)
        act_copy.triggered.connect(
            lambda: QApplication.clipboard().setText(path)
        )
        menu.addAction(act_reveal)
        menu.addAction(act_open)
        menu.addAction(act_copy)
        menu.exec(self.results.mapToGlobal(pos))

    def _build_index(self) -> None:
        if self._index_worker is not None and self._index_worker.isRunning():
            return
        self.btn_index.setEnabled(False)
        self._set_status("Building index over $HOME… (this may take a minute)")
        self._index_worker = IndexWorker(self.engine.index_path)
        self._index_worker.finished_index.connect(self._on_index_done)
        self._index_worker.start()

    def _on_index_done(self, message: str) -> None:
        self.btn_index.setEnabled(True)
        self.engine.load_index()
        self._set_status(message + "  ·  " + self.engine.status_line())
        self._run_search_now()

    def _set_status(self, text: str) -> None:
        self.status.setText(text)


def main(argv: Optional[list[str]] = None) -> int:
    app = QApplication(argv if argv is not None else sys.argv)
    app.setApplicationName("MacFind Road C")
    window = MainWindow()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
