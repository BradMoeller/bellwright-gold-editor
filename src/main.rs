//! Cross-platform GUI for viewing and editing player stats in Bellwright saves.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console on Windows release

use bellwright_gold_editor::SaveFile;
use eframe::egui;
use std::path::PathBuf;

struct Loaded {
    path: PathBuf,
    name: String,
    village: String,
    character: String,
    gold: u64,
}

#[derive(Default)]
struct App {
    loaded: Option<Loaded>,
    gold_input: String,
    // Renown can't be auto-detected (it's one of thousands of identical records),
    // so the player types the value the game shows plus the value they want.
    renown_current_input: String,
    renown_new_input: String,
    status: String,
    is_error: bool,
    pending_drop: Option<PathBuf>,
}

impl App {
    fn new(initial: Option<PathBuf>) -> Self {
        let mut app = Self::default();
        if let Some(path) = initial {
            app.open(path);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        match SaveFile::load(&path) {
            Ok(s) => {
                let gold = match s.find_gold() {
                    Ok(g) => g.value,
                    Err(e) => {
                        self.loaded = None;
                        self.status = e.to_string();
                        self.is_error = true;
                        return;
                    }
                };
                self.gold_input = gold.to_string();
                self.renown_current_input.clear();
                self.renown_new_input.clear();
                self.loaded = Some(Loaded {
                    path,
                    name: s.display_name,
                    village: s.village,
                    character: s.character,
                    gold,
                });
                self.status = "Loaded. Edit values and click Apply.".into();
                self.is_error = false;
            }
            Err(e) => {
                self.loaded = None;
                self.status = e.to_string();
                self.is_error = true;
            }
        }
    }

    fn apply(&mut self) {
        let Some(l) = &self.loaded else { return };
        let gold = match self.gold_input.trim().parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                self.status = "Gold: enter a whole number ≥ 0.".into();
                self.is_error = true;
                return;
            }
        };
        // Renown is optional: edit it only when both fields are filled in.
        let cur = self.renown_current_input.trim();
        let new = self.renown_new_input.trim();
        let renown = if cur.is_empty() && new.is_empty() {
            None
        } else {
            match (cur.parse::<u64>(), new.parse::<u64>()) {
                (Ok(c), Ok(n)) => Some((c, n)),
                _ => {
                    self.status =
                        "Renown: enter both the current (in-game) and new value as whole numbers ≥ 0.".into();
                    self.is_error = true;
                    return;
                }
            }
        };

        // Apply gold first, then renown (different chunks; order doesn't matter).
        let path = l.path.clone();
        if let Err(e) = bellwright_gold_editor::set_gold_on_disk(&path, gold) {
            self.status = e.to_string();
            self.is_error = true;
            return;
        }
        if let Some((c, n)) = renown {
            if let Err(e) = bellwright_gold_editor::set_renown_on_disk(&path, c, n) {
                self.status = e.to_string();
                self.is_error = true;
                return;
            }
        }
        self.open(path);
        let renown_msg = renown
            .map(|(_, n)| format!(", renown to {n}"))
            .unwrap_or_default();
        self.status = format!("Saved. Gold set to {gold}{renown_msg}. Backup at <file>.bak.");
        self.is_error = false;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Bellwright Save Editor");
            ui.add_space(6.0);

            if ui.button("📂  Open save file…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Bellwright save", &["sav"])
                    .set_title("Choose a Bellwright .sav file")
                    .pick_file()
                {
                    self.open(path);
                }
            }

            ctx.input(|i| {
                if let Some(f) = i.raw.dropped_files.first() {
                    if let Some(p) = &f.path {
                        self.pending_drop = Some(p.clone());
                    }
                }
            });
            if let Some(p) = self.pending_drop.take() {
                self.open(p);
            }

            ui.add_space(10.0);

            if let Some(l) = &self.loaded {
                egui::Grid::new("info").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    ui.label("File:");
                    ui.label(l.path.file_name().and_then(|s| s.to_str()).unwrap_or(""));
                    ui.end_row();
                    ui.label("Save name:");
                    ui.label(&l.name);
                    ui.end_row();
                    ui.label("Village:");
                    ui.label(&l.village);
                    ui.end_row();
                    ui.label("Character:");
                    ui.label(&l.character);
                    ui.end_row();
                    ui.label("Current gold:");
                    ui.strong(l.gold.to_string());
                    ui.end_row();
                });

                ui.add_space(12.0);
                egui::Grid::new("inputs").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("New gold:");
                    ui.add(egui::TextEdit::singleline(&mut self.gold_input).desired_width(100.0));
                    ui.label("");
                    ui.end_row();
                    ui.label("Current renown:");
                    ui.add(egui::TextEdit::singleline(&mut self.renown_current_input).desired_width(100.0));
                    ui.label(egui::RichText::new("(value shown in-game)").small().weak());
                    ui.end_row();
                    ui.label("New renown:");
                    ui.add(egui::TextEdit::singleline(&mut self.renown_new_input).desired_width(100.0));
                    ui.label(egui::RichText::new("(leave both blank to skip)").small().weak());
                    ui.end_row();
                });
                ui.add_space(4.0);
                if ui.button("Apply").clicked() {
                    self.apply();
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(
                    "⚠ Close the game (or return to main menu) before applying, then load the save.",
                ).small().italics());
            } else {
                ui.label("Open a save to begin. Saves live under:");
                ui.label(egui::RichText::new(
                    "…/Bellwright/Saved/SaveGames/<id>/Klint_<slot>.sav").monospace().small());
            }

            ui.add_space(12.0);
            if !self.status.is_empty() {
                let color = if self.is_error {
                    egui::Color32::from_rgb(0xCC, 0x33, 0x33)
                } else {
                    egui::Color32::from_rgb(0x2E, 0x8B, 0x57)
                };
                ui.colored_label(color, &self.status);
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let initial = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([600.0, 430.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Bellwright Gold Editor",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_zoom_factor(1.4); // enlarge all text/widgets
            Ok(Box::new(App::new(initial)))
        }),
    )
}
