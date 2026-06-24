//! Top-level application: window chrome (menu bar + project tab bar) and the
//! dockable workspace. Each project owns its own dock layout.

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
    /// Save-layout dialog state.
    show_save_layout: bool,
    save_layout_name: String,
}

/// The controls / quick-help text shown in the Info window. Edit `src/info.md`
/// to change it (embedded at build time).
const INFO_MARKDOWN: &str = include_str!("info.md");

/// The banner logo shown at the top of the Info window (referenced from
/// `info.md` as `logo_long_w.png`).
const INFO_LOGO_PNG: &[u8] = include_bytes!("logo_long_w.png");

impl App {
    pub fn new() -> Self {
        let config = layouts::load_config();
        let dock = layouts::load_layout(&config.default_layout)
            .unwrap_or_else(|_| layouts::builtin_default());
        Self {
            projects: vec![Project::new("unnamed", dock)],
            active: 0,
            next_project_id: 2,
            config,
            show_about: false,
            show_info: true,
            info_logo: None,
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
        self.active_project().status = Some(msg.into());
    }

    // --- Layout actions -----------------------------------------------------

    fn apply_layout(&mut self, name: &str) {
        match layouts::load_layout(name) {
            Ok(dock) => {
                self.active_project().dock_state = dock;
                self.set_status(format!("Loaded layout \"{name}\"."));
            }
            Err(e) => self.set_status(format!("Load layout failed: {e}")),
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

    fn save_project_dialog(&mut self) {
        let suggested = format!("{}.{}", self.active_project().name, crate::proj_io::EXTENSION);
        let picked = rfd::FileDialog::new()
            .set_title("Save Project")
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
                    self.set_status(format!("Saved {}", path.display()));
                }
                Err(e) => self.set_status(format!("Save failed: {e}")),
            }
        }
    }

    fn open_project_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .set_title("Open Project")
            .add_filter("Rick's Texture Ripper Project", &[crate::proj_io::EXTENSION])
            .pick_file();
        if let Some(path) = picked {
            match crate::proj_io::open(ctx, &path) {
                Ok(project) => {
                    self.projects.push(project);
                    self.active = self.projects.len() - 1;
                    self.set_status(format!("Opened {}", path.display()));
                }
                Err(e) => self.set_status(format!("Open failed: {e}")),
            }
        }
    }

    fn save_current_layout(&mut self) {
        let name = self.save_layout_name.trim().to_string();
        match layouts::save_layout(&name, &self.active_project().dock_state) {
            Ok(()) => {
                self.show_save_layout = false;
                self.set_status(format!("Saved layout \"{name}\"."));
            }
            Err(e) => self.set_status(format!("Save layout failed: {e}")),
        }
    }

    /// Picking a custom default also spins up a fresh, editable project that
    /// starts from it (the built-in "default" stays read-only).
    fn set_default_layout(&mut self, name: &str) {
        self.config.default_layout = name.to_string();
        if let Err(e) = layouts::save_config(&self.config) {
            self.set_status(format!("Could not save config: {e}"));
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
            Err(e) => self.set_status(format!("Delete failed: {e}")),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.menu_bar(ctx);
        self.about_window(ctx);
        self.info_window(ctx);
        self.save_layout_window(ctx);

        // While the pointer is held (dragging a handle/slider) extract rips at a
        // cheap preview resolution; once the user settles, rerun at full quality.
        let busy = ctx.input(|i| i.pointer.any_down());
        let project = &mut self.projects[self.active];
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
    }
}

impl App {
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            // Lay the whole row out left-to-right, vertically centered, so the
            // menu text lines up with the taller framed project tabs.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_height(26.0);
                egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Add Image").clicked() {
                        crate::texture_view::open_add_image_dialog(
                            ctx,
                            &mut self.projects[self.active],
                        );
                        ui.close_menu();
                    }
                    if ui.button("Add Rip").clicked() {
                        crate::rip_tool::add_rip(&mut self.projects[self.active]);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("New Project").clicked() {
                        self.add_project();
                        ui.close_menu();
                    }
                    if ui.button("Open…").clicked() {
                        self.open_project_dialog(ctx);
                        ui.close_menu();
                    }
                    if ui.button("Save As…").clicked() {
                        self.save_project_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export Atlas…").clicked() {
                        crate::atlas::export(&mut self.projects[self.active]);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    let can_undo = self.projects[self.active].history.can_undo();
                    let can_redo = self.projects[self.active].history.can_redo();
                    if ui.add_enabled(can_undo, egui::Button::new("Undo")).clicked() {
                        self.undo(ctx);
                        ui.close_menu();
                    }
                    if ui.add_enabled(can_redo, egui::Button::new("Redo")).clicked() {
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
                        ui.weak("Corner grab radius, edge dead-zone, and move inset.");
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
            for (tab, name) in [
                (PanelTab::Texture, "Texture View"),
                (PanelTab::Atlas, "Atlas View"),
                (PanelTab::ImageEdit, "Image Edit"),
                (PanelTab::Rips, "Rips Gallery"),
            ] {
                let mut open = self.active_project().dock_state.find_tab(&tab).is_some();
                if ui.checkbox(&mut open, name).clicked() {
                    self.toggle_panel(tab);
                    ui.close_menu();
                }
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
        let mut open = self.show_about;
        egui::Window::new("About")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Rick's Texture Ripper");
                ui.label("A texture ripper / atlas tool.");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
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
    }

    /// Permanent full-width bottom bar: build info on the left, the active
    /// project's transient status (with a dismiss `x`) on the right when set.
    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(22.0)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    ui.weak(format!(
                        "{} v{} ({})",
                        env!("CARGO_PKG_NAME"),
                        env!("CARGO_PKG_VERSION"),
                        if cfg!(debug_assertions) { "debug" } else { "release" },
                    ));

                    // Status + dismiss, pushed to the right edge.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let project = &mut self.projects[self.active];
                        if let Some(status) = project.status.clone() {
                            if ui.small_button("x").on_hover_text("Dismiss").clicked() {
                                project.status = None;
                            }
                            ui.colored_label(egui::Color32::LIGHT_YELLOW, status);
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
