//! Road_C (Rust) GUI —— egui/eframe 桌面 app（混合引擎完整版）。
//!
//! 界面：搜索框 + 过滤开关 + 结果列表（每行「在 Finder 中显示」/「打开」）+
//! 顶部后端状态条（索引 / searchfs 实时兜底）。
//!
//! 后台：搜索在独立线程跑，结果经 channel 回传，UI 线程只渲染，输入不卡顿。
//! 建索引也在后台线程跑，完成后热切回索引主路径。
//!
//! 非 macOS 目标不编译 GUI（eframe 只在 macOS 拉入）；见 `main` 下的 stub。

#[cfg(target_os = "macos")]
fn main() -> eframe::Result<()> {
    app::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "haifind-c-gui 是 macOS GUI app；请在 macOS 上构建运行，或用 haifind-c-cli 做冒烟测试。"
    );
}

#[cfg(target_os = "macos")]
mod app {
    use haifind_c::engine::{build_index, default_roots, Backend, HybridEngine, Match, SearchOptions};
    use haifind_c::index::default_index_path;
    use haifind_c::reveal;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// 后台线程回传给 UI 的消息。
    enum Msg {
        /// 一次搜索结果：查询序号（丢弃过期结果用）+ 命中 + 后端。
        Results {
            seq: u64,
            matches: Vec<Match>,
            backend: Backend,
        },
        /// 建索引完成。
        IndexBuilt {
            entries: usize,
            err: Option<String>,
        },
    }

    /// 后台线程要处理的请求。
    enum Req {
        Search { seq: u64, opts: SearchOptions },
        BuildIndex { roots: Vec<PathBuf> },
    }

    pub fn run() -> eframe::Result<()> {
        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([900.0, 620.0])
                .with_title("MacFind · Rust/egui · Road_C"),
            ..Default::default()
        };
        eframe::run_native(
            "haifind-c-gui",
            options,
            Box::new(|cc| Ok(Box::new(App::new(cc)))),
        )
    }

    struct App {
        // ── 查询状态 ──
        query: String,
        dirs_only: bool,
        files_only: bool,
        limit: usize,

        // ── 结果 ──
        results: Vec<Match>,
        backend: Backend,
        last_seq: u64, // 最近发出的查询序号
        shown_seq: u64, // 已展示的查询序号（丢弃过期）
        searching: bool,

        // ── 索引状态 ──
        index_present: bool,
        index_len: usize,
        index_path: PathBuf,
        building: bool,
        status_line: String,

        // ── 后台通道 ──
        req_tx: Sender<Req>,
        msg_rx: Receiver<Msg>,
    }

    impl App {
        fn new(cc: &eframe::CreationContext<'_>) -> Self {
            let (req_tx, req_rx) = std::sync::mpsc::channel::<Req>();
            let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
            let ctx = cc.egui_ctx.clone();

            let index_path = default_index_path();

            // 后台工作线程：持有引擎，串行处理请求。
            let engine = Arc::new(Mutex::new(HybridEngine::with_index_path(index_path.clone())));
            spawn_worker(req_rx, msg_tx, ctx, engine.clone());

            let (index_present, index_len) = {
                let e = engine.lock().unwrap();
                (e.has_index(), e.index_len())
            };

            let status_line = if index_present {
                format!("索引已加载：{index_len} 条。搜索走索引主路径。")
            } else {
                "无索引：搜索将走 searchfs() 实时兜底。可点「建索引」切到主路径。".into()
            };

            App {
                query: String::new(),
                dirs_only: false,
                files_only: false,
                limit: 1000,
                results: Vec::new(),
                backend: if index_present {
                    Backend::Index
                } else {
                    Backend::SearchfsFallback
                },
                last_seq: 0,
                shown_seq: 0,
                searching: false,
                index_present,
                index_len,
                index_path,
                building: false,
                status_line,
                req_tx,
                msg_rx,
            }
        }

        /// 发起一次新搜索（防抖由「文本变化才发」保证）。
        fn dispatch_search(&mut self) {
            if self.query.trim().is_empty() {
                self.results.clear();
                self.searching = false;
                return;
            }
            self.last_seq += 1;
            let opts = SearchOptions {
                query: self.query.trim().to_string(),
                dirs_only: self.dirs_only,
                files_only: self.files_only,
                limit: self.limit,
            };
            self.searching = true;
            let _ = self.req_tx.send(Req::Search {
                seq: self.last_seq,
                opts,
            });
        }

        fn dispatch_build_index(&mut self) {
            if self.building {
                return;
            }
            self.building = true;
            self.status_line = "正在建索引（遍历主目录）…这可能需要一会儿。".into();
            let _ = self.req_tx.send(Req::BuildIndex {
                roots: default_roots(),
            });
        }

        /// 排空后台消息。
        fn drain_messages(&mut self) {
            while let Ok(msg) = self.msg_rx.try_recv() {
                match msg {
                    Msg::Results {
                        seq,
                        matches,
                        backend,
                    } => {
                        // 只接受不早于已展示序号的结果，避免旧查询覆盖新查询。
                        if seq >= self.shown_seq {
                            self.shown_seq = seq;
                            self.results = matches;
                            self.backend = backend;
                        }
                        if seq >= self.last_seq {
                            self.searching = false;
                        }
                    }
                    Msg::IndexBuilt { entries, err } => {
                        self.building = false;
                        match err {
                            None => {
                                self.index_present = true;
                                self.index_len = entries;
                                self.status_line =
                                    format!("索引已建立：{entries} 条。已切回索引主路径。");
                                // 重新发起当前查询，走索引。
                                if !self.query.trim().is_empty() {
                                    self.dispatch_search();
                                }
                            }
                            Some(e) => {
                                self.status_line = format!("建索引失败：{e}（仍用 searchfs 兜底）");
                            }
                        }
                    }
                }
            }
        }

        fn backend_badge(&self) -> (&'static str, eframe::egui::Color32) {
            use eframe::egui::Color32;
            match self.backend {
                Backend::Index => ("● 索引主路径", Color32::from_rgb(80, 200, 120)),
                Backend::SearchfsFallback => {
                    ("● searchfs() 实时兜底", Color32::from_rgb(240, 180, 60))
                }
                Backend::Unavailable => ("● 引擎不可用", Color32::from_rgb(220, 90, 90)),
            }
        }
    }

    impl eframe::App for App {
        fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
            use eframe::egui;

            self.drain_messages();

            egui::TopBottomPanel::top("top").show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("Mac Hai Find");
                    ui.label(egui::RichText::new("Road_C · Rust 混合引擎").weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (txt, color) = self.backend_badge();
                        ui.colored_label(color, txt);
                    });
                });
                ui.add_space(4.0);

                // 搜索框：文本变化即重新搜索。
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("输入文件名关键字…（模糊匹配）")
                        .desired_width(f32::INFINITY),
                );
                if resp.changed() {
                    self.dispatch_search();
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let mut changed = false;
                    if ui.checkbox(&mut self.files_only, "仅文件").changed() {
                        if self.files_only {
                            self.dirs_only = false;
                        }
                        changed = true;
                    }
                    if ui.checkbox(&mut self.dirs_only, "仅目录").changed() {
                        if self.dirs_only {
                            self.files_only = false;
                        }
                        changed = true;
                    }
                    ui.separator();
                    ui.label("上限");
                    if ui
                        .add(egui::DragValue::new(&mut self.limit).range(10..=100_000).speed(10))
                        .changed()
                    {
                        changed = true;
                    }
                    ui.separator();
                    if self.building {
                        ui.add_enabled(false, egui::Button::new("建索引中…"));
                        ui.spinner();
                    } else if ui
                        .button(if self.index_present { "重建索引" } else { "建索引" })
                        .on_hover_text(format!("遍历主目录写入 {}", self.index_path.display()))
                        .clicked()
                    {
                        self.dispatch_build_index();
                    }
                    if self.searching {
                        ui.spinner();
                    }
                    if changed {
                        self.dispatch_search();
                    }
                });
                ui.add_space(4.0);
            });

            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&self.status_line).weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{} 条结果", self.results.len())).weak(),
                        );
                    });
                });
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                if self.results.is_empty() {
                    ui.centered_and_justified(|ui| {
                        let hint = if self.query.trim().is_empty() {
                            "在上方输入关键字开始搜索"
                        } else if self.searching {
                            "搜索中…"
                        } else {
                            "没有匹配结果"
                        };
                        ui.label(egui::RichText::new(hint).weak());
                    });
                    return;
                }

                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    // 快照要 reveal/open 的路径，避免在闭包里借用 self.results 同时可变借用。
                    let mut to_reveal: Option<PathBuf> = None;
                    let mut to_open: Option<PathBuf> = None;

                    for m in &self.results {
                        ui.horizontal(|ui| {
                            let icon = if m.is_dir { "📁" } else { "📄" };
                            let name = std::path::Path::new(&m.path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(&m.path);
                            ui.label(icon);
                            ui.label(egui::RichText::new(name).strong());
                            ui.label(egui::RichText::new(&m.path).weak().small());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Finder").on_hover_text("在 Finder 中显示").clicked() {
                                        to_reveal = Some(PathBuf::from(&m.path));
                                    }
                                    if ui.small_button("打开").clicked() {
                                        to_open = Some(PathBuf::from(&m.path));
                                    }
                                },
                            );
                        });
                        ui.separator();
                    }

                    if let Some(p) = to_reveal {
                        if let Err(e) = reveal::reveal_in_finder(&p) {
                            self.status_line = format!("在 Finder 中显示失败：{e}");
                        }
                    }
                    if let Some(p) = to_open {
                        if let Err(e) = reveal::open_path(&p) {
                            self.status_line = format!("打开失败：{e}");
                        }
                    }
                });
            });
        }
    }

    /// 启动后台工作线程：串行处理搜索 / 建索引，结果经 channel 回传并请求重绘。
    fn spawn_worker(
        req_rx: Receiver<Req>,
        msg_tx: Sender<Msg>,
        ctx: eframe::egui::Context,
        engine: Arc<Mutex<HybridEngine>>,
    ) {
        thread::spawn(move || {
            while let Ok(req) = req_rx.recv() {
                match req {
                    Req::Search { seq, opts } => {
                        // 只跑「最新」的搜索：若通道里还堆着更新的搜索请求，跳过旧的。
                        let mut latest = (seq, opts);
                        while let Ok(Req::Search { seq, opts }) = req_rx.try_recv() {
                            latest = (seq, opts);
                        }
                        let (seq, opts) = latest;
                        let res = {
                            let eng = engine.lock().unwrap();
                            eng.search(&opts)
                        };
                        let _ = msg_tx.send(Msg::Results {
                            seq,
                            matches: res.matches,
                            backend: res.backend,
                        });
                        ctx.request_repaint();
                    }
                    Req::BuildIndex { roots } => {
                        let idx_path = {
                            let eng = engine.lock().unwrap();
                            eng.index_path().to_path_buf()
                        };
                        let result = build_index(&roots, &idx_path, false);
                        let msg = match result {
                            Ok(stats) => {
                                // 热切回索引主路径。
                                engine.lock().unwrap().reload_index();
                                Msg::IndexBuilt {
                                    entries: stats.entries,
                                    err: None,
                                }
                            }
                            Err(e) => Msg::IndexBuilt {
                                entries: 0,
                                err: Some(e.to_string()),
                            },
                        };
                        let _ = msg_tx.send(msg);
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
}
