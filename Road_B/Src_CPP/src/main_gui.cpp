// main_gui.cpp — Qt6 desktop GUI for Road_B (index + fzf engine).
//
// Layout: a search box on top, a "Build Index" button + status line, and a
// results list below. Indexing runs on a worker thread so the UI never blocks;
// typing triggers an instant two-phase search against the in-memory index.

#include "index_engine.hpp"
#include "paths.hpp"
#include "scanner.hpp"

#include <QApplication>
#include <QCheckBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMainWindow>
#include <QPushButton>
#include <QThread>
#include <QVBoxLayout>
#include <QWidget>

#include <memory>

using namespace mff;

// Runs a full filesystem scan + index build off the UI thread.
class IndexWorker : public QThread {
    Q_OBJECT
public:
    explicit IndexWorker(std::vector<std::string> roots, QObject* parent = nullptr)
        : QThread(parent), roots_(std::move(roots)) {}

    // Ownership of the freshly built engine is handed to the UI on success.
    std::shared_ptr<IndexEngine> engine;
    size_t count = 0;

signals:
    void done(bool ok);

protected:
    void run() override {
        engine = std::make_shared<IndexEngine>();
        count = engine->buildFromRoots(roots_);
        // Persist so the next launch / the CLI can reuse it. Non-fatal on error.
        engine->save(defaultIndexPath());
        emit done(count > 0);
    }

private:
    std::vector<std::string> roots_;
};

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    MainWindow() {
        auto* central = new QWidget(this);
        auto* root = new QVBoxLayout(central);

        // --- top row: search box + option toggles ---
        search_ = new QLineEdit;
        search_->setPlaceholderText("Type to search (fuzzy)…");
        search_->setClearButtonEnabled(true);
        root->addWidget(search_);

        auto* opts = new QHBoxLayout;
        filesOnly_ = new QCheckBox("Files only");
        dirsOnly_  = new QCheckBox("Dirs only");
        opts->addWidget(filesOnly_);
        opts->addWidget(dirsOnly_);
        opts->addStretch();
        indexBtn_ = new QPushButton("Build Index");
        opts->addWidget(indexBtn_);
        root->addLayout(opts);

        status_ = new QLabel("No index yet — click Build Index.");
        status_->setStyleSheet("color:#888;");
        root->addWidget(status_);

        results_ = new QListWidget;
        root->addWidget(results_, 1);

        setCentralWidget(central);
        setWindowTitle("MacFind · Qt/C++ · Road_B");
        resize(760, 560);

        connect(search_, &QLineEdit::textChanged, this, &MainWindow::runSearch);
        connect(filesOnly_, &QCheckBox::toggled, this, &MainWindow::runSearch);
        connect(dirsOnly_, &QCheckBox::toggled, this, &MainWindow::runSearch);
        connect(indexBtn_, &QPushButton::clicked, this, &MainWindow::buildIndex);

        // Try to load a previously saved index so search works immediately.
        tryLoadExisting();
    }

private slots:
    void tryLoadExisting() {
        auto eng = std::make_shared<IndexEngine>();
        if (eng->loadMmap(defaultIndexPath()) && !eng->empty()) {
            engine_ = eng;
            status_->setText(QString("Loaded index: %1 entries.")
                                 .arg(engine_->entryCount()));
        }
    }

    void buildIndex() {
        if (worker_ && worker_->isRunning()) return;
        indexBtn_->setEnabled(false);
        status_->setText("Indexing… (walking the filesystem)");

        worker_ = new IndexWorker(defaultRoots(), this);
        connect(worker_, &IndexWorker::done, this, [this](bool ok) {
            if (ok) {
                engine_ = worker_->engine;
                status_->setText(QString("Indexed %1 entries. Ready.")
                                     .arg(worker_->count));
                runSearch();
            } else {
                status_->setText("Indexing produced no entries.");
            }
            indexBtn_->setEnabled(true);
            worker_->deleteLater();
            worker_ = nullptr;
        });
        worker_->start();
    }

    void runSearch() {
        results_->clear();
        if (!engine_ || engine_->empty()) return;
        const std::string q = search_->text().toStdString();
        if (q.empty()) return;

        SearchOptions opts;
        opts.filesOnly = filesOnly_->isChecked();
        opts.dirsOnly  = dirsOnly_->isChecked();
        opts.maxResults = 300;

        auto hits = engine_->search(q, opts);
        for (const auto& h : hits) {
            QString label = QString::fromStdString(h.path);
            if (h.isDir) label += "/";
            results_->addItem(label);
        }
        status_->setText(QString("%1 result(s) · %2 indexed")
                             .arg(hits.size())
                             .arg(engine_->entryCount()));
    }

private:
    QLineEdit*   search_    = nullptr;
    QCheckBox*   filesOnly_ = nullptr;
    QCheckBox*   dirsOnly_  = nullptr;
    QPushButton* indexBtn_  = nullptr;
    QLabel*      status_    = nullptr;
    QListWidget* results_   = nullptr;

    std::shared_ptr<IndexEngine> engine_;
    IndexWorker* worker_ = nullptr;
};

#include "main_gui.moc"

int main(int argc, char** argv) {
    QApplication app(argc, argv);
    MainWindow w;
    w.show();
    return app.exec();
}
