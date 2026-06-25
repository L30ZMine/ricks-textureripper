use std::path::PathBuf;

use egui_dock::{DockArea, DockState, Style};

use crate::layouts::{self, Config};
use crate::project::Project;
use crate::ui::docking::{DockViewer, PanelTab};

pub struct App {

    projects: Vec<Project>,

    active: usize,

    next_project_id: usize,

    config: Config,

    show_about: bool,

    show_info: bool,

    info_logo: Option<egui::TextureHandle>,

    about_logo: Option<egui::TextureHandle>,

    menu_logo: Option<egui::TextureHandle>,

    pending_open: Option<PathBuf>,

    show_save_layout: bool,
    save_layout_name: String,

    show_overwrite_confirm: bool,

    overwrite_dont_ask: bool,

    show_first_run: bool,

    first_run_portable: bool,

    first_run_install: bool,

    update_rx: Option<std::sync::mpsc::Receiver<crate::update::Outcome>>,

    confirm_quit: bool,

    allow_close: bool,

    last_autosave: f64,

    recover_entries: Vec<crate::autosave::Recoverable>,
    show_recover: bool,

    recovery_pending: bool,
}

const AUTOSAVE_INTERVAL: f64 = 30.0;

const INFO_MARKDOWN: &str = include_str!("info.md");

const INFO_LOGO_PNG: &[u8] = include_bytes!("logo_long_g.png");

const ABOUT_LOGO_PNG: &[u8] = include_bytes!("logo_g.png");

impl App {
    pub fn new(startup_open: Option<PathBuf>, first_run: bool) -> Self {
        let config = layouts::load_config();
        let dock = layouts::load_layout(&config.default_layout)
            .unwrap_or_else(|_| layouts::builtin_default());
        let show_info = config.show_info_on_startup;
        let mut app = Self {
            projects: vec![Project::new("unnamed", dock)],
            active: 0,
            next_project_id: 2,
            config,
            show_about: false,
            show_info,
            info_logo: None,
            about_logo: None,
            menu_logo: None,
            pending_open: startup_open,
            show_save_layout: false,
            save_layout_name: String::new(),
            show_overwrite_confirm: false,
            overwrite_dont_ask: false,
            show_first_run: first_run,
            first_run_portable: false,
            first_run_install: false,

            update_rx: Some(crate::update::spawn_check()),
            confirm_quit: false,
            allow_close: false,
            last_autosave: 0.0,
            recover_entries: Vec::new(),
            show_recover: false,
            recovery_pending: false,
        };

        let (crashed, recoverable) = crate::autosave::start_session();
        app.recover_entries = recoverable;
        app.show_recover = crashed && !app.recover_entries.is_empty();

        app.recovery_pending = app.show_recover;
        app.projects[0].set_status("Searching for updates…");
        app
    }

    fn new_project_dock(&self) -> DockState<PanelTab> {
        layouts::load_layout(&self.config.default_layout)
            .unwrap_or_else(|_| layouts::builtin_default())
    }

    fn add_project(&mut self) {
        self.next_project_id += 1;
        let dock = self.new_project_dock();
        self.projects.push(Project::new("unnamed", dock));
        self.active = self.projects.len() - 1;
    }

    fn close_project(&mut self, index: usize) {
        if self.projects.len() <= 1 {
            return;
        }
        self.projects.remove(index);
        if self.active >= self.projects.len() {
            self.active = self.projects.len() - 1;
        }
    }

    fn active_project(&mut self) -> &mut Project {
        &mut self.projects[self.active]
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.active_project().set_status(msg);
    }

    fn set_error(&mut self, msg: impl Into<String>) {
        self.active_project().set_error(msg);
    }

    fn apply_layout(&mut self, name: &str) {
        match layouts::load_layout(name) {
            Ok(dock) => {
                self.active_project().dock_state = dock;
                self.set_status(format!("Loaded layout \"{name}\"."));
            }
            Err(e) => self.set_error(format!("Load layout failed: {e}")),
        }
    }

    fn undo(&mut self, ctx: &egui::Context) {
        let active = self.active;
        if let Some(snap) = self.projects[active].history.undo() {
            crate::snapshot::restore(ctx, &mut self.projects[active], &snap);
            self.projects[active].modified = true;
            self.set_status("Undo.");
        }
    }

    fn redo(&mut self, ctx: &egui::Context) {
        let active = self.active;
        if let Some(snap) = self.projects[active].history.redo() {
            crate::snapshot::restore(ctx, &mut self.projects[active], &snap);
            self.projects[active].modified = true;
            self.set_status("Redo.");
        }
    }

    fn save_project(&mut self) {
        let Some(path) = self.active_project().path.clone() else {
            self.save_project_dialog();
            return;
        };
        match crate::proj_io::save(&path, &self.projects[self.active]) {
            Ok(()) => {
                self.active_project().modified = false;
                self.set_status(format!("Saved {}", path.display()));
            }
            Err(e) => self.set_error(format!("Save failed: {e}")),
        }
    }

    fn save_project_dialog(&mut self) {
        let suggested = format!("{}.{}", self.active_project().name, crate::proj_io::EXTENSION);
        let picked = rfd::FileDialog::new()
            .set_title("Save Project As")
            .add_filter("Rick's Texture Ripper Project", &[crate::proj_io::EXTENSION])
            .set_file_name(suggested)
            .save_file();
        if let Some(path) = picked {
            match crate::proj_io::save(&path, &self.projects[self.active]) {
                Ok(()) => {

                    if let Some(stem) = path.file_stem() {
                        self.active_project().name = stem.to_string_lossy().into_owned();
                    }
                    self.active_project().modified = false;
                    self.active_project().path = Some(path.clone());
                    self.remember_recent(&path);
                    self.set_status(format!("Saved {}", path.display()));
                }
                Err(e) => self.set_error(format!("Save failed: {e}")),
            }
        }
    }

    fn remember_recent(&mut self, path: &std::path::Path) {
        self.config.push_recent(path);
        let _ = layouts::save_config(&self.config);
    }

    fn open_project_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Open Project")
            .add_filter("Rick's Texture Ripper Project", &[crate::proj_io::EXTENSION])
            .pick_file();
        if let Some(path) = picked {

            self.pending_open = Some(path);
        }
    }

    fn open_project_path(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        match crate::proj_io::open(ctx, path) {
            Ok(mut project) => {
                project.path = Some(path.to_path_buf());

                if self.projects.len() == 1
                    && self.projects[0].images.is_empty()
                    && !self.projects[0].modified
                    && self.projects[0].path.is_none()
                {
                    self.projects[0] = project;
                } else {
                    self.projects.push(project);
                }
                self.active = self.projects.len() - 1;
                self.remember_recent(path);
                self.set_status(format!("Opened {}", path.display()));
            }
            Err(e) => self.set_error(format!("Open failed: {e}")),
        }
    }

    fn save_current_layout(&mut self) {
        let name = self.save_layout_name.trim().to_string();
        match layouts::save_layout(&name, &self.active_project().dock_state) {
            Ok(()) => {
                self.show_save_layout = false;
                self.set_status(format!("Saved layout \"{name}\"."));
            }
            Err(e) => self.set_error(format!("Save layout failed: {e}")),
        }
    }

    fn set_default_layout(&mut self, name: &str) {
        self.config.default_layout = name.to_string();
        if let Err(e) = layouts::save_config(&self.config) {
            self.set_error(format!("Could not save config: {e}"));
            return;
        }
        if name.eq_ignore_ascii_case(layouts::DEFAULT_LAYOUT) {
            self.set_status("Default layout set to the built-in \"default\".");
        } else {
            self.add_project();
            self.set_status(format!(
                "Default layout set to \"{name}\"; created an editable project from it."
            ));
        }
    }

    fn delete_layout(&mut self, name: &str) {
        match layouts::delete_layout(name) {
            Ok(()) => {
                if self.config.default_layout.eq_ignore_ascii_case(name) {
                    self.config.default_layout = layouts::DEFAULT_LAYOUT.to_string();
                    let _ = layouts::save_config(&self.config);
                }
                self.set_status(format!("Deleted layout \"{name}\"."));
            }
            Err(e) => self.set_error(format!("Delete failed: {e}")),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        ctx.set_visuals(if self.config.light_mode {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        });

        self.poll_update_check(ctx);

        if let Some(path) = self.pending_open.take() {
            self.open_project_path(ctx, &path);
        }

        self.handle_dropped_files(ctx);

        self.handle_shortcuts(ctx);
        self.menu_bar(ctx);
        self.about_window(ctx);
        self.info_window(ctx);
        self.save_layout_window(ctx);
        self.overwrite_confirm_window(ctx);
        self.first_run_window(ctx);
        self.recover_window(ctx);
        self.quit_guard(ctx);

        let busy = ctx.input(|i| i.pointer.any_down());
        let project = &mut self.projects[self.active];

        let recomputing = project.needs_full
            || project.rips.iter().any(|r| r.dirty)
            || project.images.iter().any(|i| i.dirty);
        crate::image_edit::recompute_dirty_images(ctx, project);
        crate::rip_tool::recompute_dirty(ctx, project, busy);
        if !busy && project.needs_full {
            project.needs_full = false;
            for rip in &mut project.rips {
                rip.dirty = true;
            }
            crate::rip_tool::recompute_dirty(ctx, project, false);
        }
        crate::atlas::repack_if_needed(project);

        if !busy {
            project.commit_history_if_changed();
        }

        self.status_bar(ctx);

        let active = self.active;
        let mut dock = std::mem::replace(
            &mut self.projects[active].dock_state,
            DockState::new(vec![PanelTab::Atlas]),
        );
        {
            let mut viewer = DockViewer {
                project: &mut self.projects[active],
            };
            DockArea::new(&mut dock)
                .style(Style::from_egui(ctx.style().as_ref()))
                .show(ctx, &mut viewer);
        }
        self.projects[active].dock_state = dock;

        if self.pending_open.is_some() || (recomputing && !busy) {
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
            ctx.request_repaint();
        }

        let now = ctx.input(|i| i.time);
        if now - self.last_autosave >= AUTOSAVE_INTERVAL {
            self.last_autosave = now;
            crate::autosave::autosave_modified(&self.projects);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(AUTOSAVE_INTERVAL as u64));
    }
}

impl App {

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let alt = Modifiers::ALT;

        if ctx.input_mut(|i| i.consume_key(alt, Key::Num1)) {
            self.toggle_panel(PanelTab::Texture);
        }
        if ctx.input_mut(|i| i.consume_key(alt, Key::Num2)) {
            self.toggle_panel(PanelTab::Atlas);
        }
        if ctx.input_mut(|i| i.consume_key(alt, Key::Num3)) {
            self.toggle_panel(PanelTab::Rips);
        }
        if ctx.input_mut(|i| i.consume_key(alt, Key::Num4)) {
            self.toggle_panel(PanelTab::ImageEdit);
        }

        if ctx.input_mut(|i| i.consume_key(ctrl, Key::R)) {
            crate::rip_tool::add_rip(&mut self.projects[self.active]);
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::T)) {
            crate::texture_view::open_add_image_dialog(ctx, &mut self.projects[self.active]);
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::F)) {
            self.add_project();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::G)) {
            self.open_project_dialog();
        }

        if ctx.input_mut(|i| i.consume_key(ctrl, Key::Y))
            || ctx.input_mut(|i| i.consume_key(ctrl_shift, Key::Z))
        {
            self.redo(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::Z)) {
            self.undo(ctx);
        }

        if ctx.input_mut(|i| i.consume_key(ctrl_shift, Key::S)) {
            self.save_project_dialog();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::S)) {
            self.save_project();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::X)) {
            crate::atlas::export(&mut self.projects[self.active]);
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if !ctx.wants_keyboard_input() {
            let del = ctx.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Delete)
                    || i.consume_key(Modifiers::NONE, Key::Backspace)
            });
            if del {
                self.delete_selection();
            }
        }
    }

    fn delete_selection(&mut self) {
        let project = &mut self.projects[self.active];
        if let Some(sel) = project.editor.selected.filter(|&s| s < project.rips.len()) {
            crate::texture_view::remove_rip(project, sel);
        } else if let Some(idx) = project.active_image.filter(|&i| i < project.images.len()) {
            crate::texture_view::remove_image(project, idx);
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {

        if self.menu_logo.is_none() {
            if let Ok(img) = image::load_from_memory(ABOUT_LOGO_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.menu_logo =
                    Some(ctx.load_texture("menu_logo", color, egui::TextureOptions::LINEAR));
            }
        }

        let menu_logo = self.menu_logo.clone();
        let menu_logo_alpha: u8 = if self.config.light_mode { 166 } else { 51 };

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {

            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(26.0);
                egui::menu::bar(ui, |ui| {

                if let Some(tex) = &menu_logo {
                    let height = 18.0;
                    let native = tex.size_vec2();
                    let width = native.x / native.y.max(1.0) * height;
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(
                            tex.id(),
                            egui::vec2(width, height),
                        ))
                        .tint(egui::Color32::from_white_alpha(menu_logo_alpha)),
                    );
                }
                ui.menu_button("File", |ui| {
                    if ui
                        .add(egui::Button::new("Add Image").shortcut_text("Ctrl+T"))
                        .clicked()
                    {
                        crate::texture_view::open_add_image_dialog(
                            ctx,
                            &mut self.projects[self.active],
                        );
                        ui.close_menu();
                    }
                    if ui
                        .add(egui::Button::new("Add Rip").shortcut_text("Ctrl+R"))
                        .clicked()
                    {
                        crate::rip_tool::add_rip(&mut self.projects[self.active]);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("New Project").shortcut_text("Ctrl+F"))
                        .clicked()
                    {
                        self.add_project();
                        ui.close_menu();
                    }
                    if ui
                        .add(egui::Button::new("Open…").shortcut_text("Ctrl+G"))
                        .clicked()
                    {
                        self.open_project_dialog();
                        ui.close_menu();
                    }
                    ui.menu_button("Open Recent", |ui| {

                        ui.set_min_width(114.0);
                        ui.set_max_width(228.0);
                        if self.config.recent_files.is_empty() {
                            ui.add_enabled(false, egui::Button::new("(none)"));
                        } else {

                            for path in self.config.recent_files.clone() {
                                let full = path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.display().to_string());
                                let label = ellipsize(&full, 24);
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .min_size(egui::vec2(108.0, 0.0)),
                                    )
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {

                                    self.pending_open = Some(path.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear Recent").clicked() {
                                self.config.recent_files.clear();
                                let _ = layouts::save_config(&self.config);
                                ui.close_menu();
                            }
                        }
                    });
                    if ui
                        .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                        .clicked()
                    {
                        self.save_project();
                        ui.close_menu();
                    }
                    if ui
                        .add(egui::Button::new("Save As…").shortcut_text("Ctrl+Shift+S"))
                        .clicked()
                    {
                        self.save_project_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Export Atlas…").shortcut_text("Ctrl+X"))
                        .clicked()
                    {
                        crate::atlas::export(&mut self.projects[self.active]);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Exit").shortcut_text("Ctrl+Q"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    let can_undo = self.projects[self.active].history.can_undo();
                    let can_redo = self.projects[self.active].history.can_redo();
                    if ui
                        .add_enabled(
                            can_undo,
                            egui::Button::new("Undo").shortcut_text("Ctrl+Z"),
                        )
                        .clicked()
                    {
                        self.undo(ctx);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            can_redo,
                            egui::Button::new("Redo").shortcut_text("Ctrl+Y"),
                        )
                        .clicked()
                    {
                        self.redo(ctx);
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Guide Lines", |ui| {
                        let g = &mut self.projects[self.active].guides;
                        ui.checkbox(&mut g.enabled, "Show guide lines");
                        ui.add(egui::Slider::new(&mut g.vertical, 0..=20).text("Vertical"));
                        ui.add(egui::Slider::new(&mut g.horizontal, 0..=20).text("Horizontal"));
                    });
                    ui.menu_button("Cursor Interp", |ui| {
                        let m = &mut self.projects[self.active].cursor_margin;
                        ui.add(egui::Slider::new(m, 1.0..=50.0).text("Handle margin (px)"));
                        ui.weak("Adjust if you're having issue with grabbing corners, grabbing edges or moving rips.");
                        ui.weak("Lower = harder to grab; Higher = easier.");
                    });
                    ui.menu_button("Preview Quality", |ui| {
                        let q = &mut self.projects[self.active].preview_quality;
                        ui.add(egui::Slider::new(q, 0.1..=1.0).text("Preview scale"));
                        ui.weak("Lower = live previews,faster; Higher = higher quality, slower.");
                    });
                });

                self.layout_menu(ui);
                self.window_menu(ui);

                ui.menu_button("Help", |ui| {
                    if ui.button("Info").clicked() {
                        self.show_info = true;
                        ui.close_menu();
                    }
                    if ui.button("Setup…").clicked() {

                        self.first_run_portable = layouts::is_portable();
                        self.show_first_run = true;
                        ui.close_menu();
                    }
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });

                ui.add_space(16.0);
                ui.separator();
                self.project_tabs(ui);
                });
            });
        });
    }

    fn window_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Window", |ui| {

            for (tab, name, shortcut) in [
                (PanelTab::Texture, "Texture View", "Alt+1"),
                (PanelTab::Atlas, "Atlas View", "Alt+2"),
                (PanelTab::Rips, "Rips Gallery", "Alt+3"),
                (PanelTab::ImageEdit, "Image Edit", "Alt+4"),
            ] {
                let open = self.active_project().dock_state.find_tab(&tab).is_some();
                let label = format!("{} {name}", if open { "✔" } else { "\u{2002}\u{2002}" });
                if ui
                    .add(egui::Button::new(label).shortcut_text(shortcut))
                    .clicked()
                {
                    self.toggle_panel(tab);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui
                .checkbox(&mut self.config.light_mode, "Light Mode")
                .changed()
            {
                let _ = layouts::save_config(&self.config);
            }
        });
    }

    fn toggle_panel(&mut self, tab: PanelTab) {
        let dock = &mut self.active_project().dock_state;
        if let Some(loc) = dock.find_tab(&tab) {
            dock.remove_tab(loc);
        } else {
            dock.push_to_focused_leaf(tab);
        }
    }

    fn layout_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Layout", |ui| {
            if ui.button("Save Layout…").clicked() {
                self.show_save_layout = true;
                self.save_layout_name.clear();
                ui.close_menu();
            }

            ui.menu_button("Load Layout", |ui| {
                for name in layouts::list_layouts() {
                    if ui.button(&name).clicked() {
                        self.apply_layout(&name);
                        ui.close_menu();
                    }
                }
            });

            ui.menu_button("Set Initializing Layout", |ui| {
                let current = self.config.default_layout.clone();
                for name in layouts::list_layouts() {
                    let selected = name.eq_ignore_ascii_case(&current);
                    if ui.selectable_label(selected, &name).clicked() {
                        self.set_default_layout(&name);
                        ui.close_menu();
                    }
                }
            });

            let user_layouts: Vec<String> = layouts::list_layouts()
                .into_iter()
                .filter(|n| !n.eq_ignore_ascii_case(layouts::DEFAULT_LAYOUT))
                .collect();
            ui.add_enabled_ui(!user_layouts.is_empty(), |ui| {
                ui.menu_button("Delete Layout", |ui| {
                    for name in &user_layouts {
                        if ui.button(name).clicked() {
                            self.delete_layout(name);
                            ui.close_menu();
                        }
                    }
                });
            });
        });
    }

    fn project_tabs(&mut self, ui: &mut egui::Ui) {
        let mut select: Option<usize> = None;
        let mut close: Option<usize> = None;

        let multiple = self.projects.len() > 1;
        for (i, project) in self.projects.iter().enumerate() {
            let selected = i == self.active;

            let title = if project.modified {
                format!("{} *", project.name)
            } else {
                project.name.clone()
            };
            if ui.selectable_label(selected, title).clicked() {
                select = Some(i);
            }
            if multiple && ui.small_button("x").clicked() {
                close = Some(i);
            }
            ui.separator();
        }

        if ui.button("+").on_hover_text("New project").clicked() {
            self.add_project();
        }

        if let Some(i) = select {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_project(i);
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {

        if self.about_logo.is_none() {
            if let Ok(img) = image::load_from_memory(ABOUT_LOGO_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.about_logo =
                    Some(ctx.load_texture("about_logo", color, egui::TextureOptions::LINEAR));
            }
        }

        let logo = self.about_logo.clone();
        let mut open = self.show_about;
        egui::Window::new("About")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    if let Some(tex) = &logo {
                        let native = tex.size_vec2();
                        let scale = (160.0 / native.x.max(1.0)).min(1.0);
                        ui.image(egui::load::SizedTexture::new(tex.id(), native * scale));
                    }
                    ui.add_space(10.0);
                    ui.heading("Rick's Texture Ripper");

                    let (link, year, gap) = ("l30z", "- 2026", 4.0);
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let measure = |ui: &egui::Ui, s: &str| {
                        ui.fonts(|f| {
                            f.layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
                                .size()
                                .x
                        })
                    };
                    let row_w = measure(ui, link) + gap + measure(ui, year);
                    ui.allocate_ui_with_layout(
                        egui::vec2(row_w, font.size + 4.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = gap;

                            let text = egui::RichText::new(link)
                                .color(egui::Color32::WHITE)
                                .underline();
                            let resp = ui
                                .add(egui::Label::new(text).sense(egui::Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                ui.ctx().open_url(egui::OpenUrl::same_tab(
                                    "https://github.com/L30ZMine",
                                ));
                            }
                            ui.label(year);
                        },
                    );
                    ui.add_space(16.0);
                    ui.label("Version 1.3.0");
                    ui.weak(format!("Built {}", env!("BUILD_DATE")));
                    ui.add_space(4.0);
                });
            });
        self.show_about = open;
    }

    fn poll_update_check(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.update_rx else { return };
        match rx.try_recv() {
            Ok(outcome) => {
                let v = env!("CARGO_PKG_VERSION");
                match outcome {
                    crate::update::Outcome::UpToDate => {
                        self.set_status(format!("Up to date (v{v})."))
                    }
                    crate::update::Outcome::Available(tag) => self.set_status(format!(
                        "Update available: {tag} (you have v{v}) — github.com/l30zmine/ricks-textureripper/releases"
                    )),
                    crate::update::Outcome::Failed => {
                        self.set_status("Couldn't check for updates.")
                    }
                }
                self.update_rx = None;
            }

            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(400));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_rx = None,
        }
    }

    fn first_run_window(&mut self, ctx: &egui::Context) {
        if !self.show_first_run {
            return;
        }
        let mut finish = false;
        let mut open = true;
        egui::Window::new("Setup")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Where should preferences and layouts be stored?");
                ui.add_space(4.0);
                ui.radio_value(&mut self.first_run_portable, false, "Documents (recommended)");
                if let Some(d) = layouts::documents_dir() {
                    ui.weak(format!("    {}", d.display()));
                }
                ui.radio_value(
                    &mut self.first_run_portable,
                    true,
                    "Next to the program (portable)",
                );
                if let Some(d) = layouts::portable_dir() {
                    ui.weak(format!("    {}", d.display()));
                }

                #[cfg(windows)]
                {
                    ui.add_space(10.0);

                    if cfg!(debug_assertions) {
                        ui.weak("Install to Program Files is disabled in debug builds.");
                    } else {
                        ui.checkbox(
                            &mut self.first_run_install,
                            "Install to Program Files and add a Start Menu shortcut",
                        );
                        ui.weak("    Asks for administrator approval (UAC); the app reopens from there.");
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Continue").clicked() {
                        finish = true;
                    }
                });
            });
        if finish {
            self.finish_first_run(ctx);
        } else if !open {

            self.show_first_run = false;
        }
    }

    fn finish_first_run(&mut self, ctx: &egui::Context) {

        let dir = if self.first_run_portable {
            layouts::portable_dir()
        } else {
            layouts::documents_dir()
        };
        if let Some(d) = dir {
            layouts::set_app_dir(d);
        }

        if let Err(e) = layouts::save_config(&self.config) {
            self.set_error(format!("Couldn't save preferences: {e}"));
        }
        self.show_first_run = false;

        if self.first_run_install && !cfg!(debug_assertions) {
            match crate::install::install_to_program_files() {
                Ok(()) => {

                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => self.set_error(format!("Install failed: {e}")),
            }
        }
    }

    fn has_unsaved(&self) -> bool {
        self.projects.iter().any(|p| p.modified)
    }

    fn save_all_for_quit(&mut self) -> bool {
        for i in 0..self.projects.len() {
            if self.projects[i].modified {
                self.active = i;
                self.save_project();
                if self.projects[i].modified {
                    return false;
                }
            }
        }
        true
    }

    fn quit_guard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.allow_close && self.has_unsaved() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_quit = true;
            } else if !self.recovery_pending {

                crate::autosave::mark_clean_shutdown();
            }
        }

        if !self.confirm_quit {
            return;
        }

        let mut close_now = false;
        let unsaved = self.projects.iter().filter(|p| p.modified).count();
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "{unsaved} project(s) have unsaved changes. Quit anyway?"
                ));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save all & quit").clicked() && self.save_all_for_quit() {
                        close_now = true;
                        self.confirm_quit = false;
                    }
                    if ui.button("Discard & quit").clicked() {
                        close_now = true;
                        self.confirm_quit = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_quit = false;
                    }
                });
            });

        if close_now {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn recover_window(&mut self, ctx: &egui::Context) {
        if !self.show_recover {
            return;
        }
        let mut recover = false;
        let mut dismiss = false;
        egui::Window::new("Recover unsaved work")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "The app didn't close properly last time. {} autosaved project(s) can be recovered:",
                    self.recover_entries.len()
                ));
                ui.add_space(4.0);
                for entry in &self.recover_entries {
                    ui.weak(format!("• {}", entry.name));
                }
                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Recover all").clicked() {
                        recover = true;
                    }
                    if ui.button("Ignore").clicked() {
                        dismiss = true;
                    }
                });
            });

        if recover {

            let paths: Vec<std::path::PathBuf> =
                self.recover_entries.iter().map(|e| e.path.clone()).collect();
            let mut opened = 0;
            for path in &paths {
                match crate::autosave::open_recovered(ctx, path) {
                    Ok(project) => {

                        if opened == 0
                            && self.projects.len() == 1
                            && self.projects[0].images.is_empty()
                            && !self.projects[0].modified
                        {
                            self.projects[0] = project;
                        } else {
                            self.projects.push(project);
                        }
                        opened += 1;
                    }
                    Err(e) => self.set_error(format!("Recover failed: {e}")),
                }
            }
            if opened > 0 {
                self.active = self.projects.len() - 1;
                self.set_status(format!("Recovered {opened} project(s)."));
            }
            self.show_recover = false;

            self.recovery_pending = false;
        } else if dismiss {
            self.show_recover = false;
            self.recovery_pending = false;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dnd_overlay"),
            ));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop images to add",
                egui::FontId::proportional(28.0),
                egui::Color32::WHITE,
            );
        }

        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let project = &mut self.projects[self.active];
        for file in dropped {
            if let Some(path) = file.path {
                if crate::texture_view::is_supported_image(&path) {
                    crate::texture_view::add_image_path(ctx, project, &path);
                }
            }
        }
    }

    fn info_window(&mut self, ctx: &egui::Context) {

        if self.show_first_run {
            return;
        }

        if self.info_logo.is_none() {
            if let Ok(img) = image::load_from_memory(INFO_LOGO_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.info_logo =
                    Some(ctx.load_texture("info_logo", color, egui::TextureOptions::LINEAR));
            }
        }

        let logo = self.info_logo.clone();
        let was_open = self.show_info;
        let mut open = self.show_info;
        egui::Window::new("Info")
            .open(&mut open)
            .collapsible(true)
            .resizable(true)
            .default_size([460.0, 520.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    render_markdown(ui, INFO_MARKDOWN, logo.as_ref());
                });
            });
        self.show_info = open;

        if was_open && !open && self.config.show_info_on_startup {
            self.config.show_info_on_startup = false;
            let _ = layouts::save_config(&self.config);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(6.0);

                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} v{} ({})",
                                env!("CARGO_PKG_NAME"),
                                env!("CARGO_PKG_VERSION"),
                                if cfg!(debug_assertions) { "debug" } else { "release" },
                            ))
                            .weak(),
                        )
                        .selectable(false),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let project = &mut self.projects[self.active];
                        if let Some(status) = project.status.clone() {
                            if project.status_error {

                                egui::Frame::new()
                                    .fill(egui::Color32::from_rgb(92, 34, 34))
                                    .corner_radius(5.0)
                                    .inner_margin(egui::Margin::symmetric(8, 1))
                                    .show(ui, |ui| {
                                        if ui.small_button("x").on_hover_text("Dismiss").clicked() {
                                            project.status = None;
                                        }
                                        ui.add(
                                            egui::Label::new(egui::RichText::new(status).color(
                                                egui::Color32::from_rgb(255, 205, 205),
                                            ))
                                            .selectable(true),
                                        );
                                    });
                            } else {
                                if ui.small_button("x").on_hover_text("Dismiss").clicked() {
                                    project.status = None;
                                }
                                ui.label(status);
                            }
                        }
                    });
                });
            });
    }

    fn save_layout_window(&mut self, ctx: &egui::Context) {
        if !self.show_save_layout {
            return;
        }
        let mut open = true;
        let mut do_save = false;
        egui::Window::new("Save Layout")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Layout name:");
                let resp = ui.text_edit_singleline(&mut self.save_layout_name);
                let reserved = self
                    .save_layout_name
                    .trim()
                    .eq_ignore_ascii_case(layouts::DEFAULT_LAYOUT);
                if reserved {
                    ui.colored_label(egui::Color32::LIGHT_RED, "\"default\" is reserved.");
                }
                ui.horizontal(|ui| {
                    let can_save = !self.save_layout_name.trim().is_empty() && !reserved;
                    if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                        do_save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_save_layout = false;
                    }
                });
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_save = true;
                }
            });
        if !open {
            self.show_save_layout = false;
        }
        if do_save {
            self.attempt_save_layout();
        }
    }

    fn attempt_save_layout(&mut self) {
        let name = self.save_layout_name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if self.config.confirm_layout_overwrite && layouts::layout_exists(&name) {
            self.overwrite_dont_ask = false;
            self.show_overwrite_confirm = true;
        } else {
            self.save_current_layout();
        }
    }

    fn overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        if !self.show_overwrite_confirm {
            return;
        }
        let name = self.save_layout_name.trim().to_string();
        let mut open = true;
        let mut confirm = false;
        egui::Window::new("Overwrite layout?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("A layout named \"{name}\" already exists. Overwrite it?"));
                ui.add_space(4.0);
                ui.checkbox(&mut self.overwrite_dont_ask, "Don't ask again");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite").clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_overwrite_confirm = false;
                    }
                });
            });
        if !open {
            self.show_overwrite_confirm = false;
        }
        if confirm {

            if self.overwrite_dont_ask {
                self.config.confirm_layout_overwrite = false;
                if let Err(e) = layouts::save_config(&self.config) {
                    self.set_error(format!("Could not save config: {e}"));
                }
            }
            self.show_overwrite_confirm = false;
            self.save_current_layout();
        }
    }
}

fn render_markdown(ui: &mut egui::Ui, md: &str, logo: Option<&egui::TextureHandle>) {
    for raw in md.lines() {
        let line = raw.trim_end();
        if let Some(alt) = parse_image(line) {
            match logo {
                Some(tex) => {

                    let native = tex.size_vec2();
                    let scale = (ui.available_width() / native.x.max(1.0)).min(1.0);
                    ui.add_space(2.0);
                    ui.image(egui::load::SizedTexture::new(tex.id(), native * scale));
                    ui.add_space(4.0);
                }
                None => {
                    ui.label(egui::RichText::new(alt).strong());
                }
            }
        } else if let Some(rest) = line.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(rest).strong().size(15.0));
        } else if let Some(rest) = line.strip_prefix("## ") {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(rest).strong().size(17.0));
        } else if let Some(rest) = line.strip_prefix("# ") {
            ui.heading(rest);
        } else if let Some(rest) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
        {
            ui.horizontal_wrapped(|ui| render_inline(ui, &format!("\u{2022}  {rest}")));
        } else if line.is_empty() {
            ui.add_space(6.0);
        } else {
            ui.horizontal_wrapped(|ui| render_inline(ui, line));
        }
    }
}

fn parse_image(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("![")?;
    let close = rest.find("](")?;

    if rest[close..].ends_with(')') {
        Some(&rest[..close])
    } else {
        None
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

fn render_inline(ui: &mut egui::Ui, text: &str) {
    ui.spacing_mut().item_spacing.x = 0.0;
    let mut bold = false;
    for seg in text.split("**") {
        if !seg.is_empty() {
            let rt = egui::RichText::new(seg);
            ui.label(if bold { rt.strong() } else { rt });
        }
        bold = !bold;
    }
}
