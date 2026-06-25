//! Top-level application: window chrome (menu bar + project tab bar) and the
//! dockable workspace. Each project owns its own dock layout.

use std::path::PathBuf;

use egui_dock::{DockArea, DockState, Style};

use crate::layouts::{self, Config};
use crate::project::Project;
use crate::ui::docking::{DockViewer, PanelTab};

pub struct App {
    /// Each Photoshop-style tab is an independent project (with its own layout).
    projects: Vec<Project>,
    /// Index into `projects` of the active tab.
    active: usize,
    /// Monotonic counter for default project names.
    next_project_id: usize,
    /// Persistent app config (incl. the default layout for new projects).
    config: Config,
    /// Set true to show the About window.
    show_about: bool,
    /// Set true to show the Info / controls window (opens on startup).
    show_info: bool,
    /// Lazily-loaded logo texture shown in the Info window.
    info_logo: Option<egui::TextureHandle>,
    /// Lazily-loaded logo texture shown in the About window.
    about_logo: Option<egui::TextureHandle>,
    /// Lazily-loaded small `_g` logo shown (faint, non-clickable) left of the menus.
    menu_logo: Option<egui::TextureHandle>,
    /// A `.rtrpf` path from the command line, opened on the first frame.
    pending_open: Option<PathBuf>,
    /// Save-layout dialog state.
    show_save_layout: bool,
    save_layout_name: String,
}

/// The controls / quick-help text shown in the Info window. Edit `src/info.md`
/// to change it (embedded at build time).
const INFO_MARKDOWN: &str = include_str!("info.md");

/// The banner logo shown at the top of the Info window (referenced from
/// `info.md` as `logo_long_g.png`).
const INFO_LOGO_PNG: &[u8] = include_bytes!("logo_long_g.png");

/// The square brand logo: shown (centered) in the About window and (faintly) as
/// the menu-bar logo. The neutral `_g` variant reads on light or dark.
const ABOUT_LOGO_PNG: &[u8] = include_bytes!("logo_g.png");

impl App {
    pub fn new(startup_open: Option<PathBuf>) -> Self {
        let config = layouts::load_config();
        let dock = layouts::load_layout(&config.default_layout)
            .unwrap_or_else(|_| layouts::builtin_default());
        let show_info = config.show_info_on_startup;
        Self {
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
        }
    }

    /// The dock layout a new project should start from (the configured default,
    /// falling back to the built-in if it can't be loaded).
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
            return; // always keep at least one project open
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

    // --- Layout actions -----------------------------------------------------

    fn apply_layout(&mut self, name: &str) {
        match layouts::load_layout(name) {
            Ok(dock) => {
                self.active_project().dock_state = dock;
                self.set_status(format!("Loaded layout \"{name}\"."));
            }
            Err(e) => self.set_error(format!("Load layout failed: {e}")),
        }
    }

    // --- Undo / redo --------------------------------------------------------

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

    // --- Project file save / open -------------------------------------------

    /// "Save": overwrite the project's existing file if it has one, else fall
    /// back to "Save As".
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

    /// "Save As": always prompt for a path, then remember it for later "Save".
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
                    // Saving names the project after the file and clears the `*`.
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

    /// Records `path` in the recent-files list and persists the config.
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
            // Defer the load by one frame so the wait cursor (set in `update`) is
            // shown while the project decodes.
            self.pending_open = Some(path);
        }
    }

    /// Opens the project at `path` into a new tab (used by the Open dialog, the
    /// command line, and double-clicked `.rtrpf` files).
    fn open_project_path(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        match crate::proj_io::open(ctx, path) {
            Ok(mut project) => {
                project.path = Some(path.to_path_buf());
                // Reuse the initial pristine "unnamed" tab if it's untouched.
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

    /// Picking a custom default also spins up a fresh, editable project that
    /// starts from it (the built-in "default" stays read-only).
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
        // Apply the chosen theme each frame (cheap; egui caches it). The dock and
        // canvas backgrounds read `dark_mode` from these visuals.
        ctx.set_visuals(if self.config.light_mode {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        });

        // Open a project passed on the command line / by double-click, once we
        // have a context to upload its textures with.
        if let Some(path) = self.pending_open.take() {
            self.open_project_path(ctx, &path);
        }

        self.handle_shortcuts(ctx);
        self.menu_bar(ctx);
        self.about_window(ctx);
        self.info_window(ctx);
        self.save_layout_window(ctx);

        // While the pointer is held (dragging a handle/slider) extract rips at a
        // cheap preview resolution; once the user settles, rerun at full quality.
        let busy = ctx.input(|i| i.pointer.any_down());
        let project = &mut self.projects[self.active];
        // Heavy CPU work this frame (full-res rip recompute / dirty images), used
        // below to show a background-progress cursor.
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

        // Commit an undo step once the user has settled (not mid-drag).
        if !busy {
            project.commit_history_if_changed();
        }

        // Permanent bottom chin bar (build info + transient status). Added before
        // the central dock area so it reserves its strip at the bottom.
        self.status_bar(ctx);

        // Render the dock against the active project. The dock state lives inside
        // the project, so temporarily swap it out to avoid aliasing the project
        // reference held by the viewer, then put it back.
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

        // Reflect work in the OS cursor (set last so it wins over panel cursors).
        // A wait cursor while a project is loading; a background-progress cursor
        // while rips/images are (re)computing or the app is otherwise lagging.
        if self.pending_open.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
            ctx.request_repaint();
        } else if recomputing && !busy {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
            ctx.request_repaint();
        }
    }
}

impl App {
    /// Global keyboard shortcuts. `consume_key` matches *logically* (it ignores
    /// extra modifiers), so the Shift variants are checked first and consume the
    /// event before the plain ones can match.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let alt = Modifiers::ALT;

        // Alt+1..4 toggle the workspace panels.
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

        // Redo before undo so Ctrl+Shift+Z isn't swallowed by the plain Ctrl+Z.
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::Y))
            || ctx.input_mut(|i| i.consume_key(ctrl_shift, Key::Z))
        {
            self.redo(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::Z)) {
            self.undo(ctx);
        }

        // Save As before Save for the same reason.
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

        // Delete / Backspace removes the selected rip, else the active image.
        // Guarded so it never fires while typing into a text field / drag value.
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

    /// Removes the selected rip if one is selected, otherwise the active image.
    /// Shared by the Delete/Backspace shortcut.
    fn delete_selection(&mut self) {
        let project = &mut self.projects[self.active];
        if let Some(sel) = project.editor.selected.filter(|&s| s < project.rips.len()) {
            crate::texture_view::remove_rip(project, sel);
        } else if let Some(idx) = project.active_image.filter(|&i| i < project.images.len()) {
            crate::texture_view::remove_image(project, idx);
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        // Decode the small `_g` menu-bar logo once.
        if self.menu_logo.is_none() {
            if let Ok(img) = image::load_from_memory(ABOUT_LOGO_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.menu_logo =
                    Some(ctx.load_texture("menu_logo", color, egui::TextureOptions::LINEAR));
            }
        }
        // Cloned out so the panel closure can still borrow `self` mutably. The logo
        // is fainter in light mode (35% transparent) than dark (80% transparent).
        let menu_logo = self.menu_logo.clone();
        let menu_logo_alpha: u8 = if self.config.light_mode { 166 } else { 51 };

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            // Lay the whole row out left-to-right, vertically centered, so the
            // menu text lines up with the taller framed project tabs.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(26.0);
                egui::menu::bar(ui, |ui| {
                // A small, faint, non-clickable logo flush to the left of the File
                // menu (fainter in light mode — see `menu_logo_alpha`).
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
                        // Force a sensible width band so the submenu neither
                        // collapses to tiny file-name buttons nor stretches to a
                        // full path; over-long names are cut with a trailing
                        // ellipsis (the full path stays in the hover tooltip).
                        ui.set_min_width(114.0);
                        ui.set_max_width(228.0);
                        if self.config.recent_files.is_empty() {
                            ui.add_enabled(false, egui::Button::new("(none)"));
                        } else {
                            // Clone so the loop can mutably borrow `self` to open.
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
                                    // Defer so the wait cursor shows while loading.
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
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });

                // Project tabs share the menu bar row: a margin, then the tabs
                // pushed to the right of the menus.
                ui.add_space(16.0);
                ui.separator();
                self.project_tabs(ui);
                });
            });
        });
    }

    /// The Window menu: toggle each workspace panel on/off in the active
    /// project's dock. A checkmark shows which panels are currently open.
    fn window_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Window", |ui| {
            // A leading check glyph marks open panels; the Alt+N shortcut is shown
            // greyed on the right via `shortcut_text`.
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

    /// Shows the panel if it's hidden, or removes it if it's currently docked.
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

    /// Photoshop-style project tabs, drawn inline in the menu-bar row.
    fn project_tabs(&mut self, ui: &mut egui::Ui) {
        let mut select: Option<usize> = None;
        let mut close: Option<usize> = None;

        let multiple = self.projects.len() > 1;
        for (i, project) in self.projects.iter().enumerate() {
            let selected = i == self.active;
            // Drawn inline (no group frame) so the tabs sit on the same vertical
            // centerline as the menu-bar text. A trailing `*` marks unsaved work.
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
        // Decode the logo once, on first show.
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
                    // Center the "l30z - 2026" row: a plain nested horizontal would
                    // span the full width and left-align, so measure the row and
                    // allocate exactly that width for `vertical_centered` to center.
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
                            // Custom link: always white + underlined, pointing-hand
                            // cursor on hover (egui's default link is blue and only
                            // underlines on hover).
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
                    ui.label("Version 1.2.0");
                    ui.weak(format!("Built {}", env!("BUILD_DATE")));
                    ui.add_space(4.0);
                });
            });
        self.show_about = open;
    }

    /// The Info / quick-controls window (opens on startup, reopenable via
    /// Help > Info). Content is the embedded `info.md`, lightly rendered.
    fn info_window(&mut self, ctx: &egui::Context) {
        // Decode the banner logo once, on first show.
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

        // The user closed the Info window: remember not to auto-open it next time.
        if was_open && !open && self.config.show_info_on_startup {
            self.config.show_info_on_startup = false;
            let _ = layouts::save_config(&self.config);
        }
    }

    /// Permanent full-width bottom bar: build info on the left, the active
    /// project's transient status (with a dismiss `x`) on the right when set.
    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    // Build info is informational only — not selectable for copy.
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

                    // Status + dismiss, pushed to the right edge.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let project = &mut self.projects[self.active];
                        if let Some(status) = project.status.clone() {
                            if project.status_error {
                                // Errors: soft dark-red rounded background covering
                                // the whole message, and selectable so it can be
                                // copied.
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
            self.save_current_layout();
        }
    }
}

/// Minimal Markdown renderer for the Info window: handles `![alt](img)` images
/// (drawn from `logo`, scaled to fit width), headings (`#`/`##`/`###`), `-`/`*`
/// bullets, blank-line spacing, and inline `**bold**`. Anything else renders as
/// a wrapped paragraph.
fn render_markdown(ui: &mut egui::Ui, md: &str, logo: Option<&egui::TextureHandle>) {
    for raw in md.lines() {
        let line = raw.trim_end();
        if let Some(alt) = parse_image(line) {
            match logo {
                Some(tex) => {
                    // Scale the banner down to fit the available width (never up).
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

/// Parses a Markdown image line `![alt](path)`, returning the alt text. The path
/// is ignored — the Info window only has one (the embedded banner logo).
fn parse_image(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("![")?;
    let close = rest.find("](")?;
    // Must be a well-formed `](...)` tail to count as an image line.
    if rest[close..].ends_with(')') {
        Some(&rest[..close])
    } else {
        None
    }
}

/// Truncates `s` to at most `max` characters, appending `…` when it had to cut
/// (so over-long recent-file names don't blow out the Open Recent submenu).
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// Renders a single line, turning `**bold**` spans strong. Item spacing is zeroed
/// so adjacent spans read as continuous text.
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
