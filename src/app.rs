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
    /// A blocking save deferred one frame so the wait cursor shows.
    pending_op: Option<PendingOp>,
    /// Background image decoder (added images load off-thread with a % indicator).
    image_loader: crate::render::ImageLoader,
    /// Save-layout dialog state.
    show_save_layout: bool,
    save_layout_name: String,
    /// "Overwrite existing layout?" confirmation dialog state.
    show_overwrite_confirm: bool,
    /// The "Don't ask again" checkbox in the overwrite-confirmation dialog.
    overwrite_dont_ask: bool,
    /// First-run setup dialog (preferences location + optional install).
    show_first_run: bool,
    /// First-run choice: store preferences next to the exe (portable) vs Documents.
    first_run_portable: bool,
    /// First-run choice: install to Program Files + add a Start Menu shortcut.
    first_run_install: bool,
    /// True for the whole of a genuine first-launch session (no config existed at
    /// startup). Unlike `show_first_run` (which clears once the setup dialog is
    /// dismissed), this stays set all session. It gates the "closing the Info
    /// window turns off show-on-startup" rule to the first run only — on later runs
    /// the Preferences toggle is respected, so closing the window doesn't flip it.
    first_launch: bool,
    /// Receiver for the background update check spawned at startup.
    update_rx: Option<std::sync::mpsc::Receiver<crate::update::Outcome>>,
    /// Showing the "unsaved changes" confirmation before quitting.
    confirm_quit: bool,
    /// Set once the user has confirmed quitting, so the next close isn't vetoed.
    allow_close: bool,
    /// `ctx.input().time` of the last autosave, for the ~30s cadence.
    last_autosave: f64,
    /// Crash-recovery: autosaves offered after an unclean shutdown, and whether
    /// the recovery prompt is showing.
    recover_entries: Vec<crate::autosave::Recoverable>,
    show_recover: bool,
    /// True while a crash-recovery offer hasn't been acted on yet. It keeps the
    /// session lock alive across a clean shutdown so the offer reappears on the
    /// next launch — the crash flag is only cleared once the user dismisses (or
    /// recovers from) the recover dialog.
    recovery_pending: bool,
    /// Storage-conflict chooser: shown when a config exists in *both* Documents and
    /// a portable folder, asking which to use (with each one's last-modified time).
    show_storage_choice: bool,
    /// Last-modified stamps (Unix millis, `0` = unknown) for the two competing
    /// config locations, shown in the chooser.
    storage_doc_ms: u64,
    storage_port_ms: u64,
    /// Chooser checkbox: also merge the *other* location's layouts into the chosen
    /// one and remove it, so the prompt doesn't recur. On by default.
    storage_consolidate: bool,
    /// The autosave session start is deferred while the storage chooser is up, so
    /// `session.lock` and crash-recovery are evaluated against the *chosen*
    /// location rather than the provisional default.
    session_pending: bool,
    /// Preferences window state.
    show_preferences: bool,
    prefs_category: PrefsCategory,
    /// True while the interface-zoom slider is being dragged, so the zoom is held
    /// (locked) until the grab is released.
    zoom_slider_active: bool,
    /// A preference changed and needs persisting — deferred until the pointer is
    /// released, so dragging a slider doesn't rewrite the config every frame.
    prefs_dirty: bool,
    /// Pending "close this project with unsaved changes?" confirmation (project
    /// index), plus the dialog's "Don't ask again" checkbox.
    close_req: Option<usize>,
    close_dont_ask: bool,
    /// The "Don't ask again" checkbox in the quit (unsaved-changes) dialog.
    quit_dont_ask: bool,
}

/// Which category page the Blender-style Preferences window is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrefsCategory {
    General,
    Appearance,
    Editing,
    Confirmations,
}

/// A blocking save deferred by one frame so the wait cursor is visible while it
/// runs (the cursor is only applied at the end of a frame, so a synchronous op
/// started this frame can't show it). Carries the target project index, captured
/// at trigger time so switching tabs in between can't misdirect it. (Image adds
/// run on a real background thread instead — see `crate::render::ImageLoader`.)
enum PendingOp {
    /// Overwrite the project's existing file.
    Save(usize),
    /// Write the project to a chosen path (the name is already set).
    SaveAs(usize, PathBuf),
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
    pub fn new(startup_open: Option<PathBuf>, storage: layouts::Storage) -> Self {
        let config = layouts::load_config();
        let dock = layouts::load_layout(&config.default_layout)
            .unwrap_or_else(|_| layouts::builtin_default());
        let show_info = config.show_info_on_startup;
        let check_updates = config.check_updates;
        // Reopen the most-recently-used project on startup when asked (unless the
        // command line already named a file to open).
        let pending_open = startup_open.or_else(|| {
            if config.reopen_last {
                config.recent_files.first().cloned()
            } else {
                None
            }
        });
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
            pending_open,
            pending_op: None,
            image_loader: crate::render::ImageLoader::default(),
            show_save_layout: false,
            save_layout_name: String::new(),
            show_overwrite_confirm: false,
            overwrite_dont_ask: false,
            show_first_run: matches!(storage, layouts::Storage::FirstRun),
            first_launch: matches!(storage, layouts::Storage::FirstRun),
            first_run_portable: false,
            first_run_install: false,
            // Check for a newer release in the background (if enabled) and show
            // the result.
            update_rx: if check_updates {
                Some(crate::update::spawn_check())
            } else {
                None
            },
            confirm_quit: false,
            allow_close: false,
            last_autosave: 0.0,
            recover_entries: Vec::new(),
            show_recover: false,
            recovery_pending: false,
            show_storage_choice: false,
            storage_doc_ms: 0,
            storage_port_ms: 0,
            storage_consolidate: true,
            session_pending: false,
            show_preferences: false,
            prefs_category: PrefsCategory::General,
            zoom_slider_active: false,
            prefs_dirty: false,
            close_req: None,
            close_dont_ask: false,
            quit_dont_ask: false,
        };
        if let layouts::Storage::Conflict { doc_ms, port_ms } = storage {
            // Two config locations — ask which to use *before* starting the autosave
            // session, so `session.lock`/recovery bind to the chosen location.
            app.show_storage_choice = true;
            app.storage_doc_ms = doc_ms;
            app.storage_port_ms = port_ms;
            app.session_pending = true;
        } else {
            app.begin_session();
        }
        if check_updates {
            app.projects[0].set_status("Searching for updates…");
        }
        app
    }

    /// Starts the autosave session for the active storage location and surfaces an
    /// unclean-shutdown recovery offer. Deferred past the storage chooser so the
    /// session binds to whichever location the user picks.
    fn begin_session(&mut self) {
        let (crashed, recoverable) = crate::autosave::start_session();
        self.recover_entries = recoverable;
        self.show_recover = crashed && !self.recover_entries.is_empty();
        // The offer is "pending" until the user acts on it, so a clean shutdown in
        // the meantime won't silently drop the chance to recover.
        self.recovery_pending = self.show_recover;
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

    /// "Save" trigger: overwrite the project's existing file if it has one (the
    /// write is deferred one frame so the wait cursor shows), else fall back to
    /// "Save As".
    fn save_project(&mut self) {
        if self.active_project().path.is_some() {
            self.pending_op = Some(PendingOp::Save(self.active));
        } else {
            self.save_project_dialog();
        }
    }

    /// "Save As" trigger: prompt for a path now, then defer the write one frame.
    fn save_project_dialog(&mut self) {
        let idx = self.active;
        if let Some(path) = self.save_as_dialog_path(idx) {
            self.pending_op = Some(PendingOp::SaveAs(idx, path));
        }
    }

    /// Shows the "Save Project As" dialog for project `idx` and, if a path is
    /// chosen, names the project after the file stem (so the persisted name matches
    /// the file, not a stale "unnamed") and returns the path.
    fn save_as_dialog_path(&mut self, idx: usize) -> Option<PathBuf> {
        let suggested = format!("{}.{}", self.projects[idx].name, crate::proj_io::EXTENSION);
        let path = rfd::FileDialog::new()
            .set_title("Save Project As")
            .add_filter("Rick's Texture Ripper Project", &[crate::proj_io::EXTENSION])
            .set_file_name(suggested)
            .save_file()?;
        if let Some(stem) = path.file_stem() {
            self.projects[idx].name = stem.to_string_lossy().into_owned();
        }
        Some(path)
    }

    /// Synchronously writes project `idx` to its existing path. Returns true on
    /// success (so callers like quit / close-confirm can know it actually saved).
    fn do_save(&mut self, idx: usize) -> bool {
        let Some(path) = self.projects.get(idx).and_then(|p| p.path.clone()) else {
            return false;
        };
        match crate::proj_io::save(&path, &self.projects[idx]) {
            Ok(()) => {
                self.projects[idx].modified = false;
                // The work is now persisted, so its autosaves aren't needed for
                // recovery anymore — drop them so a later crash doesn't offer to
                // "recover" an already-saved project.
                crate::autosave::clear_project(self.projects[idx].id);
                self.projects[idx].set_status(format!("Saved {}", path.display()));
                true
            }
            Err(e) => {
                self.projects[idx].set_error(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Synchronously writes project `idx` to `path` (the name is already set),
    /// recording it as the project's path + a recent file. Returns true on success.
    fn do_save_as(&mut self, idx: usize, path: PathBuf) -> bool {
        match crate::proj_io::save(&path, &self.projects[idx]) {
            Ok(()) => {
                self.projects[idx].modified = false;
                self.projects[idx].path = Some(path.clone());
                crate::autosave::clear_project(self.projects[idx].id);
                self.remember_recent(&path);
                self.projects[idx].set_status(format!("Saved {}", path.display()));
                true
            }
            Err(e) => {
                self.projects[idx].set_error(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Saves project `idx` synchronously, prompting for a path if it doesn't have
    /// one yet. Returns true once it's actually saved (false if cancelled/failed).
    /// Used by the quit guard and the close-confirm dialog, which must finish the
    /// save before closing.
    fn save_now(&mut self, idx: usize) -> bool {
        if self.projects[idx].path.is_some() {
            self.do_save(idx)
        } else if let Some(path) = self.save_as_dialog_path(idx) {
            self.do_save_as(idx, path)
        } else {
            false
        }
    }

    /// Runs a deferred blocking save. Called one frame after it was triggered,
    /// while the wait cursor is showing.
    fn perform_pending_op(&mut self, op: PendingOp) {
        match op {
            PendingOp::Save(idx) => {
                self.do_save(idx);
            }
            PendingOp::SaveAs(idx, path) => {
                self.do_save_as(idx, path);
            }
        }
    }

    /// "Add Image" trigger: prompt for image files, then decode them on a
    /// background thread (the app stays responsive; a % shows by the cursor).
    fn add_image_dialog(&mut self) {
        if let Some(paths) = crate::texture_view::pick_image_files() {
            self.image_loader.start(self.active, paths);
        }
    }

    /// Applies any images the background loader has finished decoding, uploading
    /// each as a texture (the only UI-thread cost) and adding it to its project.
    fn poll_image_loader(&mut self, ctx: &egui::Context) {
        let target = self.image_loader.target;
        for msg in self.image_loader.poll() {
            match msg {
                crate::render::ImageLoadMsg::Loaded(data) => {
                    if target >= self.projects.len() {
                        continue;
                    }
                    let crate::render::LoadedImageData {
                        name,
                        source_path,
                        original,
                        pixels,
                        mips,
                    } = *data;
                    let offset = self.projects[target].images.len() as f32 * 32.0;
                    let mut img = crate::texture_view::assemble_loaded_image(
                        ctx, name, source_path, original, pixels, mips,
                    );
                    img.pos = egui::Vec2::new(offset, offset);
                    let name = img.name.clone();
                    let idx = self.projects[target].images.len();
                    self.projects[target].images.push(img);
                    self.projects[target].active_image = Some(idx);
                    self.projects[target].modified = true;
                    self.projects[target].set_status(format!("Loaded {name}"));
                }
                crate::render::ImageLoadMsg::Failed(e) => {
                    if target < self.projects.len() {
                        self.projects[target].set_error(e);
                    }
                }
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
                // The tab title always reflects the file it lives in, so an old
                // project saved with a stale "unnamed" name still shows correctly.
                if let Some(stem) = path.file_stem() {
                    project.name = stem.to_string_lossy().into_owned();
                }
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

        // Global interface scale (Preferences > Appearance). Held while the zoom
        // slider is being dragged (it applies on release), and only set when it
        // actually changed so egui doesn't discard the frame every time.
        let zoom = self.config.ui_zoom.clamp(0.4, 2.0);
        if !self.zoom_slider_active && (ctx.zoom_factor() - zoom).abs() > f32::EPSILON {
            ctx.set_zoom_factor(zoom);
        }

        // Preview quality and the handle grab margin are global preferences, so
        // mirror them into every project each frame (there's no per-project editor
        // for them anymore — they live in Preferences > Editing).
        let (cursor_margin, preview_quality) =
            (self.config.cursor_margin, self.config.preview_quality);
        for p in &mut self.projects {
            p.cursor_margin = cursor_margin;
            p.preview_quality = preview_quality;
        }

        // Surface the background update-check result in the status bar.
        self.poll_update_check(ctx);

        // Open a project passed on the command line / by double-click, once we
        // have a context to upload its textures with.
        if let Some(path) = self.pending_open.take() {
            self.open_project_path(ctx, &path);
        }

        // Run a deferred blocking save — deferred one frame from its trigger so the
        // wait cursor (set at the end of last frame) is showing.
        if let Some(op) = self.pending_op.take() {
            self.perform_pending_op(op);
        }
        // Apply any images the background loader finished decoding.
        self.poll_image_loader(ctx);
        // The Texture View's "Add Image" button can't reach `self`, so it sets a
        // flag; pick it up here and show the dialog + background load.
        if std::mem::take(&mut self.projects[self.active].want_add_image) {
            self.add_image_dialog();
        }

        // Drag-and-drop: dropping image files anywhere on the window adds them.
        self.handle_dropped_files(ctx);

        self.handle_shortcuts(ctx);
        self.menu_bar(ctx);
        self.about_window(ctx);
        self.info_window(ctx);
        self.preferences_window(ctx);
        self.save_layout_window(ctx);
        self.overwrite_confirm_window(ctx);
        self.confirm_close_window(ctx);
        self.first_run_window(ctx);
        self.storage_choice_window(ctx);
        self.recover_window(ctx);
        self.quit_guard(ctx);

        // Rip rendering: dirty rips always get a *cheap live preview* immediately;
        // the crisp full-resolution version is rendered on a background thread once
        // the user settles, so letting go of a handle no longer freezes the app.
        let busy = ctx.input(|i| i.pointer.any_down());
        let project = &mut self.projects[self.active];
        // Synchronous work this frame (dirty image colour/filter rebuild, or a rip
        // preview about to run) drives the wait cursor; the *background* full-res
        // render deliberately does not (the whole point is to stay responsive).
        let recomputing =
            project.rips.iter().any(|r| r.dirty) || project.images.iter().any(|i| i.dirty);
        // A geometry drag (canvas handles) previews at the user's Preview Quality
        // since the perspective warp is heavy; appearance-only edits aren't dragging
        // geometry, and recolour/filter is far cheaper, so they preview crisper.
        let geometry_drag = project.editor.is_dragging()
            || project.view.dragging_image.is_some()
            || project.view.scaling_image.is_some();
        let preview_scale = if geometry_drag {
            project.preview_quality
        } else {
            crate::rip_tool::edit_preview_scale(project.preview_quality)
        };
        crate::image_edit::recompute_dirty_images(ctx, project);
        let rips_changed = crate::rip_tool::recompute_dirty(ctx, project, Some(preview_scale));
        if busy {
            // Interacting: cancel any in-flight full-res render and keep previewing.
            project.renderer.cancel();
        } else {
            // Settled: (re)start a background full-res render for rips still showing
            // a preview. Restart when a rip changed this frame (so a stale in-flight
            // result can't overwrite the new preview), or when nothing is rendering
            // yet; otherwise let the running job finish. Then apply finished results.
            if project.rips.iter().any(|r| r.previewed)
                && (rips_changed || !project.renderer.is_active())
            {
                start_rip_render(project);
            }
            apply_rip_render_results(ctx, project);
        }
        let rendering = project.renderer.is_active();
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

        // Reflect work in the OS cursor with a single wait cursor (set last so it
        // wins over panel cursors): while a project loads / saves, or while
        // rips/images are (re)computing synchronously and the user isn't mid-drag.
        if self.pending_open.is_some() || self.pending_op.is_some() || (recomputing && !busy) {
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
            ctx.request_repaint();
        }

        // Background image loading: show a small grey % by the cursor, and keep
        // ticking so results are applied promptly. (No wait cursor — it's async.)
        if self.image_loader.active() {
            let pct = (self.image_loader.progress() * 100.0).round() as u32;
            if let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("img_load_pct"),
                ));
                painter.text(
                    pos + egui::vec2(18.0, 6.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{pct}%"),
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_gray(160),
                );
            }
            ctx.request_repaint();
        }
        // Keep ticking while a background full-res rip render is in flight, so its
        // results are picked up and drawn without waiting for another event.
        if rendering {
            ctx.request_repaint();
        }

        // Periodic autosave of modified projects (writes happen off-thread). The
        // timer wake keeps it firing even while the app is idle. The cadence is a
        // preference; `0` disables autosave entirely.
        let secs = self.config.autosave_secs;
        if secs > 0 {
            let now = ctx.input(|i| i.time);
            if now - self.last_autosave >= secs as f64 {
                self.last_autosave = now;
                crate::autosave::autosave_modified(&self.projects);
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(secs as u64));
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
            self.add_image_dialog();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::F)) {
            self.add_project();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::G)) {
            self.open_project_dialog();
        }
        if ctx.input_mut(|i| i.consume_key(ctrl, Key::I)) {
            self.show_info = !self.show_info;
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
                    // Never wrap menu item text onto a new line (e.g. at low UI
                    // zoom) — widen the menu to fit instead.
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui
                        .add(egui::Button::new("Add Image").shortcut_text("Ctrl+T"))
                        .clicked()
                    {
                        self.add_image_dialog();
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
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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
                    // Guide Lines / Cursor Interp / Preview Quality moved into the
                    // Blender-style Preferences window (Edit > Preferences).
                    if ui.button("Preferences…").clicked() {
                        self.show_preferences = true;
                        ui.close_menu();
                    }
                });

                self.layout_menu(ui);
                self.window_menu(ui);

                ui.menu_button("Help", |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui
                        .add(egui::Button::new("Info").shortcut_text("Ctrl+I"))
                        .clicked()
                    {
                        self.show_info = true;
                        ui.close_menu();
                    }
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Setup…").clicked() {
                        // Reopen the first-run dialog, reflecting the current
                        // storage choice.
                        self.first_run_portable = layouts::is_portable();
                        self.show_first_run = true;
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
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let panels = [
                (PanelTab::Texture, "Texture View", "Alt+1"),
                (PanelTab::Atlas, "Atlas View", "Alt+2"),
                (PanelTab::Rips, "Rips Gallery", "Alt+3"),
                (PanelTab::ImageEdit, "Image Edit", "Alt+4"),
            ];
            // Compute open-states before the grid closure so it doesn't capture
            // `self` (which the menu_button closure already borrows).
            let open_state: Vec<bool> = panels
                .iter()
                .map(|(tab, _, _)| self.active_project().dock_state.find_tab(tab).is_some())
                .collect();
            // A 2-column grid sizes naturally to its content (no forced width) and
            // keeps the panel checkbox in column 1, the Alt+N shortcut aligned in
            // column 2 — like the other menus' shortcut hints.
            let mut toggle: Option<PanelTab> = None;
            egui::Grid::new("window_panel_toggles")
                .num_columns(2)
                .show(ui, |ui| {
                    for (i, (tab, name, shortcut)) in panels.iter().enumerate() {
                        let mut open = open_state[i];
                        if ui.checkbox(&mut open, *name).changed() {
                            toggle = Some(*tab);
                        }
                        ui.weak(*shortcut);
                        ui.end_row();
                    }
                });
            if let Some(tab) = toggle {
                self.toggle_panel(tab);
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
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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

        // A little extra padding so the selected tab's blue highlight has some
        // breathing room; the close icon shares the same padding for a uniform look.
        ui.spacing_mut().button_padding = egui::vec2(8.0, 3.0);

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
            // A small × close icon (instead of the old "x" text), with matching
            // padding so it lines up with the tab. (× = U+00D7, which the default
            // font has — U+2715 ✕ renders as a missing-glyph box.) Muted when idle,
            // and high-contrast on hover (white on dark, black on light) — driven by
            // the widget visuals rather than an explicit colour, so hover works.
            if multiple {
                let clicked = ui
                    .scope(|ui| {
                        let weak = ui.visuals().weak_text_color();
                        let hover = if ui.visuals().dark_mode {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::BLACK
                        };
                        {
                            let w = &mut ui.visuals_mut().widgets;
                            w.inactive.fg_stroke.color = weak;
                            w.hovered.fg_stroke.color = hover;
                            w.active.fg_stroke.color = hover;
                        }
                        ui.add(egui::Button::new("×").frame(false))
                            .on_hover_text("Close project")
                    })
                    .inner
                    .clicked();
                if clicked {
                    close = Some(i);
                }
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
            self.request_close_project(i);
        }
    }

    /// Closes a project tab, first confirming if it has unsaved changes (unless
    /// the user opted out of that prompt).
    fn request_close_project(&mut self, index: usize) {
        if self.projects.len() <= 1 || index >= self.projects.len() {
            return;
        }
        if self.projects[index].modified && self.config.confirm_close_modified {
            self.close_req = Some(index);
            self.close_dont_ask = false;
        } else {
            self.close_project(index);
        }
    }

    /// The Blender-style Preferences window: a left category list with the
    /// selected category's settings on the right. Values persist to the config.
    fn preferences_window(&mut self, ctx: &egui::Context) {
        if !self.show_preferences {
            // Make sure a half-finished zoom drag doesn't leave the zoom locked,
            // and flush any preference change that hadn't been persisted yet.
            self.zoom_slider_active = false;
            if self.prefs_dirty {
                let _ = layouts::save_config(&self.config);
                self.prefs_dirty = false;
            }
            return;
        }
        let mut open = true;
        let mut dirty = false; // a config value changed this frame
        egui::Window::new("Preferences")
            .open(&mut open)
            .collapsible(false)
            // Locked to one size — never resizes (esp. not the width).
            .resizable(false)
            .fixed_size([580.0, 380.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    // --- Left: category list ---
                    ui.vertical(|ui| {
                        ui.set_width(130.0);
                        for (cat, name) in [
                            (PrefsCategory::General, "General"),
                            (PrefsCategory::Appearance, "Appearance"),
                            (PrefsCategory::Editing, "Editing"),
                            (PrefsCategory::Confirmations, "Confirmations"),
                        ] {
                            if ui
                                .selectable_label(self.prefs_category == cat, name)
                                .clicked()
                            {
                                self.prefs_category = cat;
                            }
                        }
                    });
                    ui.separator();
                    // --- Right: the selected category's settings (fixed width) ---
                    ui.vertical(|ui| {
                        ui.set_width(400.0);
                        match self.prefs_category {
                            PrefsCategory::General => dirty |= self.prefs_general(ui),
                            PrefsCategory::Appearance => dirty |= self.prefs_appearance(ui),
                            PrefsCategory::Editing => dirty |= self.prefs_editing(ui),
                            PrefsCategory::Confirmations => {
                                dirty |= self.prefs_confirmations(ui)
                            }
                        }
                    });
                });
            });
        if dirty {
            self.prefs_dirty = true;
        }
        // Persist once the user settles (pointer released), so dragging a slider
        // doesn't rewrite the config every frame.
        if self.prefs_dirty && !ctx.input(|i| i.pointer.any_down()) {
            let _ = layouts::save_config(&self.config);
            self.prefs_dirty = false;
        }
        self.show_preferences = open;
    }

    /// Preferences > General. Returns true if a config value changed.
    fn prefs_general(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.add_space(2.0);
        // Autosave: a master on/off toggle plus the interval (disabled when off).
        let mut enabled = self.config.autosave_secs > 0;
        if ui.checkbox(&mut enabled, "Autosave").changed() {
            self.config.autosave_secs = if enabled { 30 } else { 0 };
            changed = true;
        }
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Interval");
                let cur = self.config.autosave_secs.max(15);
                let label = if cur % 60 == 0 {
                    format!("Every {} min", cur / 60)
                } else {
                    format!("Every {cur}s")
                };
                egui::ComboBox::from_id_salt("pref_autosave")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (v, name) in [
                            (15u32, "Every 15s"),
                            (30, "Every 30s"),
                            (60, "Every 1 min"),
                            (300, "Every 5 min"),
                        ] {
                            changed |= ui
                                .selectable_value(&mut self.config.autosave_secs, v, name)
                                .changed();
                        }
                    });
            });
        });
        ui.weak("Periodically saves a recoverable copy of modified projects.");
        ui.separator();
        changed |= ui
            .checkbox(&mut self.config.check_updates, "Check for updates on startup")
            .changed();
        changed |= ui
            .checkbox(&mut self.config.reopen_last, "Reopen last project on startup")
            .changed();
        changed |= ui
            .checkbox(
                &mut self.config.show_info_on_startup,
                "Show the Info window on startup",
            )
            .changed();
        changed
    }

    /// Preferences > Appearance. Returns true if a config value changed.
    fn prefs_appearance(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.add_space(2.0);
        changed |= ui.checkbox(&mut self.config.light_mode, "Light mode").changed();
        ui.separator();
        ui.label("Interface zoom");
        ui.horizontal(|ui| {
            let resp =
                ui.add(egui::Slider::new(&mut self.config.ui_zoom, 0.4..=2.0).show_value(false));
            // Hold the zoom while dragging; it's applied (in `update`) once released.
            self.zoom_slider_active = resp.dragged();
            changed |= resp.changed();
            ui.label(format!("{:.0}%", self.config.ui_zoom * 100.0));
        });
        ui.weak("Scales the whole interface (40%–200%). Applied when you release the slider.");
        changed
    }

    /// Preferences > Editing. Returns true if a config value changed. Preview
    /// quality and the grab margin are global; the guide lines edit the current
    /// project (per-project document state). Atlas padding stays project-side (in
    /// the Atlas panel), not here.
    fn prefs_editing(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.add_space(2.0);
        ui.label("Preview quality");
        changed |= ui
            .add(egui::Slider::new(&mut self.config.preview_quality, 0.1..=1.0))
            .changed();
        ui.weak("Live perspective-warp preview while dragging (lower = faster).");
        ui.separator();
        ui.label("Handle grab margin");
        changed |= ui
            .add(egui::Slider::new(&mut self.config.cursor_margin, 1.0..=50.0).suffix(" px"))
            .changed();
        ui.weak("How close the cursor must be to grab a rip's corners / edges.");
        ui.separator();
        ui.label("Guide lines (current project)");
        let g = &mut self.projects[self.active].guides;
        ui.checkbox(&mut g.enabled, "Show guide lines");
        ui.add(egui::Slider::new(&mut g.vertical, 0..=20).text("Vertical"));
        ui.add(egui::Slider::new(&mut g.horizontal, 0..=20).text("Horizontal"));
        changed
    }

    /// Preferences > Confirmations. Returns true if a config value changed.
    fn prefs_confirmations(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.add_space(2.0);
        ui.label("Ask for confirmation before:");
        ui.add_space(2.0);
        changed |= ui
            .checkbox(
                &mut self.config.confirm_close_modified,
                "Closing or quitting with unsaved changes",
            )
            .changed();
        changed |= ui
            .checkbox(
                &mut self.config.confirm_layout_overwrite,
                "Overwriting an existing layout",
            )
            .changed();
        ui.add_space(8.0);
        ui.separator();
        if ui
            .button("Reset all confirmation prompts")
            .on_hover_text("Re-enable every prompt you dismissed with \"Don't ask again\"")
            .clicked()
        {
            self.config.confirm_close_modified = true;
            self.config.confirm_layout_overwrite = true;
            changed = true;
        }
        changed
    }

    /// "This project has unsaved changes — close it?" confirmation, with a
    /// "Don't ask again" opt-out and a Save-first option.
    fn confirm_close_window(&mut self, ctx: &egui::Context) {
        let Some(index) = self.close_req else { return };
        if index >= self.projects.len() {
            self.close_req = None;
            return;
        }
        let name = self.projects[index].name.clone();
        let mut open = true;
        let mut decided: Option<bool> = None; // Some(true) = close, Some(false) = cancel
        let mut save_first = false;
        egui::Window::new("Unsaved changes")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("\"{name}\" has unsaved changes. Close it anyway?"));
                ui.add_space(4.0);
                ui.checkbox(&mut self.close_dont_ask, "Don't ask again");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save & close").clicked() {
                        save_first = true;
                        decided = Some(true);
                    }
                    if ui.button("Discard & close").clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
        if !open {
            decided = Some(false); // closing the dialog itself = cancel
        }
        if let Some(close) = decided {
            if close {
                // Persist the opt-out before acting so it sticks regardless.
                if self.close_dont_ask {
                    self.config.confirm_close_modified = false;
                    let _ = layouts::save_config(&self.config);
                }
                if save_first {
                    self.active = index;
                    // Synchronous — must finish before we can close the tab.
                    if !self.save_now(index) {
                        // Save cancelled / failed → keep the project open.
                        self.close_req = None;
                        return;
                    }
                }
                self.close_project(index);
            }
            self.close_req = None;
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
                            // Custom link: always underlined, with a pointing-hand
                            // cursor on hover (egui's default link is blue and only
                            // underlines on hover). The colour follows the theme —
                            // white on dark, black on light — so it stays readable.
                            let link_color = if ui.visuals().dark_mode {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::BLACK
                            };
                            let text = egui::RichText::new(link).color(link_color).underline();
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
                    ui.label("Version 1.3.3");
                    ui.weak(format!(
                        "Built {} {} UTC",
                        env!("BUILD_DATE"),
                        env!("BUILD_TIME")
                    ));
                    ui.add_space(4.0);
                });
            });
        self.show_about = open;
    }

    /// The Info / quick-controls window (opens on startup, reopenable via
    /// Help > Info). Content is the embedded `info.md`, lightly rendered.
    /// Polls the background update check and, once it finishes, replaces the
    /// "Searching for updates…" status with the result.
    fn poll_update_check(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.update_rx else { return };
        match rx.try_recv() {
            Ok(outcome) => {
                let v = env!("CARGO_PKG_VERSION");
                match outcome {
                    crate::update::Outcome::UpToDate => {
                        self.set_status(format!("Up to date (v{v})."))
                    }
                    // A newer release with a downloaded exe ready: swap it in and
                    // relaunch silently (the download happened on the background
                    // thread, so the UI never blocked). Applying force-closes the
                    // app (bypassing the quit guard), so only do it when there's
                    // nothing unsaved to lose — if the user has already started
                    // editing, just flag it and let the next launch apply it.
                    crate::update::Outcome::Available {
                        tag,
                        ready_exe: Some(exe),
                    } => {
                        if self.has_unsaved() {
                            self.set_status(format!(
                                "Update {tag} downloaded — restart to apply."
                            ));
                        } else {
                            match crate::update::spawn_replace_worker(&exe) {
                                Ok(()) => {
                                    self.set_status(format!("Updating to {tag}…"));
                                    self.allow_close = true;
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                Err(e) => {
                                    self.set_error(format!("Update to {tag} failed: {e}"))
                                }
                            }
                        }
                    }
                    // Newer release, but nothing to auto-apply (debug build, no exe
                    // asset, or the download failed) — just point the user at it.
                    crate::update::Outcome::Available { tag, ready_exe: None } => {
                        self.set_status(format!(
                            "Update available: {tag} (you have v{v}) — github.com/L30ZMine/ricks-textureripper/releases"
                        ))
                    }
                    crate::update::Outcome::Failed => {
                        self.set_status("Couldn't check for updates.")
                    }
                }
                self.update_rx = None;
            }
            // Still running: keep the UI ticking so we pick the result up promptly.
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(400));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_rx = None,
        }
    }

    /// First-run setup: choose where preferences live and optionally install.
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

                // If this changes the active storage location, tell the user their
                // data will move — this note is the confirmation for that move.
                if self.first_run_portable != layouts::is_portable() {
                    ui.add_space(6.0);
                    let where_to = if self.first_run_portable {
                        "next to the program"
                    } else {
                        "Documents"
                    };
                    let n = layouts::user_layout_count();
                    if n > 0 {
                        let s = if n == 1 { "" } else { "s" };
                        ui.weak(format!(
                            "    Your preferences and {n} saved layout{s} will move to {where_to}."
                        ));
                    } else {
                        ui.weak(format!("    Your preferences will move to {where_to}."));
                    }
                }

                #[cfg(windows)]
                {
                    ui.add_space(10.0);
                    // Installing a debug build makes no sense (it's not a
                    // distributable exe), so the option is release-only.
                    if cfg!(debug_assertions) {
                        ui.weak("Install to Program Files is disabled in debug builds.");
                    } else if crate::install::is_installed() {
                        // Already running from Program Files — nothing to install,
                        // so grey the option out.
                        self.first_run_install = false;
                        ui.add_enabled(
                            false,
                            egui::Checkbox::new(
                                &mut self.first_run_install,
                                "Install to Program Files and add a Start Menu shortcut",
                            ),
                        );
                        ui.weak("    Already installed (running from Program Files).");
                    } else if crate::install::installed_exists() {
                        // An install already exists elsewhere — compare versions and
                        // reframe the action as an update (the install worker
                        // overwrites the existing exe, so it *is* the update).
                        self.install_option_ui(ui);
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
            // Dismissed via the window's close button without applying.
            self.show_first_run = false;
        }
    }

    /// Renders the install row when a Program Files install already exists,
    /// reframing the action by version: up-to-date (disabled), update an older
    /// (or unknown) install, or replace a newer one. The install worker overwrites
    /// the existing exe, so "update" and "install" are the same operation — only
    /// the wording differs. Windows-only.
    #[cfg(windows)]
    fn install_option_ui(&mut self, ui: &mut egui::Ui) {
        use std::cmp::Ordering;
        let running = env!("CARGO_PKG_VERSION");
        let installed = crate::install::installed_version();
        let inst_label = installed.as_deref().unwrap_or("unknown");
        // Running vs installed; `None` when the installed version is unknown.
        let rel = installed
            .as_deref()
            .and_then(|v| crate::update::version_cmp(running, v));
        match rel {
            Some(Ordering::Equal) => {
                // The install already matches this build — nothing to do.
                self.first_run_install = false;
                ui.add_enabled(
                    false,
                    egui::Checkbox::new(
                        &mut self.first_run_install,
                        format!("Existing install is up to date (v{running})"),
                    ),
                );
                ui.weak("    The installed copy already matches this version.");
            }
            Some(Ordering::Less) => {
                // The installed copy is newer than what's running — updating would
                // downgrade it, so make the user opt in explicitly.
                ui.checkbox(
                    &mut self.first_run_install,
                    format!("Replace the newer installed copy (v{inst_label} -> v{running})"),
                );
                ui.weak("    A newer version is already installed — this downgrades it.");
            }
            _ => {
                // An older install, or one with an unknown (pre-stamping) version.
                let from = if matches!(rel, Some(Ordering::Greater)) {
                    format!("v{inst_label} -> ")
                } else {
                    String::new()
                };
                ui.checkbox(
                    &mut self.first_run_install,
                    format!("Update the existing install ({from}v{running})"),
                );
                ui.weak("    Replaces the installed copy and reopens from there (UAC).");
            }
        }
    }

    /// Applies the first-run choices: storage location, saved config, optional
    /// (elevated) install. On install the app closes itself so the elevated
    /// worker can replace/relaunch it from the new path.
    fn finish_first_run(&mut self, ctx: &egui::Context) {
        // Apply the chosen storage location (explicitly either way so switching
        // back to Documents from portable also takes effect).
        let new_dir = if self.first_run_portable {
            layouts::portable_dir()
        } else {
            layouts::documents_dir()
        };
        if let Some(new_dir) = new_dir {
            // Where the app is reading from now, before the switch.
            let old_dir = layouts::app_dir();
            layouts::set_app_dir(new_dir.clone());
            // Move the user's existing data so changing locations doesn't orphan
            // their layouts/autosaves/config — and so switching away from portable
            // actually sticks (see `layouts::migrate_storage`).
            if let Some(old_dir) = old_dir {
                if old_dir != new_dir {
                    let report = layouts::migrate_storage(&old_dir, &new_dir);
                    self.set_status(report.summary());
                }
            }
        }
        // Writing the config marks setup complete (it now exists, so the dialog
        // won't reappear next launch) and seeds the new location.
        if let Err(e) = layouts::save_config(&self.config) {
            self.set_error(format!("Couldn't save preferences: {e}"));
        }
        self.show_first_run = false;

        // Never install a debug build (it's not a distributable exe).
        if self.first_run_install && !cfg!(debug_assertions) {
            match crate::install::install_to_program_files() {
                Ok(()) => {
                    // The elevated worker waits for us to exit, then deletes this
                    // exe and relaunches from Program Files. This is an explicit,
                    // deliberate close, so bypass the quit guard.
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => self.set_error(format!("Install failed: {e}")),
            }
        }
    }

    /// True when any open project has unsaved changes.
    fn has_unsaved(&self) -> bool {
        self.projects.iter().any(|p| p.modified)
    }

    /// Saves every modified project for a quit (prompting Save As for un-named
    /// ones). Returns false if any save was cancelled/failed (so quit aborts).
    fn save_all_for_quit(&mut self) -> bool {
        for i in 0..self.projects.len() {
            if self.projects[i].modified {
                self.active = i;
                // Synchronous (not deferred) — the quit must wait for the save.
                if !self.save_now(i) {
                    return false;
                }
            }
        }
        true
    }

    /// Intercepts close requests (window X / Alt+F4 / OS shutdown / Exit / Ctrl+Q)
    /// and confirms when there are unsaved changes, vetoing the close until the
    /// user decides.
    fn quit_guard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            // The same toggle that governs closing a tab governs quitting; if it's
            // off, quit straight through (discarding unsaved work) without asking.
            if !self.allow_close && self.has_unsaved() && self.config.confirm_close_modified {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_quit = true;
                self.quit_dont_ask = false;
            } else if !self.recovery_pending {
                // A close is actually proceeding — record a clean shutdown so the
                // next start doesn't treat it as a crash. But if a crash-recovery
                // offer is still unacknowledged, leave the session lock in place so
                // the offer reappears next launch (only the recover dialog clears it).
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
                ui.add_space(4.0);
                ui.checkbox(&mut self.quit_dont_ask, "Don't ask again");
                ui.add_space(8.0);
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
            // Persist the opt-out so future quits (and tab closes) don't prompt.
            if self.quit_dont_ask {
                self.config.confirm_close_modified = false;
                let _ = layouts::save_config(&self.config);
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Crash-recovery prompt: shown at startup when the previous run didn't shut
    /// down cleanly and autosaves are available.
    /// Startup chooser shown when a config exists in *both* Documents and a
    /// portable folder: presents each with its last-modified time (recommending
    /// the newer) and lets the user pick which to use, optionally folding the
    /// other in so the prompt doesn't recur.
    fn storage_choice_window(&mut self, ctx: &egui::Context) {
        if !self.show_storage_choice {
            return;
        }
        // Some(true) = portable, Some(false) = Documents.
        let mut pick: Option<bool> = None;
        let port_newer = self.storage_port_ms > self.storage_doc_ms;
        let doc_dir = layouts::documents_dir();
        let port_dir = layouts::portable_dir();

        // One labelled option box with a "Use this" button.
        let option = |ui: &mut egui::Ui,
                      title: &str,
                      newer: bool,
                      dir: &Option<PathBuf>,
                      ms: u64|
         -> bool {
            let mut clicked = false;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        if newer {
                            ui.strong(format!("{title}  (newer — recommended)"));
                        } else {
                            ui.strong(title);
                        }
                        if let Some(d) = dir {
                            ui.weak(d.display().to_string());
                        }
                        ui.weak(format!("Last modified: {}", layouts::format_modified(ms)));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked = ui.button("Use this").clicked();
                    });
                });
            });
            clicked
        };

        egui::Window::new("Preferences found in two places")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("You have saved preferences in two locations. Which should this app use?");
                ui.add_space(8.0);
                if option(ui, "Documents", !port_newer, &doc_dir, self.storage_doc_ms) {
                    pick = Some(false);
                }
                ui.add_space(4.0);
                if option(ui, "Next to the program", port_newer, &port_dir, self.storage_port_ms) {
                    pick = Some(true);
                }
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.storage_consolidate,
                    "Move the other copy's layouts here and remove it (stop asking)",
                );
            });

        if let Some(portable) = pick {
            self.apply_storage_choice(portable, ctx);
        }
    }

    /// Applies the storage-location choice: switches the active location, loads its
    /// preferences, optionally consolidates the other copy into it, then starts the
    /// (deferred) autosave session there.
    fn apply_storage_choice(&mut self, portable: bool, ctx: &egui::Context) {
        let (chosen, other) = if portable {
            (layouts::portable_dir(), layouts::documents_dir())
        } else {
            (layouts::documents_dir(), layouts::portable_dir())
        };
        if let Some(chosen) = chosen {
            layouts::set_app_dir(chosen.clone());
            // Load the chosen location's preferences (theme/zoom/prefs auto-apply
            // next frame from `self.config`).
            self.config = layouts::load_config();
            // Optionally fold the other copy's layouts/autosaves in and remove it,
            // so a single source of truth remains and the prompt doesn't recur.
            if self.storage_consolidate {
                if let Some(other) = other {
                    if other != chosen {
                        let report = layouts::migrate_storage(&other, &chosen);
                        self.set_status(report.summary());
                    }
                }
            }
            // Re-stamp the chosen config (marks it newest going forward).
            let _ = layouts::save_config(&self.config);
            // Rebuild the pristine startup project's dock from the chosen default
            // layout (the provisional location's may have differed).
            if self.projects.len() == 1
                && !self.projects[0].modified
                && self.projects[0].images.is_empty()
            {
                self.projects[0].dock_state = self.new_project_dock();
            }
        }
        self.show_storage_choice = false;
        // The location is settled — start the autosave session against it now.
        if self.session_pending {
            self.session_pending = false;
            self.begin_session();
        }
        ctx.request_repaint();
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
            // Clone the paths out so we can mutably borrow `self` to open them.
            let paths: Vec<std::path::PathBuf> =
                self.recover_entries.iter().map(|e| e.path.clone()).collect();
            let mut opened = 0;
            for path in &paths {
                match crate::autosave::open_recovered(ctx, path) {
                    Ok(project) => {
                        // Replace the pristine startup tab with the first recovery.
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
            // The offer has been acted on; let the next clean shutdown clear the
            // crash flag.
            self.recovery_pending = false;
        } else if dismiss {
            self.show_recover = false;
            self.recovery_pending = false;
        }
    }

    /// Adds any image files dropped onto the window, and shows a hover overlay
    /// while files are dragged over it. The whole window is the drop target.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // Overlay hint while files hover over the window.
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
        // Collect supported image paths and decode them on the background loader
        // (the app stays responsive; a % shows by the cursor).
        let paths: Vec<PathBuf> = dropped
            .into_iter()
            .filter_map(|f| f.path)
            .filter(|p| crate::texture_view::is_supported_image(p))
            .collect();
        if !paths.is_empty() {
            self.image_loader.start(self.active, paths);
        }
    }

    fn info_window(&mut self, ctx: &egui::Context) {
        // Hold the Info window back until the first-run setup dialog is dismissed.
        if self.show_first_run {
            return;
        }
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

        // Closing the Info window turns off show-on-startup only on the *first*
        // launch (so brand-new users see it once, then never again). On later runs
        // the Preferences toggle is the sole control — closing the window must not
        // flip it off, or a user who re-enabled it would lose that choice.
        if was_open && !open && self.first_launch && self.config.show_info_on_startup {
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
                                        if ui.small_button("×").on_hover_text("Dismiss").clicked() {
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
                                if ui.small_button("×").on_hover_text("Dismiss").clicked() {
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

    /// Saves the layout, first prompting to confirm if it would overwrite an
    /// existing one (unless the user has opted out of that prompt).
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

    /// "A layout named X already exists. Overwrite it?" with a "Don't ask again"
    /// checkbox that disables the prompt for future saves.
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
            // Persist the opt-out before saving so it sticks even if the save fails.
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

/// Starts a background full-resolution render for every rip still showing a live
/// preview. Each source image's working pixels are cloned once (shared via `Arc`)
/// so rips on the same image don't each copy it.
fn start_rip_render(project: &mut Project) {
    use std::collections::HashMap;
    use std::sync::Arc;
    let mut srcs: HashMap<usize, Arc<image::RgbaImage>> = HashMap::new();
    let mut inputs = Vec::new();
    for (i, rip) in project.rips.iter().enumerate() {
        if !rip.previewed || rip.image >= project.images.len() {
            continue;
        }
        let src = srcs
            .entry(rip.image)
            .or_insert_with(|| Arc::new(project.images[rip.image].pixels.clone()))
            .clone();
        inputs.push(crate::render::RipRenderInput {
            rip: i,
            src,
            shape: rip.shape.clone(),
            adjust: rip.adjust,
            orient: rip.orient,
            resize: rip.resize,
        });
    }
    if !inputs.is_empty() {
        project.renderer.start(inputs);
    }
}

/// Applies any full-resolution rip renders the background worker has finished:
/// uploads each as a texture (the only UI-thread cost) and swaps it in for the
/// preview. Repacks the atlas if anything changed.
fn apply_rip_render_results(ctx: &egui::Context, project: &mut Project) {
    let msgs = project.renderer.poll();
    if msgs.is_empty() {
        return;
    }
    for msg in msgs {
        if msg.rip >= project.rips.len() {
            continue;
        }
        match msg.result {
            Some((size, pixels)) => {
                let texture =
                    crate::texture_view::upload_texture(ctx, &project.rips[msg.rip].name, &pixels);
                project.rips[msg.rip].output = Some(crate::project::RipOutput {
                    size,
                    texture,
                    pixels,
                });
            }
            None => project.rips[msg.rip].output = None,
        }
        project.rips[msg.rip].previewed = false;
    }
    project.atlas_dirty = true;
}
