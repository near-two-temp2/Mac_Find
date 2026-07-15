// main_gui.cpp — Qt6 desktop GUI for Road_A (C++): searchfs() real-time search.
//
// UI shell shared by the "18 implementations" matrix: a search box on top and a
// results list below. The only route-specific piece is the backend engine, here
// SearchEngine (searchfs, no index).
//
// The actual searchfs() scan runs on a QThread worker so the UI stays responsive.
#include "SearchEngine.h"

#include <QApplication>
#include <QCheckBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMainWindow>
#include <QSpinBox>
#include <QThread>
#include <QTimer>
#include <QVBoxLayout>
#include <QWidget>
#include <QMetaObject>
#include <QStatusBar>

#include <atomic>
#include <memory>

using namespace macfind;

// Worker that runs one searchfs() query off the GUI thread. It streams matches
// back to the main thread via queued signals and can be cancelled mid-flight.
class SearchWorker : public QObject {
    Q_OBJECT
public:
    SearchWorker(std::string term, SearchOptions opts, QObject* parent = nullptr)
        : QObject(parent), term_(std::move(term)), opts_(opts) {}

    void cancel() { cancelled_.store(true); }

public slots:
    void run() {
        SearchEngine engine;
        auto outcome = engine.search(term_, opts_, std::string(),
            [this](const SearchResult& r) {
                if (cancelled_.load()) return false;
                emit matchFound(QString::fromStdString(r.path));
                return true;
            });
        if (!cancelled_.load()) {
            emit finished(outcome.ok, QString::fromStdString(outcome.error),
                          static_cast<int>(outcome.results.size()));
        } else {
            emit finished(true, QString(), -1);
        }
    }

signals:
    void matchFound(const QString& path);
    void finished(bool ok, const QString& error, int count);

private:
    std::string        term_;
    SearchOptions      opts_;
    std::atomic<bool>  cancelled_{false};
};

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    MainWindow() {
        auto* central = new QWidget(this);
        auto* root = new QVBoxLayout(central);

        // --- Search box ---
        searchBox_ = new QLineEdit(central);
        searchBox_->setPlaceholderText("Search filenames via searchfs()…");
        searchBox_->setClearButtonEnabled(true);
        root->addWidget(searchBox_);

        // --- Option row ---
        auto* optRow = new QHBoxLayout();
        filesOnly_     = new QCheckBox("Files only", central);
        dirsOnly_      = new QCheckBox("Dirs only", central);
        exactMatch_    = new QCheckBox("Exact", central);
        caseSensitive_ = new QCheckBox("Case", central);
        optRow->addWidget(filesOnly_);
        optRow->addWidget(dirsOnly_);
        optRow->addWidget(exactMatch_);
        optRow->addWidget(caseSensitive_);
        optRow->addWidget(new QLabel("Limit:", central));
        limit_ = new QSpinBox(central);
        limit_->setRange(0, 1000000);
        limit_->setValue(1000);
        limit_->setSpecialValueText("∞");  // 0 shows as infinity
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

        // --- Results list ---
        results_ = new QListWidget(central);
        results_->setUniformItemSizes(true);
        root->addWidget(results_, 1);

        setCentralWidget(central);
        setWindowTitle("MacFind · Qt/C++ · Road_A");
        resize(720, 520);
        statusBar()->showMessage("Type to search. Powered by searchfs().");

        // Debounce: search 250ms after the user stops typing.
        debounce_ = new QTimer(this);
        debounce_->setSingleShot(true);
        debounce_->setInterval(250);
        connect(debounce_, &QTimer::timeout, this, &MainWindow::startSearch);
        connect(searchBox_, &QLineEdit::textChanged, this, [this] { debounce_->start(); });
        connect(searchBox_, &QLineEdit::returnPressed, this, &MainWindow::startSearch);

        // Re-run when any option changes.
        for (auto* cb : {filesOnly_, dirsOnly_, exactMatch_, caseSensitive_}) {
            connect(cb, &QCheckBox::toggled, this, [this] { debounce_->start(); });
        }
        connect(limit_, &QSpinBox::valueChanged,
                this, [this] { debounce_->start(); });
    }

    ~MainWindow() override { stopWorker(); }

private slots:
    void startSearch() {
        const QString term = searchBox_->text().trimmed();
        stopWorker();
        results_->clear();
        if (term.isEmpty()) {
            statusBar()->showMessage("Type to search.");
            return;
        }

        SearchOptions opts;
        opts.filesOnly     = filesOnly_->isChecked();
        opts.dirsOnly      = dirsOnly_->isChecked();
        opts.exactMatch    = exactMatch_->isChecked();
        opts.caseSensitive = caseSensitive_->isChecked();
        opts.limit         = static_cast<std::size_t>(limit_->value());

        statusBar()->showMessage("Searching…");

        thread_ = new QThread(this);
        worker_ = new SearchWorker(term.toStdString(), opts);
        worker_->moveToThread(thread_);

        connect(thread_, &QThread::started, worker_, &SearchWorker::run);
        connect(worker_, &SearchWorker::matchFound, this, &MainWindow::onMatch,
                Qt::QueuedConnection);
        connect(worker_, &SearchWorker::finished, this, &MainWindow::onFinished,
                Qt::QueuedConnection);
        thread_->start();
    }

    void onMatch(const QString& path) {
        results_->addItem(path);
    }

    void onFinished(bool ok, const QString& error, int count) {
        if (count < 0) {
            statusBar()->showMessage("Cancelled.");
        } else if (!ok) {
            statusBar()->showMessage("Error: " + error);
        } else {
            statusBar()->showMessage(QString("%1 match(es).").arg(count));
        }
        stopWorker();
    }

private:
    void stopWorker() {
        if (worker_) worker_->cancel();
        if (thread_) {
            thread_->quit();
            thread_->wait();
            thread_->deleteLater();
            thread_ = nullptr;
        }
        if (worker_) {
            worker_->deleteLater();
            worker_ = nullptr;
        }
    }

    QLineEdit*   searchBox_ = nullptr;
    QListWidget* results_   = nullptr;
    QCheckBox*   filesOnly_ = nullptr;
    QCheckBox*   dirsOnly_  = nullptr;
    QCheckBox*   exactMatch_ = nullptr;
    QCheckBox*   caseSensitive_ = nullptr;
    QSpinBox*    limit_     = nullptr;
    QTimer*      debounce_  = nullptr;

    QThread*      thread_ = nullptr;
    SearchWorker* worker_ = nullptr;
};

#include "main_gui.moc"

int main(int argc, char* argv[]) {
    QApplication app(argc, argv);
    app.setApplicationName("MacFindRoadACpp");
    MainWindow win;
    win.show();
    return app.exec();
}
