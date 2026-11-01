//! metascrub desktop app: a small window where you drop files (or pick them
//! with Browse) and get metadata-free copies saved next to the originals.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui::{self, Color32, RichText, Stroke};
use metascrub_core::{self as core, Options, Report};

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default();
    // Window / taskbar / dock icon (the .exe file icon is embedded by build.rs).
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("metascrub")
            .with_inner_size([440.0, 420.0])
            .with_min_inner_size([360.0, 320.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("metascrub", options, Box::new(|_cc| Ok(Box::new(App::new()))))
}

/// Result of cleaning one file, sent from the worker thread to the window.
struct Row {
    input: String,
    outcome: Result<Done, String>,
}

struct Done {
    output: String,
    removed: Vec<String>,
}

struct App {
    rows: Vec<Row>,
    tx: Sender<Row>,
    rx: Receiver<Row>,
    pending: usize,
    /// Files given on the command line (e.g. dropped onto the app icon).
    startup: Vec<PathBuf>,
}

impl App {
    fn new() -> Self {
        let (tx, rx) = channel();
        let startup = std::env::args_os().skip(1).map(PathBuf::from).collect();
        App { rows: Vec::new(), tx, rx, pending: 0, startup }
    }

    /// Expand folders, then clean everything on a background thread so the
    /// window never freezes on large PDFs.
    fn start(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let mut files = Vec::new();
        for p in paths {
            if p.is_dir() {
                let found = core::files_in_folder(&p);
                if found.is_empty() {
                    self.rows
                        .push(Row { input: file_name(&p), outcome: Err("no supported files in this folder".into()) });
                }
                files.extend(found);
            } else {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        self.pending += files.len();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let opts = Options::default();
            for f in files {
                let outcome = core::clean_file(&f, None, false, &opts)
                    .map(|(out, rep)| Done { output: file_name(&out), removed: summary(&rep) })
                    .map_err(|e| format!("{e:#}"));
                // The window may have been closed; nothing to do then.
                if tx.send(Row { input: file_name(&f), outcome }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// One line per removed item, e.g. "EXIF: GPS location 50.45, 30.52 · Camera model X".
fn summary(rep: &Report) -> Vec<String> {
    rep.removed
        .iter()
        .map(|item| {
            let details: Vec<String> =
                item.fields.iter().map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}: {v}") }).collect();
            if details.is_empty() { item.label.clone() } else { format!("{} ({})", item.label, details.join(" · ")) }
        })
        .collect()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Collect finished results from the worker.
        while let Ok(row) = self.rx.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            self.rows.push(row);
        }

        if !self.startup.is_empty() {
            let files = std::mem::take(&mut self.startup);
            self.start(&ctx, files);
        }

        // Files dropped onto the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.start(&ctx, dropped);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("metascrub");
            ui.label(RichText::new("Removes GPS location, camera details, author names and edit history.").weak());
            ui.add_space(10.0);

            // Drop zone.
            let accent = ui.visuals().selection.bg_fill;
            let stroke = if hovering { Stroke::new(2.0, accent) } else { ui.visuals().widgets.inactive.bg_stroke };
            let fill = if hovering { accent.gamma_multiply(0.15) } else { ui.visuals().faint_bg_color };
            egui::Frame::new().stroke(stroke).fill(fill).corner_radius(10.0).inner_margin(24.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    let text = if hovering { "Release to clean" } else { "Drop files or folders here" };
                    ui.label(RichText::new(text).size(17.0));
                    ui.add_space(8.0);
                    if ui.button("Browse files…").clicked()
                        && let Some(files) = rfd::FileDialog::new()
                            .add_filter("Supported files", core::EXTENSIONS)
                            .add_filter("All files", &["*"])
                            .pick_files()
                    {
                        self.start(&ctx, files);
                    }
                });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Saved next to the original as cleaned_<name>.").small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.rows.is_empty() && self.pending == 0 && ui.small_button("Clear").clicked() {
                        self.rows.clear();
                    }
                    if self.pending > 0 {
                        ui.spinner();
                        ui.label(format!("{} left", self.pending));
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                for (i, row) in self.rows.iter().enumerate() {
                    show_row(ui, i, row);
                }
            });
        });
    }
}

fn show_row(ui: &mut egui::Ui, i: usize, row: &Row) {
    let green = Color32::from_rgb(60, 170, 90);
    let red = Color32::from_rgb(220, 80, 70);
    match &row.outcome {
        Ok(done) if done.removed.is_empty() => {
            ui.label(RichText::new(format!("{} — already clean, nothing to remove", row.input)).color(green));
        }
        Ok(done) => {
            let title = RichText::new(format!("{} — saved as {}", row.input, done.output)).color(green);
            egui::CollapsingHeader::new(title).id_salt(i).show(ui, |ui| {
                for line in &done.removed {
                    ui.label(RichText::new(format!("removed {line}")).small());
                }
            });
        }
        Err(e) => {
            ui.label(RichText::new(format!("{} — failed: {e}", row.input)).color(red));
        }
    }
}
