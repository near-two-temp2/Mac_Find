//! GUI 入口：egui/eframe 桌面 app（搜索框 + 结果列表）。
//!
//! 后台引擎 = 自建 mmap 二进制索引 + 并行 bitmask 预过滤 + fzf 评分（见 haifind 库）。
//!
//! 交互：
//!   - 顶部搜索框：输入即时搜索（每次改动重新执行两阶段搜索）。
//!   - 「建立/重建索引」按钮：在后台线程遍历 $HOME 建索引，完成后自动加载。
//!   - 结果列表：显示分数、类型（文件/目录）与路径；双击「在 Finder 显示」。
//!   - 单选：仅文件 / 仅目录 / 全部。

use eframe::egui;
use haifind::search::KindFilter;
use haifind::{default_index_path, search, IndexReader, SearchOptions};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

/// 后台索引线程 → UI 的消息。
enum IndexMsg {
    Progress(String),
    Done { entry_count: u64, path: PathBuf },
    Failed(String),
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 600.0])
            .with_min_inner_size([560.0, 360.0])
            .with_title("MacFind · Rust/egui · Road_B"),
        ..Default::default()
    };
    eframe::run_native(
        "mac-hai-find-b-rust",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

struct App {
    query: String,
    kind: KindFilter,
    prev_kind: KindFilter,
    limit: usize,

    reader: Option<IndexReader>,
    index_path: PathBuf,

    results: Vec<haifind::Match>,
    last_query_ms: f64,
    status: String,

    // 后台索引状态。
    indexing: bool,
    index_rx: Option<Receiver<IndexMsg>>,
    index_progress: String,
}

impl App {
    fn new() -> Self {
        let index_path = default_index_path();
        // 启动时若已有索引则自动加载。
        let (reader, status) = match IndexReader::open(&index_path) {
            Ok(r) => {
                let s = format!("已加载索引：{} 条目", r.entry_count());
                (Some(r), s)
            }
            Err(_) => (
                None,
                "未找到索引，请点击「建立/重建索引」。".to_string(),
            ),
        };
        Self {
            query: String::new(),
            kind: KindFilter::All,
            prev_kind: KindFilter::All,
            limit: 200,
            reader,
            index_path,
            results: Vec::new(),
            last_query_ms: 0.0,
            status,
            indexing: false,
            index_rx: None,
            index_progress: String::new(),
        }
    }

    /// 执行一次搜索并更新结果与耗时。
    fn run_search(&mut self) {
        let Some(reader) = &self.reader else {
            self.results.clear();
            return;
        };
        let opts = SearchOptions {
            limit: self.limit,
            kind: self.kind,
            use_ext_filter: true,
        };
        let t0 = Instant::now();
        self.results = search(reader, &self.query, &opts);
        self.last_query_ms = t0.elapsed().as_secs_f64() * 1000.0;
    }

    /// 启动后台索引线程（遍历 $HOME）。
    fn start_indexing(&mut self) {
        if self.indexing {
            return;
        }
        self.indexing = true;
        self.index_progress = "准备中…".to_string();
        let out = self.index_path.clone();
        let (tx, rx): (Sender<IndexMsg>, Receiver<IndexMsg>) = mpsc::channel();
        self.index_rx = Some(rx);

        thread::spawn(move || {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let _ = tx.send(IndexMsg::Progress(format!(
                "遍历 {} …",
                home.display()
            )));
            let mut writer = haifind::IndexWriter::new();
            // GUI 首轮限制条目数，避免全盘遍历卡住演示；可按需放开。
            writer.add_root(&home, Some(2_000_000));
            let _ = tx.send(IndexMsg::Progress(format!(
                "已收集 {} 条目，写入索引…",
                writer.len()
            )));
            match writer.write_to(&out) {
                Ok(stats) => {
                    let _ = tx.send(IndexMsg::Done {
                        entry_count: stats.entry_count,
                        path: out,
                    });
                }
                Err(e) => {
                    let _ = tx.send(IndexMsg::Failed(format!("写入索引失败：{e}")));
                }
            }
        });
    }

    /// 轮询后台索引线程消息。
    fn poll_indexing(&mut self, ctx: &egui::Context) {
        // 先把当前所有消息抽干到本地 Vec，随即释放对 self.index_rx 的借用，
        // 之后再处理消息（其中 IndexMsg::Done 需要 &mut self 调 run_search）。
        let msgs: Vec<IndexMsg> = match &self.index_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        let mut done_or_failed = false;
        for msg in msgs {
            match msg {
                IndexMsg::Progress(p) => self.index_progress = p,
                IndexMsg::Done { entry_count, path } => {
                    match IndexReader::open(&path) {
                        Ok(r) => {
                            self.reader = Some(r);
                            self.status =
                                format!("索引完成并加载：{entry_count} 条目");
                        }
                        Err(e) => {
                            self.status = format!("索引已写入但加载失败：{e}");
                        }
                    }
                    self.indexing = false;
                    done_or_failed = true;
                    self.run_search();
                }
                IndexMsg::Failed(e) => {
                    self.status = e;
                    self.indexing = false;
                    done_or_failed = true;
                }
            }
        }
        if done_or_failed {
            self.index_rx = None;
        }
        if self.indexing {
            // 索引进行中，持续重绘以刷新进度。
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_indexing(ctx);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("搜索：");
                let resp = ui.add_sized(
                    [ui.available_width() - 260.0, 24.0],
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("输入文件名 / 路径片段（支持模糊）"),
                );
                if resp.changed() {
                    self.run_search();
                }

                egui::ComboBox::from_id_salt("kind")
                    .selected_text(match self.kind {
                        KindFilter::All => "全部",
                        KindFilter::FilesOnly => "仅文件",
                        KindFilter::DirsOnly => "仅目录",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.kind, KindFilter::All, "全部");
                        ui.selectable_value(
                            &mut self.kind,
                            KindFilter::FilesOnly,
                            "仅文件",
                        );
                        ui.selectable_value(
                            &mut self.kind,
                            KindFilter::DirsOnly,
                            "仅目录",
                        );
                    });

                let btn = if self.indexing {
                    egui::Button::new("索引中…")
                } else {
                    egui::Button::new("建立/重建索引")
                };
                if ui.add_enabled(!self.indexing, btn).clicked() {
                    self.start_indexing();
                }
            });
            ui.add_space(4.0);
        });

        // kind 组合框在闭包里不便直接触发搜索，改为在闭包外比对上一帧的值：
        // 一旦「仅文件/仅目录/全部」发生变化就重跑一次。
        if self.kind != self.prev_kind {
            self.prev_kind = self.kind;
            self.run_search();
        }

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if self.indexing && !self.index_progress.is_empty() {
                    ui.separator();
                    ui.spinner();
                    ui.label(&self.index_progress);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "{} 条结果 · {:.2} ms",
                        self.results.len(),
                        self.last_query_ms
                    ));
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.reader.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        "尚无索引。点击右上角「建立/重建索引」\n将遍历你的主目录并生成 mmap 二进制索引。",
                    );
                });
                return;
            }

            let row_height = 20.0;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, row_height, self.results.len(), |ui, range| {
                    for i in range {
                        let m = &self.results[i];
                        ui.horizontal(|ui| {
                            let icon = if m.is_dir { "📁" } else { "📄" };
                            ui.monospace(format!("{icon} {:>5}", m.score));
                            let resp = ui.add(
                                egui::Label::new(&m.path)
                                    .sense(egui::Sense::click()),
                            );
                            if resp.double_clicked() {
                                reveal_in_finder(&m.path);
                            }
                            resp.on_hover_text("双击在 Finder 中显示");
                        });
                    }
                });
        });
    }
}

/// 在 Finder 中定位文件（macOS）。索引里的路径是小写化后的展示副本，
/// 对大小写敏感卷可能不完全一致，作为演示交互足够。
#[cfg(target_os = "macos")]
fn reveal_in_finder(path: &str) {
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn reveal_in_finder(_path: &str) {}
