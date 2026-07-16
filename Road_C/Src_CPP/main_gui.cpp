// main_gui.cpp — Qt6 desktop GUI for Road_C (C++): hybrid index + searchfs.
//
// Shares the "search box + results list" shell of the 18-implementation matrix.
// Route-specific pieces: the backend is the HybridEngine (index primary,
// searchfs() fallback), the status bar shows which backend served each query,
// there's a "Rebuild Index" action, and the results list has a right-click
// "Reveal in Finder" context menu.
//
// Search and index builds run on a QThread worker so the UI stays responsive.
#include "HybridEngine.h"

#include <QApplication>
#include <QClipboard>
#include <QCheckBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMainWindow>
#include <QMenu>
#include <QMessageBox>
#include <QProcess>
#include <QPushButton>
#include <QSpinBox>
#include <QStatusBar>
#include <QThread>
#include <QTimer>
#include <QVBoxLayout>
#include <QWidget>

#include <atomic>
#include <memory>

using namespace macfind;

// One shared HybridEngine for the process. The index lives in memory once built
// or loaded; both the GUI thread (reads: indexAvailable/count) and the worker
// thread (search / build) touch it, but never concurrently — the GUI serialises
// operations (a build or a search, never both, and one search at a time).
static HybridEngine& engine() {
    static HybridEngine e;
    return e;
}

// -------- Worker: runs one query off the GUI thread and streams matches. -------
class SearchWorker : public QObject {
    Q_OBJECT
public:
    SearchWorker(std::string term, SearchOptions opts, QObject* parent = nullptr)
        : QObject(parent), term_(std::move(term)), opts_(opts) {}

    void cancel() { cancelled_.store(true); }

public slots:
    void run() {
        SearchOutcome outcome = engine().search(term_, opts_, /*allowFallback=*/true);
        if (cancelled_.load()) {
            emit finished(true, QString(), -1, QString());
            return;
        }
        // Push results to the UI in one batch (already ranked for the index path).
        for (const auto& r : outcome.results) {
            if (cancelled_.load()) break;
            emit matchFound(QString::fromStdString(r.path));
        }
        emit finished(outcome.ok, QString::fromStdString(outcome.error),
                      static_cast<int>(outcome.results.size()),
                      QString::fromLatin1(backendName(outcome.backend)));
    }

signals:
    void matchFound(const QString& path);
    void finished(bool ok, const QString& error, int count, const QString& backend);

private:
    std::string       term_;
    SearchOptions     opts_;
    std::atomic<bool> cancelled_{false};
};

// -------- Worker: builds/persists the index off the GUI thread. ----------------
class IndexWorker : public QObject {
    Q_OBJECT
public slots:
    void run() {
        bool ok = engine().buildIndex();  // default roots ($HOME), default path
        emit finished(ok, static_cast<qulonglong>(engine().indexEntryCount()));
    }
signals:
    void finished(bool ok, qulonglong entries);
};

// ------------------------------- Main window -----------------------------------
class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    MainWindow() {
        auto* central = new QWidget(this);
        auto* root = new QVBoxLayout(central);

        // --- Search box + rebuild button ---
        auto* topRow = new QHBoxLayout();
        searchBox_ = new QLineEdit(central);
        searchBox_->setPlaceholderText("Search filenames (index-first, searchfs fallback)…");
        searchBox_->setClearButtonEnabled(true);
        topRow->addWidget(searchBox_, 1);
        rebuildBtn_ = new QPushButton("Rebuild Index", central);
        topRow->addWidget(rebuildBtn_);
        root->addLayout(topRow);

        // --- Option row ---
        auto* optRow = new QHBoxLayout();
        filesOnly_     = new QCheckBox("Files only", central);
        dirsOnly_      = new QCheckBox("Dirs only", central);
        caseSensitive_ = new QCheckBox("Case", central);
        optRow->addWidget(filesOnly_);
        optRow->addWidget(dirsOnly_);
        optRow->addWidget(caseSensitive_);
        optRow->addWidget(new QLabel("Limit:", central));
        limit_ = new QSpinBox(central);
        limit_->setRange(0, 1000000);
        limit_->setValue(1000);
        limit_->setSpecialValueText("∞");
        optRow->addWidget(limit_);
        optRow->addStretch();
        root->addLayout(optRow);

        // Files-only and dirs-only are mutually exclusive.
        connect(filesOnly_, &QCheckBox::toggled, this, [this](bool on) {
            if (on) dirsOnly_->setChecked(false);
        });
        connect(dirsOnly_, &QCheckBox::toggled, this, [this](bool on) {
            if (on) filesOnly_->setChecked(false);
        });

        // --- Results list with right-click "Reveal in Finder" ---
        results_ = new QListWidget(central);
        results_->setUniformItemSizes(true);
        results_->setContextMenuPolicy(Qt::CustomContextMenu);
        connect(results_, &QListWidget::customContextMenuRequested,
                this, &MainWindow::showContextMenu);
        connect(results_, &QListWidget::itemActivated, this, [this](QListWidgetItem* it) {
            if (it) revealInFinder(it->text());
        });
        root->addWidget(results_, 1);

        setCentralWidget(central);
        setWindowTitle("MacFind · Qt/C++ · Road_C");
        resize(760, 560);
        updateStatus();

        // Debounce: search 250ms after the user stops typing.
        debounce_ = new QTimer(this);
        debounce_->setSingleShot(true);
        debounce_->setInterval(250);
        connect(debounce_, &QTimer::timeout, this, &MainWindow::startSearch);
        connect(searchBox_, &QLineEdit::textChanged, this, [this] { debounce_->start(); });
        connect(searchBox_, &QLineEdit::returnPressed, this, &MainWindow::startSearch);
        for (auto* cb : {filesOnly_, dirsOnly_, caseSensitive_}) {
            connect(cb, &QCheckBox::toggled, this, [this] { debounce_->start(); });
        }
        connect(limit_, QOverload<int>::of(&QSpinBox::valueChanged),
                this, [this] { debounce_->start(); });

        connect(rebuildBtn_, &QPushButton::clicked, this, &MainWindow::startRebuild);

        // First-launch auto-index: if no index exists yet, build one right away
        // instead of silently dropping every query onto the slow searchfs()
        // path (~86s full-disk). Deferred so the window paints first. If an
        // index is already loaded, we stay quiet and ready.
        if (!engine().indexAvailable()) {
            QTimer::singleShot(0, this, [this] {
                statusBar()->showMessage(
                    "No index yet — building one now for fast fuzzy search…");
                startRebuild();
            });
        }
    }

    ~MainWindow() override { stopSearchWorker(); }

private slots:
    void startSearch() {
        if (indexing_) return;  // don't search while the index is rebuilding
        const QString term = searchBox_->text().trimmed();
        stopSearchWorker();
        results_->clear();
        if (term.isEmpty()) { updateStatus(); return; }

        SearchOptions opts;
        opts.filesOnly     = filesOnly_->isChecked();
        opts.dirsOnly      = dirsOnly_->isChecked();
        opts.caseSensitive = caseSensitive_->isChecked();
        opts.limit         = static_cast<std::size_t>(limit_->value());

        statusBar()->showMessage("Searching…");

        searchThread_ = new QThread(this);
        searchWorker_ = new SearchWorker(term.toStdString(), opts);
        searchWorker_->moveToThread(searchThread_);
        connect(searchThread_, &QThread::started, searchWorker_, &SearchWorker::run);
        connect(searchWorker_, &SearchWorker::matchFound, this, &MainWindow::onMatch,
                Qt::QueuedConnection);
        connect(searchWorker_, &SearchWorker::finished, this, &MainWindow::onSearchFinished,
                Qt::QueuedConnection);
        searchThread_->start();
    }

    void onMatch(const QString& path) { results_->addItem(path); }

    void onSearchFinished(bool ok, const QString& error, int count, const QString& backend) {
        if (count < 0) {
            statusBar()->showMessage("Cancelled.");
        } else if (!ok) {
            statusBar()->showMessage("Error: " + error);
        } else {
            const QString via = backend == "index"
                ? "index (fuzzy, ranked)"
                : backend == "searchfs" ? "searchfs (fallback)" : backend;
            statusBar()->showMessage(QString("%1 match(es) via %2.").arg(count).arg(via));
        }
        stopSearchWorker();
    }

    void startRebuild() {
        if (indexing_) return;
        stopSearchWorker();
        indexing_ = true;
        rebuildBtn_->setEnabled(false);
        searchBox_->setEnabled(false);
        statusBar()->showMessage("Building index over your home folder… (this can take a while)");

        indexThread_ = new QThread(this);
        auto* worker = new IndexWorker();
        worker->moveToThread(indexThread_);
        connect(indexThread_, &QThread::started, worker, &IndexWorker::run);
        connect(worker, &IndexWorker::finished, this,
                [this, worker](bool ok, qulonglong entries) {
                    indexing_ = false;
                    rebuildBtn_->setEnabled(true);
                    searchBox_->setEnabled(true);
                    if (ok) {
                        statusBar()->showMessage(
                            QString("Index built: %1 entries. Searches now use the index.")
                                .arg(entries));
                    } else {
                        statusBar()->showMessage("Index build failed; searches use searchfs().");
                    }
                    worker->deleteLater();
                    indexThread_->quit();
                    indexThread_->wait();
                    indexThread_->deleteLater();
                    indexThread_ = nullptr;
                    // Re-run the current query against the new index.
                    if (!searchBox_->text().trimmed().isEmpty()) startSearch();
                },
                Qt::QueuedConnection);
        indexThread_->start();
    }

    void showContextMenu(const QPoint& pos) {
        QListWidgetItem* item = results_->itemAt(pos);
        if (!item) return;
        QMenu menu(this);
        QAction* reveal = menu.addAction("Reveal in Finder");
        QAction* copy   = menu.addAction("Copy Path");
        QAction* chosen = menu.exec(results_->viewport()->mapToGlobal(pos));
        if (chosen == reveal) {
            revealInFinder(item->text());
        } else if (chosen == copy) {
            QApplication::clipboard()->setText(item->text());
        }
    }

private:
    // Select the file in Finder via `open -R` (works even for non-existent-after
    // -index entries: Finder simply reports it can't be found).
    void revealInFinder(const QString& path) {
        QProcess::startDetached("/usr/bin/open", {"-R", path});
    }

    void updateStatus() {
        if (engine().indexAvailable()) {
            statusBar()->showMessage(
                QString("Ready. Index loaded (%1 entries). Type to search.")
                    .arg(engine().indexEntryCount()));
        } else {
            statusBar()->showMessage(
                "Ready. No index yet — building automatically; searchfs() serves searches meanwhile.");
        }
    }

    void stopSearchWorker() {
        if (searchWorker_) searchWorker_->cancel();
        if (searchThread_) {
            searchThread_->quit();
            searchThread_->wait();
            searchThread_->deleteLater();
            searchThread_ = nullptr;
        }
        if (searchWorker_) {
            searchWorker_->deleteLater();
            searchWorker_ = nullptr;
        }
    }

    QLineEdit*   searchBox_ = nullptr;
    QPushButton* rebuildBtn_ = nullptr;
    QListWidget* results_ = nullptr;
    QCheckBox*   filesOnly_ = nullptr;
    QCheckBox*   dirsOnly_ = nullptr;
    QCheckBox*   caseSensitive_ = nullptr;
    QSpinBox*    limit_ = nullptr;
    QTimer*      debounce_ = nullptr;

    QThread*      searchThread_ = nullptr;
    SearchWorker* searchWorker_ = nullptr;
    QThread*      indexThread_  = nullptr;
    bool          indexing_ = false;
};

#include "main_gui.moc"

int main(int argc, char* argv[]) {
    QApplication app(argc, argv);
    app.setApplicationName("MacFindRoadCCpp");
    MainWindow win;
    win.show();
    return app.exec();
}
