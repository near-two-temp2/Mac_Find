"""PyQt6 GUI for mac_find_a (Road_A: searchfs, no index).

Layout: a search box on top, an option bar (files/dirs, case, exact, limit),
and a streaming results list.  Each keystroke (debounced) launches a background
:class:`SearchWorker` thread that iterates the ``searchfs()`` engine and pushes
paths back to the UI thread via Qt signals.  A running search is cancelled when
a new query starts, so typing stays responsive.
"""

from __future__ import annotations

import sys

from PyQt6.QtCore import Qt, QThread, QTimer, pyqtSignal
from PyQt6.QtGui import QGuiApplication
from PyQt6.QtWidgets import (
    QApplication,
    QCheckBox,
    QComboBox,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QMainWindow,
    QVBoxLayout,
    QWidget,
)

from . import searchfs_engine as engine


class SearchWorker(QThread):
    """Runs a single searchfs query off the UI thread and streams results."""

    result = pyqtSignal(str)
    finished_count = pyqtSignal(int)
    error = pyqtSignal(str)

    def __init__(self, term: str, opts: engine.SearchOptions):
        super().__init__()
        self._term = term
        self._opts = opts
        self._cancelled = False

    def cancel(self) -> None:
        self._cancelled = True

    def _keep_going(self) -> bool:
        return not self._cancelled

    def run(self) -> None:  # noqa: D401 - QThread entry point
        count = 0
        try:
            for path in engine.search(self._term, self._opts,
                                      should_continue=self._keep_going):
                if self._cancelled:
                    break
                self.result.emit(path)
                count += 1
        except Exception as exc:  # pragma: no cover - defensive
            self.error.emit(str(exc))
            return
        if not self._cancelled:
            self.finished_count.emit(count)


class MainWindow(QMainWindow):
    DEBOUNCE_MS = 250

    def __init__(self):
        super().__init__()
        self.setWindowTitle("MacFind · Python/PyQt6 · Road_A")
        self.resize(820, 560)

        self._worker: SearchWorker | None = None

        central = QWidget()
        root = QVBoxLayout(central)

        # --- search box ---
        self.search_box = QLineEdit()
        self.search_box.setPlaceholderText(
            "Type a filename substring…  (live searchfs, no index)")
        self.search_box.setClearButtonEnabled(True)
        self.search_box.textChanged.connect(self._on_text_changed)
        root.addWidget(self.search_box)

        # --- option bar ---
        opts_bar = QHBoxLayout()
        self.type_combo = QComboBox()
        self.type_combo.addItems(["Files + Dirs", "Files only", "Dirs only"])
        self.type_combo.currentIndexChanged.connect(lambda _: self._schedule())
        opts_bar.addWidget(self.type_combo)

        self.case_cb = QCheckBox("Case sensitive")
        self.case_cb.stateChanged.connect(lambda _: self._schedule())
        opts_bar.addWidget(self.case_cb)

        self.exact_cb = QCheckBox("Exact match")
        self.exact_cb.stateChanged.connect(lambda _: self._schedule())
        opts_bar.addWidget(self.exact_cb)

        self.skip_pkg_cb = QCheckBox("Skip packages")
        self.skip_pkg_cb.stateChanged.connect(lambda _: self._schedule())
        opts_bar.addWidget(self.skip_pkg_cb)

        opts_bar.addWidget(QLabel("Limit:"))
        self.limit_combo = QComboBox()
        self.limit_combo.addItems(["500", "1000", "5000", "Unlimited"])
        self.limit_combo.setCurrentText("1000")
        self.limit_combo.currentIndexChanged.connect(lambda _: self._schedule())
        opts_bar.addWidget(self.limit_combo)

        opts_bar.addStretch(1)
        root.addLayout(opts_bar)

        # --- results ---
        self.results = QListWidget()
        self.results.setUniformItemSizes(True)
        root.addWidget(self.results, stretch=1)

        # --- status ---
        self.status = QLabel()
        root.addWidget(self.status)

        self.setCentralWidget(central)

        # debounce timer
        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.timeout.connect(self._start_search)

        if not engine.searchfs_available():
            self.search_box.setEnabled(False)
            self.status.setText(
                "searchfs() unavailable — this build only runs on macOS "
                f"({engine.load_error()})")
        else:
            self.status.setText("Ready. Full Disk Access may be needed for "
                                "complete results.")

    # ------------------------------------------------------------------
    # option plumbing
    # ------------------------------------------------------------------
    def _current_opts(self) -> engine.SearchOptions:
        idx = self.type_combo.currentIndex()
        limit_text = self.limit_combo.currentText()
        limit = 0 if limit_text == "Unlimited" else int(limit_text)
        return engine.SearchOptions(
            dirs_only=(idx == 2),
            files_only=(idx == 1),
            case_sensitive=self.case_cb.isChecked(),
            exact_match=self.exact_cb.isChecked(),
            skip_packages=self.skip_pkg_cb.isChecked(),
            limit=limit,
        )

    # ------------------------------------------------------------------
    # search lifecycle
    # ------------------------------------------------------------------
    def _on_text_changed(self, _text: str) -> None:
        self._schedule()

    def _schedule(self) -> None:
        self._debounce.start(self.DEBOUNCE_MS)

    def _cancel_worker(self) -> None:
        if self._worker is not None:
            self._worker.cancel()
            self._worker.wait(2000)
            self._worker = None

    def _start_search(self) -> None:
        if not engine.searchfs_available():
            return
        self._cancel_worker()
        self.results.clear()

        term = self.search_box.text().strip()
        if not term:
            self.status.setText("Type to search.")
            return

        self.status.setText(f"Searching for “{term}” …")
        worker = SearchWorker(term, self._current_opts())
        worker.result.connect(self._on_result)
        worker.finished_count.connect(self._on_done)
        worker.error.connect(self._on_error)
        self._worker = worker
        worker.start()

    def _on_result(self, path: str) -> None:
        self.results.addItem(path)
        # Cheap live counter without walking the whole list each time.
        if self.results.count() % 50 == 0:
            self.status.setText(f"{self.results.count()} matches so far…")

    def _on_done(self, count: int) -> None:
        self.status.setText(f"Done — {count} match(es).")

    def _on_error(self, msg: str) -> None:
        self.status.setText(f"Error: {msg}")

    def closeEvent(self, event):  # noqa: N802 - Qt override
        self._cancel_worker()
        super().closeEvent(event)


def main() -> int:
    app = QApplication(sys.argv)
    app.setApplicationName("Mac Find Road A")
    QGuiApplication.setApplicationDisplayName("Mac Find — Road A")
    win = MainWindow()
    win.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
