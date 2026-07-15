"""PyQt6 desktop GUI — search box + result list.

Backing engine is the self-built binary index (:mod:`mac_find_b.engine`).
The window offers:
  * a "Build index" button that walks the filesystem on a worker thread,
  * a search box that runs the two-phase index search as you type (debounced),
  * a result list showing path + score, double-click reveals in Finder.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import List, Optional

from PyQt6.QtCore import Qt, QThread, QTimer, pyqtSignal
from PyQt6.QtGui import QFont
from PyQt6.QtWidgets import (
    QApplication,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from . import engine


class IndexWorker(QThread):
    """Builds the index on a background thread so the UI stays responsive."""

    progress = pyqtSignal(int)
    finished_index = pyqtSignal(object)  # engine.Index
    failed = pyqtSignal(str)

    def __init__(self, roots: List[str], index_dir: Path, max_entries: Optional[int]):
        super().__init__()
        self._roots = roots
        self._index_dir = index_dir
        self._max = max_entries

    def run(self) -> None:  # noqa: D401 - Qt override
        try:
            idx = engine.build_index(
                self._roots,
                max_entries=self._max,
                progress=lambda n: self.progress.emit(n),
            )
            engine.save_index(idx, self._index_dir)
            self.finished_index.emit(idx)
        except Exception as exc:  # pragma: no cover - surfaced in the UI
            self.failed.emit(str(exc))


class MainWindow(QMainWindow):
    def __init__(self) -> None:
        super().__init__()
        self.setWindowTitle("MacFind · Python/PyQt6 · Road_B")
        self.resize(820, 560)

        self._index: Optional[engine.Index] = None
        self._index_dir = engine.default_index_dir()
        self._worker: Optional[IndexWorker] = None

        # Debounce timer so typing does not fire a search per keystroke.
        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.setInterval(120)
        self._debounce.timeout.connect(self._run_search)

        self._build_ui()
        self._try_load_existing()

    # ---- UI ----------------------------------------------------------------
    def _build_ui(self) -> None:
        central = QWidget()
        self.setCentralWidget(central)
        root = QVBoxLayout(central)

        # Top bar: search box + build button
        top = QHBoxLayout()
        self.search_box = QLineEdit()
        self.search_box.setPlaceholderText("Type to search the index…")
        self.search_box.setClearButtonEnabled(True)
        self.search_box.textChanged.connect(lambda _: self._debounce.start())
        f = QFont()
        f.setPointSize(15)
        self.search_box.setFont(f)
        top.addWidget(self.search_box, 1)

        self.build_btn = QPushButton("Build index")
        self.build_btn.clicked.connect(self._on_build_clicked)
        top.addWidget(self.build_btn)
        root.addLayout(top)

        # Result list
        self.results = QListWidget()
        self.results.itemActivated.connect(self._reveal_item)
        self.results.itemDoubleClicked.connect(self._reveal_item)
        root.addWidget(self.results, 1)

        # Status line
        self.status = QLabel("No index. Click “Build index” to scan your files.")
        self.status.setStyleSheet("color: #888;")
        root.addWidget(self.status)

    # ---- Index lifecycle ---------------------------------------------------
    def _try_load_existing(self) -> None:
        if engine.index_exists(self._index_dir):
            try:
                self._index = engine.load_index(self._index_dir)
                self.status.setText(
                    f"Loaded index: {self._index.count} entries from {self._index_dir}"
                )
            except Exception as exc:  # pragma: no cover
                self.status.setText(f"Failed to load index: {exc}")

    def _on_build_clicked(self) -> None:
        if self._worker is not None and self._worker.isRunning():
            return
        self.build_btn.setEnabled(False)
        self.status.setText("Building index (scanning $HOME)…")

        roots = [str(Path.home())]
        # First build is capped so the demo/CI finishes quickly; remove the cap
        # for a full-disk index.
        self._worker = IndexWorker(roots, self._index_dir, max_entries=400_000)
        self._worker.progress.connect(self._on_progress)
        self._worker.finished_index.connect(self._on_index_ready)
        self._worker.failed.connect(self._on_index_failed)
        self._worker.start()

    def _on_progress(self, n: int) -> None:
        self.status.setText(f"Building index… scanned {n:,} entries")

    def _on_index_ready(self, idx: engine.Index) -> None:
        self._index = idx
        self.build_btn.setEnabled(True)
        self.status.setText(f"Index ready: {idx.count:,} entries")
        self._run_search()

    def _on_index_failed(self, msg: str) -> None:
        self.build_btn.setEnabled(True)
        self.status.setText(f"Index build failed: {msg}")

    # ---- Search ------------------------------------------------------------
    def _run_search(self) -> None:
        self.results.clear()
        if self._index is None:
            return
        query = self.search_box.text()
        if not query.strip():
            self.status.setText(f"Index: {self._index.count:,} entries")
            return

        hits = engine.search(self._index, query, limit=500)
        for h in hits:
            prefix = "📁 " if h.is_dir else "📄 "
            item = QListWidgetItem(prefix + h.path)
            item.setData(Qt.ItemDataRole.UserRole, h.path)
            self.results.addItem(item)
        self.status.setText(f"{len(hits)} result(s) for “{query}”")

    def _reveal_item(self, item: QListWidgetItem) -> None:
        path = item.data(Qt.ItemDataRole.UserRole)
        if not path:
            return
        try:
            subprocess.run(["open", "-R", path], check=False)
        except Exception:  # pragma: no cover
            pass


def main() -> int:
    app = QApplication(sys.argv)
    win = MainWindow()
    win.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
