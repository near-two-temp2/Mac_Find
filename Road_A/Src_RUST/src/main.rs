//! Road_A (Rust) GUI — macOS instant filename search.
//!
//! An egui/eframe desktop app: a search box + options row + scrollable results
//! list. Each query spins the `searchfs()`-backed engine on a background thread
//! so typing never blocks the UI; results stream back through a channel and a
//! generation counter discards stale responses from older queries.
//!
//! On non-macOS targets `main` degrades to a short message so the crate still
//! builds everywhere; the real binary is only ever produced on macos-latest.

// Hide the console window on the (irrelevant here) Windows target.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "mac-find-gui: this GUI targets macOS (searchfs syscall). \
         Build/run on macOS. See README.md."
    );
}

#[cfg(target_os = "macos")]
fn main() -> eframe::Result<()> {
    use eframe::egui;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([420.0, 320.0])
            .with_title("MacFind · Rust/egui · Road_A"),
        ..Default::default()
    };

    eframe::run_native(
        "Mac Find — Road A",
        native_options,
        Box::new(|_cc| Ok(Box::new(app::FindApp::default()))),
    )
}

#[cfg(target_os = "macos")]
mod app {
    use eframe::egui;
    use mac_find_a_rust::engine::{SearchHit, SearchOptions};
    use mac_find_a_rust::search;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// A finished search response tagged with the generation it belongs to, so
    /// the UI can drop results from a query the user has already moved past.
    struct SearchResult {
        generation: u64,
        hits: Vec<SearchHit>,
        elapsed_ms: u128,
    }

    pub struct FindApp {
        query: String,
        dirs_only: bool,
        files_only: bool,
        exact_match: bool,
        case_sensitive: bool,
        limit_text: String,

        /// Monotonic query id; only results matching `generation` are shown.
        generation: u64,
        searching: bool,
        hits: Vec<SearchHit>,
        last_elapsed_ms: u128,
        status: String,

        tx: Sender<SearchResult>,
        rx: Receiver<SearchResult>,
        /// Guards against launching two engine calls concurrently; the engine
        /// itself is stateless so this is just to keep the thread count at one.
        worker_busy: Arc<Mutex<()>>,
    }

    impl Default for FindApp {
        fn default() -> Self {
            let (tx, rx) = std::sync::mpsc::channel();
            FindApp {
                query: String::new(),
                dirs_only: false,
                files_only: false,
                exact_match: false,
                case_sensitive: false,
                limit_text: "1000".to_string(),
                generation: 0,
                searching: false,
                hits: Vec::new(),
                last_elapsed_ms: 0,
                status: "Type to search filenames across all volumes.".to_string(),
                tx,
                rx,
                worker_busy: Arc::new(Mutex::new(())),
            }
        }
    }

    impl FindApp {
        fn current_options(&self) -> SearchOptions {
            let limit = self.limit_text.trim().parse::<usize>().unwrap_or(1000);
            SearchOptions {
                query: self.query.trim().to_string(),
                dirs_only: self.dirs_only,
                files_only: self.files_only,
                exact_match: self.exact_match,
                case_sensitive: self.case_sensitive,
                limit,
            }
        }

        /// Kick off a background search for the current query/options.
        fn launch_search(&mut self, ctx: &egui::Context) {
            let opts = self.current_options();
            if opts.query.is_empty() {
                self.hits.clear();
                self.searching = false;
                self.status = "Type to search filenames across all volumes.".to_string();
                return;
            }
            if opts.dirs_only && opts.files_only {
                self.status = "‘Files only’ and ‘Dirs only’ are mutually exclusive.".to_string();
                return;
            }

            self.generation += 1;
            self.searching = true;
            self.status = format!("Searching for “{}”…", opts.query);

            let gen = self.generation;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let busy = Arc::clone(&self.worker_busy);

            thread::spawn(move || {
                // Serialize engine calls; if one is running, wait for it.
                let _guard = busy.lock().unwrap();
                let start = std::time::Instant::now();
                let hits = search(&opts);
                let _ = tx.send(SearchResult {
                    generation: gen,
                    hits,
                    elapsed_ms: start.elapsed().as_millis(),
                });
                // Wake the UI thread to consume the result.
                ctx.request_repaint();
            });
        }

        /// Drain any completed searches, keeping only the freshest generation.
        fn drain_results(&mut self) {
            while let Ok(res) = self.rx.try_recv() {
                if res.generation == self.generation {
                    self.hits = res.hits;
                    self.last_elapsed_ms = res.elapsed_ms;
                    self.searching = false;
                    self.status = format!(
                        "{} result(s) in {} ms",
                        self.hits.len(),
                        self.last_elapsed_ms
                    );
                }
                // else: stale generation, discard.
            }
        }

        /// Reveal a hit in Finder via `open -R`.
        fn reveal_in_finder(path: &str) {
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn();
        }
    }

    impl eframe::App for FindApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            self.drain_results();

            egui::TopBottomPanel::top("controls").show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    let resp = ui.add_sized(
                        [ui.available_width() - 90.0, 24.0],
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search filenames…"),
                    );
                    let go = ui.button("Search").clicked();
                    if go
                        || (resp.changed())
                        || (resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        self.launch_search(ctx);
                    }
                });

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let mut changed = false;
                    changed |= ui.checkbox(&mut self.files_only, "Files only").changed();
                    changed |= ui.checkbox(&mut self.dirs_only, "Dirs only").changed();
                    changed |= ui.checkbox(&mut self.exact_match, "Exact match").changed();
                    changed |= ui
                        .checkbox(&mut self.case_sensitive, "Case sensitive")
                        .changed();
                    ui.separator();
                    ui.label("Limit:");
                    let limit_resp = ui.add_sized(
                        [64.0, 20.0],
                        egui::TextEdit::singleline(&mut self.limit_text),
                    );
                    changed |= limit_resp.changed();

                    // Keep the mutually-exclusive checkboxes sane.
                    if self.files_only && self.dirs_only {
                        self.dirs_only = false;
                    }

                    if changed && !self.query.trim().is_empty() {
                        self.launch_search(ctx);
                    }
                });
                ui.add_space(6.0);
            });

            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    if self.searching {
                        ui.spinner();
                    }
                    ui.label(&self.status);
                });
                ui.add_space(3.0);
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                if self.hits.is_empty() && !self.searching {
                    ui.centered_and_justified(|ui| {
                        ui.weak("No results. Try a different term.\n(Full Disk Access may be required for a complete scan.)");
                    });
                    return;
                }

                let row_height = 20.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, self.hits.len(), |ui, range| {
                        for i in range {
                            let hit = &self.hits[i];
                            ui.horizontal(|ui| {
                                let icon = if hit.is_dir { "📁" } else { "📄" };
                                let label = format!("{}  {}", icon, hit.path);
                                let resp = ui.selectable_label(false, label);
                                if resp.double_clicked() {
                                    Self::reveal_in_finder(&hit.path);
                                }
                                resp.on_hover_text("Double-click to reveal in Finder");
                            });
                        }
                    });
            });
        }
    }
}
