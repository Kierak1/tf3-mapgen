#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use eframe::egui;
use serde::{Deserialize, Serialize};

use tf3_mapgen::{build, mapsize, userdata, GenerateOptions, GenerateSummary, Progress};

const SIZES: [&str; 8] = [
    "tiny",
    "small",
    "medium",
    "large",
    "verylarge",
    "huge",
    "megalomaniac",
    "gigantomaniac",
];
const RATIOS: [&str; 5] = ["1:1", "1:2", "1:3", "1:4", "1:5"];
const ZOOMS: [&str; 9] = ["auto", "8", "9", "10", "11", "12", "13", "14", "15"];

fn main() -> eframe::Result<()> {
    install_panic_hook();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Transport Fever 3 Map Generator")
            .with_inner_size([1024.0, 760.0])
            .with_min_inner_size([820.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Transport Fever 3 Map Generator",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

fn install_panic_hook() {
    let log = log_path();
    std::panic::set_hook(Box::new(move |info| {
        append_log(&log, &format!("PANIC: {info}"));
        append_log(&log, "the application will close");
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("tf3-mapgen crashed")
            .set_description(format!(
                "An unexpected error occurred and the app must close.\n\n{info}\n\nDetails were written to:\n{}",
                log.display()
            ))
            .show();
    }));
}

enum Message {
    Log(String),
    Warn(String),
    Done(Result<Box<GenerateSummary>, String>),
}

struct ChannelProgress {
    tx: Mutex<Sender<Message>>,
}

impl Progress for ChannelProgress {
    fn info(&self, message: &str) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Message::Log(message.to_string()));
        }
    }

    fn warn(&self, message: &str) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Message::Warn(message.to_string()));
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct UiConfig {
    location: String,
    name: String,
    size: String,
    ratio: String,
    water_level_enabled: bool,
    water_level: f64,
    min_height_enabled: bool,
    min_height: f64,
    max_height_enabled: bool,
    max_height: f64,
    sea_depth: f64,
    zoom: String,
    max_towns_enabled: bool,
    max_towns: usize,
    skip_osm: bool,
    flip_x: bool,
    flip_y: bool,
    #[serde(default = "default_true")]
    repair_seams: bool,
    game_dir: String,
    userdata_dir: String,
    out_dir: String,
}

fn default_true() -> bool {
    true
}

impl UiConfig {
    fn recommended() -> Self {
        UiConfig {
            repair_seams: true,
            ..Default::default()
        }
    }
}

struct App {
    location: String,
    name: String,
    size_index: usize,
    ratio_index: usize,
    water_level_enabled: bool,
    water_level: f64,
    min_height_enabled: bool,
    min_height: f64,
    max_height_enabled: bool,
    max_height: f64,
    sea_depth: f64,
    zoom_index: usize,
    max_towns_enabled: bool,
    max_towns: usize,
    skip_osm: bool,
    flip_x: bool,
    flip_y: bool,
    repair_seams: bool,
    game_dir: String,
    userdata_dir: String,
    out_dir: String,
    logs: Vec<String>,
    running: bool,
    rx: Option<Receiver<Message>>,
    summary: Option<GenerateSummary>,
    preview: Option<egui::TextureHandle>,
    error_modal: Option<String>,
    config_path: PathBuf,
    log_path: PathBuf,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_paths(config_path(), log_path(), Some(&cc.egui_ctx))
    }

    #[cfg(test)]
    fn for_test(config_path: PathBuf, log_path: PathBuf) -> Self {
        Self::with_paths(config_path, log_path, None)
    }

    fn with_paths(config_path: PathBuf, log_path: PathBuf, ctx: Option<&egui::Context>) -> Self {
        let config = load_config(&config_path).unwrap_or_else(UiConfig::recommended);
        let size_index = SIZES
            .iter()
            .position(|size| *size == config.size)
            .unwrap_or(2);
        let ratio_index = RATIOS
            .iter()
            .position(|ratio| *ratio == config.ratio)
            .unwrap_or(0);
        let zoom_index = ZOOMS
            .iter()
            .position(|zoom| *zoom == config.zoom)
            .unwrap_or(0);

        let default_game_dir =
            Path::new("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Transport Fever 3");
        let game_dir = if config.game_dir.is_empty() && default_game_dir.is_dir() {
            default_game_dir.display().to_string()
        } else {
            config.game_dir
        };
        let userdata_dir = if config.userdata_dir.is_empty() {
            userdata::find_userdata()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        } else {
            config.userdata_dir
        };

        if let Some(ctx) = ctx {
            ctx.all_styles_mut(|style| {
                style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            });
        }

        let mut app = App {
            location: config.location,
            name: config.name,
            size_index,
            ratio_index,
            water_level_enabled: config.water_level_enabled,
            water_level: config.water_level,
            min_height_enabled: config.min_height_enabled,
            min_height: config.min_height,
            max_height_enabled: config.max_height_enabled,
            max_height: config.max_height,
            sea_depth: if config.sea_depth == 0.0 {
                60.0
            } else {
                config.sea_depth
            },
            zoom_index,
            max_towns_enabled: config.max_towns_enabled,
            max_towns: if config.max_towns == 0 {
                16
            } else {
                config.max_towns
            },
            skip_osm: config.skip_osm,
            flip_x: config.flip_x,
            flip_y: config.flip_y,
            repair_seams: config.repair_seams,
            game_dir,
            userdata_dir,
            out_dir: config.out_dir,
            logs: Vec::new(),
            running: false,
            rx: None,
            summary: None,
            preview: None,
            error_modal: None,
            config_path,
            log_path,
        };
        app.log_line(&format!(
            "--- tf3-mapgen-gui {} starting ---",
            env!("CARGO_PKG_VERSION")
        ));
        app.log_line(&format!("config    : {}", app.config_path.display()));
        app.log_line(&format!("log file  : {}", app.log_path.display()));
        app.log_line(&format!(
            "game dir  : {}",
            if app.game_dir.trim().is_empty() {
                "(auto)".to_string()
            } else {
                app.game_dir.clone()
            }
        ));
        app.log_line(&format!(
            "userdata  : {}",
            if app.userdata_dir.trim().is_empty() {
                "(auto)".to_string()
            } else {
                app.userdata_dir.clone()
            }
        ));
        app
    }

    fn log_line(&mut self, line: &str) {
        self.logs.push(line.to_string());
        append_log(&self.log_path, line);
    }

    fn to_options(&self) -> GenerateOptions {
        GenerateOptions {
            location: non_empty(&self.location),
            lat: None,
            lon: None,
            name: non_empty(&self.name),
            size: SIZES[self.size_index].to_string(),
            ratio: RATIOS[self.ratio_index].to_string(),
            water_level: self.water_level_enabled.then_some(self.water_level),
            min_height: self.min_height_enabled.then_some(self.min_height),
            max_height: self.max_height_enabled.then_some(self.max_height),
            sea_depth: self.sea_depth,
            zoom: if self.zoom_index == 0 {
                None
            } else {
                Some((self.zoom_index as u8) + 7)
            },
            game_dir: non_empty(&self.game_dir).map(PathBuf::from),
            userdata_dir: non_empty(&self.userdata_dir).map(PathBuf::from),
            out: non_empty(&self.out_dir).map(PathBuf::from),
            skip_osm: self.skip_osm,
            osm_endpoint: None,
            max_towns: self.max_towns_enabled.then_some(self.max_towns),
            flip_x: self.flip_x,
            flip_y: self.flip_y,
            repair_seams: self.repair_seams,
        }
    }

    fn start_generate(&mut self, ctx: &egui::Context) {
        self.save_config();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.summary = None;
        self.preview = None;
        self.error_modal = None;
        self.running = true;
        self.log_line(&format!(
            "--- generating: '{}' ({} {}) ---",
            self.location.trim(),
            mapsize::display_size(SIZES[self.size_index]),
            RATIOS[self.ratio_index]
        ));
        let progress = ChannelProgress {
            tx: Mutex::new(tx.clone()),
        };
        let options = self.to_options();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = build::generate(&options, &progress);
            let _ = tx.send(Message::Done(
                result.map(Box::new).map_err(|error| format!("{error:#}")),
            ));
            ctx.request_repaint();
        });
    }

    fn poll_messages(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.rx.take() else {
            return;
        };
        let mut finished = false;
        while let Ok(message) = rx.try_recv() {
            match message {
                Message::Log(line) => self.log_line(&line),
                Message::Warn(line) => self.log_line(&format!("warning: {line}")),
                Message::Done(result) => {
                    finished = true;
                    match result {
                        Ok(summary) => {
                            let text = build::summary_text(&summary);
                            for line in text.lines() {
                                self.log_line(line);
                            }
                            self.load_preview(ctx, &summary.preview);
                            self.summary = Some(*summary);
                        }
                        Err(error) => {
                            self.log_line(&format!("error: {error}"));
                            self.log_line("check the location and the paths, then try again");
                            self.error_modal = Some(error);
                        }
                    }
                }
            }
        }
        if finished {
            self.running = false;
        } else {
            self.rx = Some(rx);
        }
        if self.running {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context, path: &Path) {
        let Ok(image) = image::open(path) else {
            return;
        };
        let image = image.thumbnail(900, 900).to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = image.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        self.preview = Some(ctx.load_texture("preview", color_image, egui::TextureOptions::LINEAR));
    }

    fn save_config(&self) {
        let config = UiConfig {
            location: self.location.clone(),
            name: self.name.clone(),
            size: SIZES[self.size_index].to_string(),
            ratio: RATIOS[self.ratio_index].to_string(),
            water_level_enabled: self.water_level_enabled,
            water_level: self.water_level,
            min_height_enabled: self.min_height_enabled,
            min_height: self.min_height,
            max_height_enabled: self.max_height_enabled,
            max_height: self.max_height,
            sea_depth: self.sea_depth,
            zoom: ZOOMS[self.zoom_index].to_string(),
            max_towns_enabled: self.max_towns_enabled,
            max_towns: self.max_towns,
            skip_osm: self.skip_osm,
            flip_x: self.flip_x,
            flip_y: self.flip_y,
            repair_seams: self.repair_seams,
            game_dir: self.game_dir.clone(),
            userdata_dir: self.userdata_dir.clone(),
            out_dir: self.out_dir.clone(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&config) {
            if let Some(parent) = self.config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&self.config_path, text);
        }
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_messages(&ctx);

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Transport Fever 3 Map Generator");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.running {
                        ui.spinner();
                        ui.label("Generating...");
                    }
                });
            });
            ui.label(
                "Paste a Google Maps link, a place name (e.g. \"Interlaken, Switzerland\") or lat,lon. \
                 The map files are written into the game's userdata folder.",
            );
            ui.add_space(6.0);
        });

        egui::Panel::left("settings")
            .resizable(false)
            .exact_size(370.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Location");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.location)
                            .hint_text(
                                "https://maps.app.goo.gl/... or Interlaken, Switzerland or 46.68,7.86",
                            )
                            .desired_width(f32::INFINITY),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Map name (optional)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.name)
                                .hint_text("auto")
                                .desired_width(170.0),
                        );
                    });
                    ui.add_space(8.0);

                    ui.heading("Map");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("Size")
                            .selected_text(mapsize::display_size(SIZES[self.size_index]))
                            .show_ui(ui, |ui| {
                                for (index, size) in SIZES.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.size_index,
                                        index,
                                        mapsize::display_size(size),
                                    );
                                }
                            });
                        egui::ComboBox::from_label("Ratio")
                            .selected_text(RATIOS[self.ratio_index])
                            .show_ui(ui, |ui| {
                                for (index, ratio) in RATIOS.iter().enumerate() {
                                    ui.selectable_value(&mut self.ratio_index, index, *ratio);
                                }
                            });
                    });
                    if mapsize::is_experimental(SIZES[self.size_index]) {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 140, 40),
                            "Experimental size: requires experimentalMapFeatures = true in settings.lua",
                        );
                    }
                    ui.add_space(8.0);

                    ui.heading("Paths");
                    ui.label("Transport Fever 3 game folder (optional)");
                    ui.horizontal(|ui| {
                        if ui.button("Browse...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.game_dir = folder.display().to_string();
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.game_dir)
                                .hint_text("C:\\...\\steamapps\\common\\Transport Fever 3")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    path_status(ui, &self.game_dir);

                    ui.label("Userdata folder (where the maps are written)");
                    ui.horizontal(|ui| {
                        if ui.button("Browse...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.userdata_dir = folder.display().to_string();
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.userdata_dir)
                                .hint_text("auto-detected")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Detect from game folder").clicked() {
                            let detected = if self.game_dir.trim().is_empty() {
                                userdata::find_userdata()
                            } else {
                                userdata::find_from_game_dir(Path::new(self.game_dir.trim()))
                            };
                            if let Some(path) = detected {
                                self.userdata_dir = path.display().to_string();
                            }
                        }
                        if ui.button("Auto-detect").clicked() {
                            if let Some(path) = userdata::find_userdata() {
                                self.userdata_dir = path.display().to_string();
                            }
                        }
                    });
                    path_status(ui, &self.userdata_dir);

                    ui.label("Output folder (optional - overrides the userdata folder)");
                    ui.horizontal(|ui| {
                        if ui.button("Browse...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.out_dir = folder.display().to_string();
                            }
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.out_dir)
                                .hint_text("(not set)")
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.add_space(8.0);

                    ui.collapsing("Advanced", |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.water_level_enabled, "Water level");
                            ui.add_enabled(
                                self.water_level_enabled,
                                egui::DragValue::new(&mut self.water_level)
                                    .range(-500.0..=5000.0)
                                    .speed(1.0)
                                    .suffix(" m"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.min_height_enabled, "Min height");
                            ui.add_enabled(
                                self.min_height_enabled,
                                egui::DragValue::new(&mut self.min_height)
                                    .range(-1000.0..=8000.0)
                                    .speed(1.0)
                                    .suffix(" m"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.max_height_enabled, "Max height");
                            ui.add_enabled(
                                self.max_height_enabled,
                                egui::DragValue::new(&mut self.max_height)
                                    .range(-1000.0..=9000.0)
                                    .speed(1.0)
                                    .suffix(" m"),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Sea depth");
                            ui.add(
                                egui::DragValue::new(&mut self.sea_depth)
                                    .range(10.0..=500.0)
                                    .speed(1.0)
                                    .suffix(" m"),
                            );
                        });
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_label("Elevation zoom")
                                .selected_text(ZOOMS[self.zoom_index])
                                .show_ui(ui, |ui| {
                                    for (index, zoom) in ZOOMS.iter().enumerate() {
                                        ui.selectable_value(&mut self.zoom_index, index, *zoom);
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.max_towns_enabled, "Max towns");
                            ui.add_enabled(
                                self.max_towns_enabled,
                                egui::DragValue::new(&mut self.max_towns).range(0..=200),
                            );
                        });
                        ui.checkbox(&mut self.skip_osm, "Skip OpenStreetMap (terrain only)");
                        ui.checkbox(&mut self.repair_seams, "Repair elevation data seams")
                            .on_hover_text(
                                "Detects and interpolates narrow artificial trenches/cliffs \
                                 in the elevation data (e.g. dataset seams) before generating",
                            );
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.flip_x, "Flip east/west");
                            ui.checkbox(&mut self.flip_y, "Flip north/south");
                        });
                        ui.label(
                            egui::RichText::new(
                                "Use the flip options only if the imported map looks mirrored. \
                                 The *_biomes.png mask is experimental.",
                            )
                            .small()
                            .weak(),
                        );
                    });
                });
            });

        egui::Panel::bottom("actions").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let generate =
                    egui::Button::new(egui::RichText::new("Generate map").size(16.0).strong())
                        .min_size(egui::vec2(160.0, 30.0));
                if ui
                    .add_enabled(!self.running, generate)
                    .on_hover_text(
                        "Fetches elevation + OpenStreetMap data and writes the TF3 map files",
                    )
                    .clicked()
                {
                    self.start_generate(&ctx);
                }
                if let Some(summary) = &self.summary {
                    if ui.button("Open heightmaps folder").clicked() {
                        if let Some(parent) = summary.heightmap.parent() {
                            open_path(parent);
                        }
                    }
                    if ui.button("Open preview").clicked() {
                        open_path(&summary.preview);
                    }
                    if ui.button("Open settings.txt").clicked() {
                        open_path(&summary.settings);
                    }
                    if ui.button("Open output folder").clicked() {
                        open_path(&summary.output_root);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Open log file").clicked() {
                        if !self.log_path.exists() {
                            append_log(&self.log_path, "log opened");
                        }
                        open_path(&self.log_path);
                    }
                    if let Some(summary) = &self.summary {
                        ui.label(format!(
                            "{} towns | {:.0}..{:.0} m | water {:.0} m",
                            summary.town_count, summary.min_h, summary.max_h, summary.water_level
                        ));
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(texture) = &self.preview {
                let available = ui.available_height();
                let size = texture.size_vec2();
                let max_height = (available * 0.5).max(180.0);
                let scale = (max_height / size.y).min(1.0);
                ui.add(
                    egui::Image::new(egui::load::SizedTexture::new(texture.id(), size * scale))
                        .max_width(ui.available_width()),
                );
                ui.separator();
            }

            ui.label(egui::RichText::new("Log").strong());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.monospace(line);
                    }
                });
        });

        if let Some(error) = self.error_modal.clone() {
            let response = egui::Modal::new(egui::Id::new("error_modal")).show(&ctx, |ui| {
                ui.set_max_width(560.0);
                ui.heading("Generation failed");
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.label(&error);
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy error").clicked() {
                        ui.ctx().copy_text(error.clone());
                    }
                    if ui.button("Open log file").clicked() {
                        open_path(&self.log_path);
                    }
                    if ui.button("Close").clicked() {
                        self.error_modal = None;
                    }
                });
            });
            if response.should_close() {
                self.error_modal = None;
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_config();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_config();
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn path_status(ui: &mut egui::Ui, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ui.label(egui::RichText::new("auto-detect").small().weak());
    } else if Path::new(trimmed).is_dir() {
        ui.colored_label(
            egui::Color32::from_rgb(60, 160, 80),
            egui::RichText::new("folder exists").small(),
        );
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(200, 120, 60),
            egui::RichText::new("folder does not exist yet (it will be created)").small(),
        );
    }
}

fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn appdata_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("TF3_MAPGEN_CONFIG") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    appdata_dir().join("tf3-mapgen").join("ui.json")
}

fn log_path() -> PathBuf {
    if let Ok(path) = std::env::var("TF3_MAPGEN_LOG") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    appdata_dir()
        .join("tf3-mapgen")
        .join("logs")
        .join("tf3-mapgen.log")
}

fn append_log(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "[{}] {}", utc_timestamp(), line);
    }
}

fn utc_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0) as i64;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn load_config(path: &Path) -> Option<UiConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    fn test_app(tag: &str) -> App {
        let dir =
            std::env::temp_dir().join(format!("tf3-mapgen-gui-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        App::for_test(dir.join("ui.json"), dir.join("test.log"))
    }

    #[test]
    fn renders_main_widgets() {
        let mut harness =
            Harness::new_ui_state(|ui, app: &mut App| app.draw(ui), test_app("render"));
        harness.run();
        harness.get_by_label("Generate map");
        harness.get_by_label("Location");
        harness.get_by_label("Paths");
        harness.get_by_label("Advanced");
        harness.get_by_label("Open log file");
        assert!(harness
            .state()
            .logs
            .iter()
            .any(|line| line.contains("starting")));
    }

    #[test]
    fn generation_without_location_shows_error_modal() {
        let mut harness =
            Harness::new_ui_state(|ui, app: &mut App| app.draw(ui), test_app("error"));
        harness.get_by_label("Generate map").click();
        for _ in 0..200 {
            harness.run_steps(1);
            if harness.state().error_modal.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let error = harness.state().error_modal.clone();
        assert!(error.is_some(), "error modal did not appear");
        assert!(error.unwrap().contains("no location given"));
        let log = std::fs::read_to_string(&harness.state().log_path).unwrap();
        assert!(
            log.contains("no location given"),
            "log did not contain the error"
        );

        harness.run();
        harness.get_by_label("Close").click();
        harness.run();
        assert!(harness.state().error_modal.is_none());
    }

    #[test]
    fn advanced_checkboxes_update_state() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 1200.0))
            .build_ui_state(|ui, app: &mut App| app.draw(ui), test_app("advanced"));
        harness.run();
        harness.get_by_label("Advanced").click();
        harness.run();
        harness
            .get_by_label("Skip OpenStreetMap (terrain only)")
            .click();
        harness.run();
        assert!(harness.state().skip_osm);
        harness.get_by_label("Flip north/south").click();
        harness.run();
        assert!(harness.state().flip_y);
        harness.get_by_label("Max towns").click();
        harness.run();
        assert!(harness.state().max_towns_enabled);
        harness.get_by_label("Repair elevation data seams").click();
        harness.run();
        assert!(!harness.state().repair_seams);
    }

    #[test]
    fn options_mapping_uses_ui_values() {
        let mut app = test_app("options");
        app.location = "  Interlaken, Switzerland  ".to_string();
        app.name = String::new();
        app.size_index = 1;
        app.ratio_index = 1;
        app.skip_osm = true;
        app.flip_y = true;
        app.zoom_index = 3;
        app.max_towns_enabled = true;
        app.max_towns = 7;
        let options = app.to_options();
        assert_eq!(options.location.as_deref(), Some("Interlaken, Switzerland"));
        assert_eq!(options.name, None);
        assert_eq!(options.size, "small");
        assert_eq!(options.ratio, "1:2");
        assert_eq!(options.zoom, Some(10));
        assert_eq!(options.max_towns, Some(7));
        assert!(options.skip_osm);
        assert!(options.flip_y);
        assert!(options.repair_seams);
    }

    #[test]
    fn config_round_trip() {
        let dir = std::env::temp_dir().join(format!("tf3-mapgen-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config_file = dir.join("ui.json");
        let mut app = App::for_test(config_file.clone(), dir.join("test.log"));
        app.location = "46.0,9.26".to_string();
        app.skip_osm = true;
        app.sea_depth = 42.0;
        app.save_config();

        let reloaded = App::for_test(config_file.clone(), dir.join("test.log"));
        assert_eq!(reloaded.location, "46.0,9.26");
        assert!(reloaded.skip_osm);
        assert!((reloaded.sea_depth - 42.0).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_file_gets_timestamped_lines() {
        let dir = std::env::temp_dir().join(format!("tf3-mapgen-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log_file = dir.join("logs").join("test.log");
        append_log(&log_file, "hello");
        let text = std::fs::read_to_string(&log_file).unwrap();
        assert!(text.contains("hello"));
        assert!(text.starts_with('['));
        assert!(text.contains("Z]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "requires network access (OpenStreetMap + elevation tiles)"]
    fn full_generation_through_gui() {
        let dir = std::env::temp_dir().join(format!("tf3-mapgen-gui-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let output = dir.join("output");

        let mut harness = Harness::new_ui_state(|ui, app: &mut App| app.draw(ui), test_app("e2e"));
        harness.state_mut().location = "46.0,9.26".to_string();
        harness.state_mut().out_dir = output.display().to_string();
        harness.state_mut().size_index = 0;
        harness.get_by_label("Generate map").click();
        for _ in 0..900 {
            harness.run_steps(1);
            if harness.state().summary.is_some() || harness.state().error_modal.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(error) = &harness.state().error_modal {
            panic!("generation failed: {error}");
        }
        let summary = harness.state().summary.as_ref().expect("no summary");
        assert_eq!(summary.width, 1025);
        assert!(summary.heightmap.exists());
        assert!(summary.towns.exists());
        assert!(summary.preview.exists());
        harness.run();
        harness.get_by_label("Open heightmaps folder");
        harness.get_by_label("Open preview");
        harness.get_by_label("Open settings.txt");
        harness.get_by_label("Open output folder");
        let log = std::fs::read_to_string(&harness.state().log_path).unwrap();
        assert!(log.contains("Next steps in Transport Fever 3"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
