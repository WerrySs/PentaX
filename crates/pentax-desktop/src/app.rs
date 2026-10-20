// SPDX-License-Identifier: GPL-2.0-or-later

use crate::controller::{Action, Controllers};
use crate::library::{self, Inspection, LibraryEntry};
use crate::profile::ProfileEditor;
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use pentax_config::{ConfigError, ConfigStore, GlobalConfig, Theme};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const BACKGROUND: Color32 = Color32::from_rgb(8, 7, 12);
const SIDEBAR: Color32 = Color32::from_rgb(15, 12, 22);
const CARD: Color32 = Color32::from_rgb(24, 19, 34);
const LINE: Color32 = Color32::from_rgb(61, 49, 80);
const VIOLET: Color32 = Color32::from_rgb(139, 92, 246);
const AQUA: Color32 = Color32::from_rgb(98, 231, 214);
const CORAL: Color32 = Color32::from_rgb(255, 145, 127);
const MUTED: Color32 = Color32::from_rgb(168, 161, 181);
const TEXT: Color32 = Color32::from_rgb(250, 248, 255);

fn muted_text(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        MUTED
    } else {
        Color32::from_rgb(92, 80, 110)
    }
}

fn warning_text(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        CORAL
    } else {
        Color32::from_rgb(168, 58, 38)
    }
}

fn accent_text(ui: &egui::Ui, accent: Color32) -> Color32 {
    accent_for_theme(ui.visuals().dark_mode, accent)
}

fn accent_for_theme(dark: bool, accent: Color32) -> Color32 {
    if dark {
        accent
    } else if accent == VIOLET {
        Color32::from_rgb(109, 40, 217)
    } else if accent == AQUA {
        Color32::from_rgb(8, 110, 102)
    } else if accent == CORAL {
        Color32::from_rgb(168, 58, 38)
    } else {
        // Keep custom accents opaque and dark enough for the light surfaces.
        Color32::from_rgb(
            (f32::from(accent.r()) * 0.42) as u8,
            (f32::from(accent.g()) * 0.42) as u8,
            (f32::from(accent.b()) * 0.42) as u8,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Settings,
    Diagnostics,
}

enum Job {
    Chosen(Option<PathBuf>),
    Inspected {
        path: PathBuf,
        add: bool,
        result: Result<Inspection, String>,
    },
    Demo(Result<String, String>),
}

pub struct Desktop {
    page: Page,
    config: GlobalConfig,
    saved_config: GlobalConfig,
    global_load_error: Option<String>,
    store: Option<ConfigStore>,
    entries: Vec<LibraryEntry>,
    selected: Option<usize>,
    inspection: Option<Inspection>,
    inspection_error: Option<String>,
    controllers: Controllers,
    pending: Option<Receiver<Job>>,
    status: Option<(String, bool)>,
    path_input: String,
    profile: ProfileEditor,
    activate: bool,
    horizontal_step: i32,
    slider_focused: bool,
    last_demo: Option<String>,
    library_load_failed: bool,
}

impl Desktop {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (store, mut status) = match library::config_directory() {
            Ok(directory) => (Some(ConfigStore::new(directory)), None),
            Err(error) => (None, Some((error, true))),
        };
        let (config, global_load_error) = match store
            .as_ref()
            .map(ConfigStore::load_global)
            .transpose()
        {
            Ok(Some(config)) => (config, None),
            Ok(None) => (GlobalConfig::default(), None),
            Err(error) => {
                let message = format!(
                    "{error}. Existing preferences have been preserved. Saving is disabled until the file is repaired and PentaX is restarted."
                );
                status = Some((message.clone(), true));
                (GlobalConfig::default(), Some(message))
            }
        };
        let (entries, library_load_failed) = match store
            .as_ref()
            .map(|store| library::load(store.directory()))
            .transpose()
        {
            Ok(entries) => (entries.unwrap_or_default(), false),
            Err(error) => {
                status = Some((error, true));
                (Vec::new(), true)
            }
        };
        apply_appearance(&cc.egui_ctx, &config);
        Self {
            page: Page::Library,
            saved_config: config.clone(),
            global_load_error,
            config,
            store,
            entries,
            selected: None,
            inspection: None,
            inspection_error: None,
            controllers: Controllers::new(),
            pending: None,
            status,
            path_input: String::new(),
            profile: ProfileEditor::default(),
            activate: false,
            horizontal_step: 0,
            slider_focused: false,
            last_demo: None,
            library_load_failed,
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.activate = false;
        self.horizontal_step = 0;
        // Drain device events while unfocused, without navigating the launcher.
        for action in self
            .controllers
            .poll(self.config.input.controller_enabled && ctx.input(|input| input.focused))
        {
            match action {
                Action::Activate => self.activate = true,
                Action::Back => self.back(ctx),
                Action::Previous => {
                    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::Previous))
                }
                Action::Next => {
                    ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::Next))
                }
                Action::Left | Action::Right => {
                    let step = if action == Action::Left { -1 } else { 1 };
                    if self.slider_focused {
                        self.horizontal_step = step;
                    } else {
                        ctx.memory_mut(|memory| {
                            memory.move_focus(if step < 0 {
                                egui::FocusDirection::Previous
                            } else {
                                egui::FocusDirection::Next
                            })
                        });
                    }
                }
            }
        }
        self.slider_focused = false;
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.back(ctx);
        }
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Num0)) {
            self.config.ui.scale = 1.0;
            apply_appearance(ctx, &self.config);
        }
        if ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::O))
            && self.pending.is_none()
        {
            self.choose_file();
        }
        for (key, page) in [
            (egui::Key::Num1, Page::Library),
            (egui::Key::Num2, Page::Settings),
            (egui::Key::Num3, Page::Diagnostics),
        ] {
            if ctx.input(|input| input.modifiers.alt && input.key_pressed(key)) {
                self.page = page;
            }
        }
        let job = match self.pending.as_ref().map(Receiver::try_recv) {
            Some(Ok(job)) => Some(job),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.pending = None;
                self.status = Some((
                    "The background operation stopped unexpectedly. You can try again.".into(),
                    true,
                ));
                None
            }
            _ => None,
        };
        if let Some(job) = job {
            self.pending = None;
            match job {
                Job::Chosen(Some(path)) => self.inspect(path, true),
                Job::Chosen(None) => {}
                Job::Inspected { path, add, result } => match result {
                    Ok(inspection) => {
                        if add {
                            let existing = self.entries.iter().position(|entry| entry.path == path);
                            let index = existing.unwrap_or_else(|| {
                                self.entries.push(LibraryEntry {
                                    path,
                                    title_id: None,
                                });
                                self.entries.len() - 1
                            });
                            self.selected = Some(index);
                            self.load_profile();
                            if let Err(error) = self.save_library() {
                                self.status = Some((error, true));
                            }
                        }
                        self.inspection = Some(inspection);
                        self.inspection_error = None;
                        self.page = Page::Library;
                    }
                    Err(error) => {
                        self.inspection = None;
                        self.inspection_error = Some(error.clone());
                        self.status = Some((error, true));
                    }
                },
                Job::Demo(result) => match result {
                    Ok(result) => {
                        self.last_demo = Some(result.clone());
                        self.status = Some((result, false));
                        self.page = Page::Diagnostics;
                    }
                    Err(error) => {
                        self.last_demo = None;
                        self.status = Some((error, true));
                    }
                },
            }
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn back(&mut self, ctx: &egui::Context) {
        if self.page != Page::Library {
            self.page = Page::Library;
        } else {
            self.selected = None;
            self.inspection = None;
            self.inspection_error = None;
        }
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::Previous));
    }

    fn choose_file(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .set_title("Inspect an executable · read only")
                .add_filter("ELF / SELF", &["bin", "elf", "prx", "sprx"])
                .pick_file();
            let _ = sender.send(Job::Chosen(chosen));
        });
    }

    fn inspect(&mut self, path: PathBuf, add: bool) {
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.inspection = None;
        self.inspection_error = None;
        if add {
            self.selected = None;
            self.profile.invalidate();
        }
        std::thread::spawn(move || {
            let path = path.canonicalize().unwrap_or(path);
            let result = library::inspect(&path);
            let _ = sender.send(Job::Inspected { path, add, result });
        });
    }

    fn run_demo(&mut self) {
        self.last_demo = None;
        self.status = None;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = pentax_core::demo::run()
                .map(|result| {
                    format!(
                        "Reference guest completed · RAX {} · {} instructions · {} resident bytes",
                        result.result_rax, result.instructions, result.resident_bytes
                    )
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(Job::Demo(result));
        });
    }

    fn run_startup_demo(&mut self) {
        self.last_demo = None;
        self.status = None;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        std::thread::spawn(move || {
            let result = pentax_core::process::startup::run_startup_fixture()
                .map(|report| {
                    format!(
                        "Interpreted startup passed · {} instructions\nProvider {} · service {} · constructor {}",
                        report.retired, report.provider_value, report.service_value,
                        report.constructor_calls
                    )
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(Job::Demo(result));
        });
    }

    fn save_library(&self) -> Result<(), String> {
        if self.library_load_failed {
            return Err("The existing library could not be loaded and has been preserved. This session's additions are temporary.".into());
        }
        let store = self
            .store
            .as_ref()
            .ok_or("No configuration directory available. Library changes are temporary.")?;
        library::save(store.directory(), &self.entries)
    }

    fn load_profile(&mut self) {
        self.profile = ProfileEditor::default();
        self.profile.title = self
            .selected
            .and_then(|index| self.entries.get(index))
            .and_then(|entry| entry.title_id.clone())
            .unwrap_or_default();
        if !self.profile.title.is_empty()
            && let Some(store) = &self.store
            && let Err(error) = self.profile.load(store)
        {
            self.status = Some((format!("Profile preserved: {error}"), true));
        }
    }

    fn save_profile(&mut self) {
        let result = (|| {
            let store = self
                .store
                .as_ref()
                .ok_or("No configuration directory available")?;
            let saved = self.profile.save(store)?;
            if let Some(index) = self.selected
                && let Some(entry) = self.entries.get_mut(index)
            {
                entry.title_id = Some(saved.title.to_string());
            }
            Ok::<Option<String>, String>(saved.durability_warning)
        })();
        match result {
            Ok(warning) => {
                self.status = Some(profile_save_status(self.save_library(), warning));
            }
            Err(error) => self.status = Some((error, true)),
        }
    }

    fn accent(&self) -> Color32 {
        parse_color(&self.config.ui.accent).unwrap_or(VIOLET)
    }
    fn card_color(&self, ui: &egui::Ui) -> Color32 {
        if ui.visuals().dark_mode {
            CARD
        } else {
            Color32::from_rgb(245, 243, 250)
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent();
        let dark = ui.visuals().dark_mode;
        egui::Panel::left("navigation")
            .resizable(false)
            .size_range(205.0..=205.0)
            .frame(
                egui::Frame::new()
                    .fill(if dark {
                        SIDEBAR
                    } else {
                        Color32::from_rgb(242, 239, 248)
                    })
                    .inner_margin(22),
            )
            .show(ui, |ui| {
                ui.add_space(13.0);
                ui.horizontal(|ui| {
                    crate::brand::show(ui, 40.0);
                    ui.label(RichText::new("PentaX").size(25.0).strong());
                });
                ui.add_space(8.0);
                ui.label(
                    RichText::new("BEYOND THE HARDWARE")
                        .size(9.0)
                        .color(muted_text(ui)),
                );
                ui.add_space(44.0);
                for (number, label, page) in [
                    ("01", "Library", Page::Library),
                    ("02", "Settings", Page::Settings),
                    ("03", "Diagnostics", Page::Diagnostics),
                ] {
                    let text = format!("{number}    {label}");
                    if nav_button(
                        ui,
                        self.activate,
                        &text,
                        self.page == page,
                        accent,
                        [160.0, 45.0],
                    ) {
                        self.page = page;
                    }
                    ui.add_space(8.0);
                }
                let bottom_space = (ui.available_height() - 145.0).max(20.0);
                ui.add_space(bottom_space);
                ui.separator();
                ui.add_space(14.0);
                ui.label(
                    RichText::new("REFERENCE RUNTIME")
                        .size(9.0)
                        .color(accent_text(ui, accent)),
                );
                ui.label(
                    RichText::new(format!("v{}  /  Rust", env!("CARGO_PKG_VERSION"))).size(12.0),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new("An open path.\nOne verified step at a time.")
                        .size(11.0)
                        .color(muted_text(ui)),
                );
            });
    }

    fn heading(&mut self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("PENTAX  /  DEVELOPMENT PREVIEW")
                        .color(accent_text(ui, self.accent()))
                        .size(10.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new(title).size(35.0).strong());
                ui.label(RichText::new(subtitle).color(muted_text(ui)).size(13.0));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::new()
                    .fill(CORAL.gamma_multiply(0.13))
                    .corner_radius(7)
                    .inner_margin(egui::Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("EXPERIMENTAL")
                                .size(10.0)
                                .strong()
                                .color(warning_text(ui)),
                        );
                    });
            });
        });
        ui.add_space(24.0);
    }

    fn library_page(&mut self, ui: &mut egui::Ui) {
        self.heading(
            ui,
            "Library",
            "Your local executables, inspected without changing their files.",
        );
        let accent = self.accent();
        let busy = self.pending.is_some();
        let color = self.card_color(ui);
        egui::Frame::new()
            .fill(color)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(14)
            .inner_margin(25)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("START WITH SOMETHING REAL")
                                .size(10.0)
                                .color(accent_text(ui, accent)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("A small guest. A real first step.")
                                .size(23.0)
                                .strong(),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            "Run the included homebrew through the reference x86-64 interpreter.",
                        );
                        ui.label(
                            RichText::new("Commercial games are not supported yet.")
                                .color(warning_text(ui)),
                        );
                        ui.add_space(18.0);
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.horizontal(|ui| {
                                if nav_button(
                                    ui,
                                    self.activate,
                                    "Run homebrew",
                                    true,
                                    accent,
                                    [155.0, 37.0],
                                ) {
                                    self.run_demo();
                                }
                                if nav_button(
                                    ui,
                                    self.activate,
                                    "Import executable",
                                    false,
                                    accent,
                                    [160.0, 37.0],
                                ) {
                                    self.choose_file();
                                }
                            });
                        });
                    });
                    if ui.available_width() > 110.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            crate::brand::show(ui, 110.0);
                        });
                    }
                });
            });
        ui.add_space(25.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("LOCAL COLLECTION")
                    .size(10.0)
                    .color(muted_text(ui)),
            );
            ui.label(
                RichText::new(format!("{:02}", self.entries.len()))
                    .size(11.0)
                    .color(accent_text(ui, accent)),
            );
            if busy {
                ui.spinner();
                ui.label(RichText::new("Working…").size(11.0).color(muted_text(ui)));
            }
        });
        ui.add_space(11.0);
        if self.entries.is_empty() {
            egui::Frame::new().stroke(Stroke::new(1.0, LINE)).corner_radius(12).inner_margin(24).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("Your library starts here.").size(18.0));
                ui.add_space(6.0);
                ui.label(RichText::new("Import an ELF or eboot.bin to inspect its executable metadata. Original files stay in place.").color(muted_text(ui)));
                ui.add_space(13.0);
                ui.add_enabled_ui(!busy, |ui| { if nav_button(ui, self.activate, "+  Add a local executable", false, accent, [240.0, 35.0]) { self.choose_file(); } });
            });
        } else {
            let mut selection = None;
            for (index, entry) in self.entries.iter().enumerate() {
                ui.push_id(index, |ui| {
                    let fallback = "Executable · not bootable yet";
                    let label = format!(
                        "{}\n{}",
                        entry.name(),
                        entry.title_id.as_deref().unwrap_or(fallback)
                    );
                    if nav_button(
                        ui,
                        self.activate,
                        &label,
                        self.selected == Some(index),
                        accent,
                        [ui.available_width(), 58.0],
                    ) && !busy
                    {
                        selection = Some(index);
                    }
                });
                ui.add_space(6.0);
            }
            if let Some(index) = selection {
                self.selected = Some(index);
                self.load_profile();
                self.inspect(self.entries[index].path.clone(), false);
            }
        }
        ui.add_space(14.0);
        egui::CollapsingHeader::new("Import from a path").show(ui, |ui| {
            ui.horizontal(|ui| {
                let hint = "Full path to ELF or eboot.bin";
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input)
                        .hint_text(hint)
                        .desired_width((ui.available_width() - 110.0).max(100.0)),
                );
                let enter =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                ui.add_enabled_ui(!busy && !self.path_input.trim().is_empty(), |ui| {
                    if nav_button(ui, self.activate, "Inspect", false, accent, [90.0, 30.0])
                        || enter
                    {
                        self.inspect(PathBuf::from(self.path_input.trim()), true);
                    }
                });
            });
        });
        if self.selected.is_some() {
            ui.add_space(20.0);
            self.selected_details(ui);
        }
    }

    fn selected_details(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.selected else { return };
        let Some(entry) = self.entries.get(index).cloned() else {
            return;
        };
        let accent = self.accent();
        egui::Frame::new().fill(self.card_color(ui)).corner_radius(12).inner_margin(20).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("EXECUTABLE INSPECTION").size(10.0).color(accent_text(ui, accent)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        if nav_button(ui, self.activate, "Remove from library", false, CORAL, [160.0, 28.0]) {
                            self.entries.remove(index); self.selected = None; self.inspection = None;
                            if let Err(error) = self.save_library() { self.status = Some((error, true)); }
                        }
                    });
                });
            });
            ui.add_space(7.0);
            ui.label(RichText::new(entry.name()).size(21.0).strong());
            ui.add(egui::Label::new(RichText::new(entry.path.display().to_string()).small().color(muted_text(ui))).wrap());
            ui.add_space(14.0);
            if let Some(inspection) = &self.inspection {
                egui::Grid::new("image_metadata").num_columns(4).spacing([24.0, 7.0]).show(ui, |ui| {
                    field(ui, "Format", &format!("ELF64 / {}", inspection.format));
                    field(ui, "Entry point", &format!("0x{:016x}", inspection.entry)); ui.end_row();
                    field(ui, "File size", &bytes_label(inspection.size));
                    field(ui, "Loadable memory", &bytes_label(inspection.mapped_bytes)); ui.end_row();
                    field(ui, "OS / ABI", &format!("{} / {}", inspection.os_abi, inspection.abi_version));
                    field(ui, "Program headers", &inspection.headers.to_string()); ui.end_row();
                });
                let detailed = if self.profile.is_loaded() { self.profile.document.runtime.enable_debug_logging.unwrap_or(self.config.runtime.enable_debug_logging) } else { self.config.runtime.enable_debug_logging };
                if detailed {
                    ui.add_space(13.0);
                    egui::Grid::new("segments").striped(true).spacing([20.0, 5.0]).show(ui, |ui| {
                        for caption in ["Guest address", "File bytes", "Memory bytes", "Access"] { ui.strong(caption); } ui.end_row();
                        for segment in &inspection.segments {
                            ui.monospace(format!("0x{:016x}", segment.address)); ui.monospace(segment.file_size.to_string()); ui.monospace(segment.memory_size.to_string()); ui.monospace(&segment.permissions); ui.end_row();
                        }
                    });
                }
                ui.add_space(12.0);
                ui.label(RichText::new("Headers inspected. This does not verify payloads or compatibility. Game boot, imports and graphics are not implemented yet.").color(warning_text(ui)).small());
            } else if let Some(error) = &self.inspection_error { ui.colored_label(warning_text(ui), error); }
            else { ui.label("Inspecting executable…"); }
            ui.add_space(12.0);
            if nav_button(ui, self.activate, "Title preferences  →", false, accent, [180.0, 32.0]) { self.page = Page::Settings; }
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        self.heading(ui, "Settings", "Global and per-title preferences.");
        let accent = self.accent();
        egui::Frame::new().fill(self.card_color(ui)).corner_radius(12).inner_margin(23).show(ui, |ui| {
            ui.label(RichText::new("GLOBAL  /  PREFERENCES").size(10.0).color(accent_text(ui, accent))); ui.add_space(17.0);
            ui.horizontal(|ui| {
                ui.label("Color scheme"); ui.add_space(20.0);
                for (label, theme) in [("Midnight", Theme::Dark), ("Daylight", Theme::Light), ("System", Theme::System)] {
                    if nav_button(ui, self.activate, label, self.config.ui.theme == theme, accent, [95.0, 32.0]) { self.config.ui.theme = theme; }
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("Accent"); ui.add_space(57.0);
                for (label, value, color) in [("Violet", "#8B5CF6", VIOLET), ("Aqua", "#62E7D6", AQUA), ("Coral", "#FF917F", CORAL)] {
                    if nav_button(ui, self.activate, label, self.config.ui.accent.eq_ignore_ascii_case(value), color, [95.0, 32.0]) { self.config.ui.accent = value.into(); }
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("Interface scale"); ui.add_space(12.0);
                let response = ui.add(egui::Slider::new(&mut self.config.ui.scale, 0.75..=2.5).step_by(0.05).show_value(false));
                if response.has_focus() { self.slider_focused = true; self.config.ui.scale = (self.config.ui.scale + self.horizontal_step as f32 * 0.05).clamp(0.75, 2.5); }
                ui.label(format!("{:.0}%", self.config.ui.scale * 100.0));
            });
            ui.add_space(17.0); ui.separator(); ui.add_space(15.0);
            toggle(ui, self.activate, &mut self.config.input.controller_enabled, "Controller navigation");
            toggle(ui, self.activate, &mut self.config.runtime.enable_debug_logging, "Detailed inspection diagnostics");
            toggle(ui, self.activate, &mut self.config.system.skip_notice_screen, "Skip notice screens");
            ui.add_space(14.0);
            ui.label(RichText::new("Keyboard: Tab / Shift+Tab to move, Enter or Space to select. Controller: D-pad, A / Cross and B / Circle.").size(11.0).color(muted_text(ui)));
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.store.is_some() && self.global_load_error.is_none(), |ui| {
                    if nav_button(ui, self.activate, "Save global preferences", true, accent, [210.0, 36.0]) {
                        let result = self.store.as_ref().expect("store checked above").save_global(&self.config);
                        self.status = Some(global_save_status(&self.config, &mut self.saved_config, result));
                    }
                });
                if self.config != self.saved_config { ui.label(RichText::new("Unsaved changes").color(warning_text(ui)).small()); }
            });
            if let Some(error) = &self.global_load_error { ui.colored_label(warning_text(ui), error); }
        });
        ui.add_space(19.0);
        egui::Frame::new()
            .fill(self.card_color(ui))
            .corner_radius(12)
            .inner_margin(23)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("PER TITLE  /  OVERRIDES")
                        .size(10.0)
                        .color(accent_text(ui, accent)),
                );
                ui.add_space(12.0);
                if self.selected.is_none() {
                    ui.label(
                        "Select an imported executable in Library to configure its title profile.",
                    );
                    return;
                }
                ui.label("Title ID");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.profile.title)
                        .hint_text("PPSA12345")
                        .char_limit(9)
                        .desired_width(220.0),
                );
                if response.changed() {
                    self.profile.invalidate();
                    self.profile.document = Default::default();
                }
                ui.add_space(9.0);
                ui.horizontal(|ui| {
                    if nav_button(
                        ui,
                        self.activate,
                        "Load profile",
                        false,
                        accent,
                        [130.0, 30.0],
                    ) && let Some(store) = &self.store
                    {
                        match self.profile.load(store) {
                            Ok(()) => {
                                self.status = Some((
                                    "Title profile loaded. Missing values inherit globally.".into(),
                                    false,
                                ))
                            }
                            Err(error) => self.status = Some((error, true)),
                        }
                    }
                    ui.label(
                        RichText::new("Use the identifier from your own dump.")
                            .small()
                            .color(muted_text(ui)),
                    );
                });
                ui.add_space(12.0);
                ui.add_enabled_ui(self.profile.is_loaded(), |ui| {
                    let mut inherit = self.profile.document.runtime.enable_debug_logging.is_none();
                    if toggle(
                        ui,
                        self.activate,
                        &mut inherit,
                        "Inherit global diagnostic detail",
                    ) {
                        self.profile.document.runtime.enable_debug_logging = if inherit {
                            None
                        } else {
                            Some(self.config.runtime.enable_debug_logging)
                        };
                    }
                    if let Some(value) = &mut self.profile.document.runtime.enable_debug_logging {
                        toggle(
                            ui,
                            self.activate,
                            value,
                            "Detailed inspection for this title",
                        );
                    }
                });
                if !self.profile.is_loaded() {
                    ui.label(
                        RichText::new(
                            "Load this title ID before editing or saving its preferences.",
                        )
                        .color(muted_text(ui))
                        .small(),
                    );
                }
                ui.add_space(13.0);
                ui.add_enabled_ui(self.store.is_some() && self.profile.is_loaded(), |ui| {
                    if nav_button(
                        ui,
                        self.activate,
                        "Save title profile",
                        false,
                        accent,
                        [170.0, 34.0],
                    ) {
                        self.save_profile();
                    }
                });
                ui.label(
                    RichText::new(
                        "Stored in games/<title-id>.toml. Global preferences remain independent.",
                    )
                    .small()
                    .color(muted_text(ui)),
                );
            });
        ui.add_space(19.0);
        ui.label(RichText::new("Emulation graphics, audio and runtime tuning will appear as their backends become available.").color(muted_text(ui)).small());
        apply_appearance(ui.ctx(), &self.config);
    }

    fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        self.heading(
            ui,
            "Diagnostics",
            "Evidence from this runtime. No estimated performance counters.",
        );
        let accent = self.accent();
        egui::Frame::new().fill(self.card_color(ui)).corner_radius(12).inner_margin(23).show(ui, |ui| {
            ui.label(RichText::new("RUNTIME CAPABILITIES").size(10.0).color(accent_text(ui, accent))); ui.add_space(15.0);
            for (name, state, ready) in [
                ("Executable metadata", "ELF64 / SELF · x86-64 · read-only headers", true),
                ("Guest memory", "Checked sparse memory, separate from host pointers", true),
                ("Reference CPU", "Bounded subset · bundled synthetic homebrew only", true),
                ("Module startup fixture", "Original interpreted two-module check", true),
                ("Commercial game boot", "Not supported yet", false),
                ("PS5 GPU / audio", "Not implemented yet", false),
            ] {
                ui.horizontal(|ui| { status_dot(ui, if ready { accent_text(ui, accent) } else { warning_text(ui) }, ready); ui.label(RichText::new(name).strong()); ui.label(RichText::new(state).color(muted_text(ui))); }); ui.add_space(10.0);
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(self.pending.is_none(), |ui| { if nav_button(ui, self.activate, "Run built-in verification guest", true, accent, [260.0, 38.0]) { self.run_demo(); } });
            ui.add_space(8.0);
            ui.add_enabled_ui(self.pending.is_none(), |ui| { if nav_button(ui, self.activate, "Run original startup fixture", true, accent, [260.0, 38.0]) { self.run_startup_demo(); } });
            if let Some(result) = &self.last_demo { ui.add_space(13.0); ui.add(egui::Label::new(RichText::new(result).color(accent_text(ui, accent))).wrap()); }
            ui.label(RichText::new("Original fixtures only. Their startup layout and synthetic service do not establish PS5 game compatibility.").small().color(muted_text(ui)));
        });
        ui.add_space(18.0);
        egui::Frame::new()
            .fill(self.card_color(ui))
            .corner_radius(12)
            .inner_margin(23)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("INPUT DEVICES")
                        .size(10.0)
                        .color(accent_text(ui, accent)),
                );
                ui.add_space(12.0);
                let names = self.controllers.names();
                if names.is_empty() {
                    ui.label("No controller connected. Connect one at any time.");
                }
                for name in names {
                    ui.horizontal(|ui| {
                        status_dot(ui, accent_text(ui, accent), true);
                        ui.label(name);
                    });
                }
                if let Some(error) = &self.controllers.error {
                    ui.colored_label(
                        warning_text(ui),
                        format!("Controller backend unavailable: {error}"),
                    );
                }
                if !self.config.input.controller_enabled {
                    ui.colored_label(
                        warning_text(ui),
                        "Controller navigation is disabled in Settings.",
                    );
                }
            });
        ui.add_space(18.0);
        egui::Frame::new()
            .fill(self.card_color(ui))
            .corner_radius(12)
            .inner_margin(23)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("LOCAL STORAGE")
                        .size(10.0)
                        .color(accent_text(ui, accent)),
                );
                ui.add_space(12.0);
                if let Some(store) = &self.store {
                    ui.label(store.directory().display().to_string());
                    ui.label(
                        RichText::new("global.toml  /  games/*.toml  /  library.toml")
                            .monospace()
                            .small()
                            .color(muted_text(ui)),
                    );
                } else {
                    ui.colored_label(
                        warning_text(ui),
                        "No writable preference location could be resolved.",
                    );
                }
            });
    }
}

fn global_save_status(
    config: &GlobalConfig,
    saved: &mut GlobalConfig,
    result: Result<(), ConfigError>,
) -> (String, bool) {
    match result {
        Ok(()) => {
            *saved = config.clone();
            ("Global preferences saved.".into(), false)
        }
        Err(error @ ConfigError::CommittedButNotDurable { .. }) => {
            *saved = config.clone();
            (error.to_string(), true)
        }
        Err(error) => (error.to_string(), true),
    }
}

fn profile_save_status(
    library_result: Result<(), String>,
    durability_warning: Option<String>,
) -> (String, bool) {
    let (mut message, mut error) = match library_result {
        Ok(()) => (
            "Title profile saved separately from global preferences.".into(),
            false,
        ),
        Err(error) => (
            format!(
                "Title preferences were saved, but the library association was not saved: {error}"
            ),
            true,
        ),
    };
    if let Some(warning) = durability_warning {
        message.push(' ');
        message.push_str(&warning);
        error = true;
    }
    (message, error)
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll(ui.ctx());
        egui::Panel::bottom("status_bar")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(if ui.visuals().dark_mode {
                        SIDEBAR
                    } else {
                        Color32::from_gray(230)
                    })
                    .inner_margin(egui::Margin::symmetric(22, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Experimental / commercial games not supported yet")
                            .size(11.0)
                            .color(warning_text(ui)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let connected = self.controllers.names().len();
                        let connection = if connected == 0 {
                            "KEYBOARD + MOUSE".into()
                        } else {
                            format!("{connected} CONTROLLER(S) · D-PAD / A / B")
                        };
                        ui.label(RichText::new(connection).size(9.0).color(muted_text(ui)));
                    });
                });
            });
        self.sidebar(ui);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if ui.visuals().dark_mode {
                        BACKGROUND
                    } else {
                        Color32::from_rgb(247, 249, 252)
                    })
                    .inner_margin(30),
            )
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width().max(440.0));
                        if let Some((message, error)) = self.status.clone() {
                            egui::Frame::new()
                                .fill(if error {
                                    CORAL.gamma_multiply(0.10)
                                } else {
                                    self.accent().gamma_multiply(0.10)
                                })
                                .corner_radius(8)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(message).color(if error {
                                            warning_text(ui)
                                        } else {
                                            accent_text(ui, self.accent())
                                        }));
                                        if nav_button(
                                            ui,
                                            self.activate,
                                            "Dismiss",
                                            false,
                                            self.accent(),
                                            [75.0, 24.0],
                                        ) {
                                            self.status = None;
                                        }
                                    });
                                });
                            ui.add_space(10.0);
                        }
                        match self.page {
                            Page::Library => self.library_page(ui),
                            Page::Settings => self.settings_page(ui),
                            Page::Diagnostics => self.diagnostics_page(ui),
                        }
                    });
            });
    }
}

fn nav_button(
    ui: &mut egui::Ui,
    activate: bool,
    label: &str,
    selected: bool,
    accent: Color32,
    size: [f32; 2],
) -> bool {
    let dark = ui.visuals().dark_mode;
    let button = egui::Button::new(RichText::new(label).size(13.0).color(if selected {
        if dark { BACKGROUND } else { Color32::BLACK }
    } else {
        ui.visuals().text_color()
    }))
    .fill(if selected {
        accent
    } else if dark {
        Color32::from_rgb(35, 29, 47)
    } else {
        Color32::from_rgb(236, 232, 244)
    })
    .stroke(Stroke::new(1.0, if selected { accent } else { LINE }))
    .corner_radius(7);
    let response = ui.add_sized(size, button);
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(3.0),
            9,
            Stroke::new(2.0, accent_text(ui, accent)),
            egui::StrokeKind::Outside,
        );
    }
    if response.gained_focus() {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    response.clicked() || (response.enabled() && response.has_focus() && activate)
}

fn toggle(ui: &mut egui::Ui, activate: bool, value: &mut bool, label: &str) -> bool {
    let mut response = ui.checkbox(value, label);
    if response.enabled() && response.has_focus() && activate {
        *value = !*value;
        response.mark_changed();
    }
    response.changed()
}

fn status_dot(ui: &mut egui::Ui, color: Color32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
    ui.painter().circle(
        rect.center(),
        3.0,
        if filled { color } else { Color32::TRANSPARENT },
        Stroke::new(1.0, color),
    );
}

fn field(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.label(RichText::new(name).color(muted_text(ui)).small());
    ui.label(RichText::new(value).monospace().small());
}
fn bytes_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{bytes} bytes")
    }
}
fn parse_color(value: &str) -> Option<Color32> {
    let value = value.strip_prefix('#')?;
    let hex = u32::from_str_radix(value, 16).ok()?;
    Some(Color32::from_rgb(
        (hex >> 16) as u8,
        (hex >> 8) as u8,
        hex as u8,
    ))
}

fn apply_appearance(ctx: &egui::Context, config: &GlobalConfig) {
    let preference = match config.ui.theme {
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light => egui::ThemePreference::Light,
        Theme::System => egui::ThemePreference::System,
    };
    ctx.set_theme(preference);
    ctx.set_zoom_factor(config.ui.scale);
    let accent = parse_color(&config.ui.accent).unwrap_or(VIOLET);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 9.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.0));
        style.visuals.selection.bg_fill = accent.gamma_multiply(0.3);
        style.visuals.selection.stroke = Stroke::new(1.5, accent);
        let focus_color = accent_for_theme(style.visuals.dark_mode, accent);
        style.visuals.widgets.active.bg_stroke = Stroke::new(2.0, focus_color);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, focus_color);
        if style.visuals.dark_mode {
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.panel_fill = BACKGROUND;
            style.visuals.window_fill = CARD;
        } else {
            style.visuals.override_text_color = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{global_save_status, profile_save_status};
    use pentax_config::{ConfigError, GlobalConfig};
    use std::path::PathBuf;

    #[test]
    fn profile_success_does_not_hide_a_failed_library_association() {
        let (message, error) =
            profile_save_status(Err("Library directory is read-only".into()), None);
        assert!(error);
        assert!(message.contains("association was not saved"));
        assert!(message.contains("Library directory is read-only"));
    }

    #[test]
    fn committed_global_preferences_update_saved_state_and_retain_durability_warning() {
        let mut saved = GlobalConfig::default();
        let mut current = saved.clone();
        current.ui.scale = 1.5;
        let (message, warning) = global_save_status(
            &current,
            &mut saved,
            Err(ConfigError::CommittedButNotDurable {
                path: PathBuf::from("global.toml"),
                source: std::io::Error::other("directory flush failed"),
            }),
        );
        assert_eq!(saved, current);
        assert!(warning);
        assert!(message.contains("directory flush failed"));
    }
}
