//! Workspace layout persistence.
//!
//! A *layout* is a named, saved dock arrangement (`DockState<PanelTab>`). The
//! built-in `"default"` layout is read-only and never written to disk. User
//! layouts and config live under `Documents/ricks-textureripper/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::ui::docking::PanelTab;

/// Name of the immutable built-in layout.
pub const DEFAULT_LAYOUT: &str = "default";

/// Override for the config/layouts base folder, chosen at startup (resolved from
/// an existing install) or in the Setup dialog. `None` means "use Documents".
static APP_DIR_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Sets the base folder for config + layouts (takes effect immediately, so the
/// Setup dialog can switch it at runtime).
pub fn set_app_dir(dir: PathBuf) {
    if let Ok(mut w) = APP_DIR_OVERRIDE.write() {
        *w = Some(dir);
    }
}

/// `Documents/ricks-textureripper` — the default location (path only, not created).
pub fn documents_dir() -> Option<PathBuf> {
    Some(dirs::document_dir()?.join("ricks-textureripper"))
}

/// `<exe folder>/ricks-textureripper` — the portable location (path only, not created).
pub fn portable_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("ricks-textureripper"))
}

/// The active config/layouts folder (the override if set, else Documents),
/// created if missing.
pub fn app_dir() -> Option<PathBuf> {
    let p = APP_DIR_OVERRIDE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .or_else(documents_dir)?;
    fs::create_dir_all(&p).ok()?;
    Some(p)
}

/// Deletes the app's user-data folders (config, layouts, autosaves) from **both**
/// the Documents and portable locations. Called on an uninstall when the user
/// opts to also remove their data. Best-effort; only ever removes the app's own
/// `ricks-textureripper` folders.
pub fn purge_user_data() {
    for dir in [documents_dir(), portable_dir()].into_iter().flatten() {
        let _ = fs::remove_dir_all(dir);
    }
}

/// True when preferences are currently stored in the portable (next-to-exe)
/// location rather than Documents.
pub fn is_portable() -> bool {
    let active = APP_DIR_OVERRIDE.read().ok().and_then(|g| g.clone());
    match (active, portable_dir()) {
        (Some(a), Some(p)) => a == p,
        _ => false,
    }
}

/// Number of saved user layouts (excludes the built-in `default`). Used by the
/// Setup dialog to tell the user what will move when they change locations.
pub fn user_layout_count() -> usize {
    list_layouts().len().saturating_sub(1)
}

/// What a storage-location migration moved (best-effort; see [`migrate_storage`]).
#[derive(Default)]
pub struct MigrationReport {
    /// Named user layouts moved to the new location.
    pub layouts_moved: usize,
    /// Autosave files (incl. the `session.lock` marker) moved.
    pub autosaves_moved: usize,
    /// Files that couldn't be moved (the migration carries on regardless).
    pub errors: usize,
}

impl MigrationReport {
    /// A one-line summary for the status bar.
    pub fn summary(&self) -> String {
        let mut msg = "Moved your preferences".to_string();
        if self.layouts_moved > 0 {
            let s = if self.layouts_moved == 1 { "" } else { "s" };
            msg.push_str(&format!(" and {} layout{s}", self.layouts_moved));
        }
        msg.push_str(" to the new location.");
        if self.errors > 0 {
            msg.push_str(&format!(" ({} item(s) couldn't be moved.)", self.errors));
        }
        msg
    }
}

/// Moves a single file, replacing the destination if present. Falls back to
/// copy+delete when a plain `rename` can't cross volumes (e.g. Documents on `C:`
/// → a USB stick the portable build lives on).
fn move_file(src: &Path, dst: &Path) -> bool {
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::rename(src, dst).is_ok() {
        return true;
    }
    if fs::copy(src, dst).is_ok() {
        let _ = fs::remove_file(src);
        return true;
    }
    false
}

/// Moves every *file* in `src_dir` into `dst_dir` (replacing same-named files),
/// returning how many moved. A missing `src_dir` is treated as empty.
fn move_dir_files(src_dir: &Path, dst_dir: &Path, errors: &mut usize) -> usize {
    let Ok(read) = fs::read_dir(src_dir) else {
        return 0;
    };
    let mut moved = 0;
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        if move_file(&path, &dst_dir.join(name)) {
            moved += 1;
        } else {
            *errors += 1;
        }
    }
    moved
}

/// Moves the user's data (named layouts + autosaves, incl. the session lock) from
/// `old` into `new` when the storage location changes, so a switch never orphans
/// their layouts or crash-recovery state. The *moving* (current) copy wins on a
/// name clash — it's the data the user is actively using.
///
/// `config.json` is deliberately **removed** from `old` rather than copied: the
/// caller writes the live in-memory config to `new` straight after, and — more
/// importantly — a leftover portable `config.json` next to the exe would be
/// auto-detected by `main::resolve_storage` and silently override a switch *to*
/// Documents on the next launch. Deleting it is what makes the choice stick.
///
/// The now-empty `old` folders are removed last; `remove_dir` only succeeds when a
/// folder is truly empty, so unrelated files left there are never deleted.
pub fn migrate_storage(old: &Path, new: &Path) -> MigrationReport {
    let mut report = MigrationReport::default();
    if old == new {
        return report;
    }
    report.layouts_moved =
        move_dir_files(&old.join("layouts"), &new.join("layouts"), &mut report.errors);
    report.autosaves_moved =
        move_dir_files(&old.join("autosaves"), &new.join("autosaves"), &mut report.errors);
    let _ = fs::remove_file(old.join("config.json"));
    let _ = fs::remove_dir(old.join("layouts"));
    let _ = fs::remove_dir(old.join("autosaves"));
    let _ = fs::remove_dir(old);
    report
}

fn layouts_dir() -> Option<PathBuf> {
    let mut p = app_dir()?;
    p.push("layouts");
    fs::create_dir_all(&p).ok()?;
    Some(p)
}

fn layout_path(name: &str) -> Option<PathBuf> {
    let mut p = layouts_dir()?;
    p.push(format!("{name}.json"));
    Some(p)
}

/// The built-in Blender-style split: Atlas top-left, Image Edit bottom-left,
/// Texture top-right, Rips bottom-right.
pub fn builtin_default() -> DockState<PanelTab> {
    let mut state = DockState::new(vec![PanelTab::Atlas]);
    let surface = state.main_surface_mut();
    let [left, right] = surface.split_right(NodeIndex::root(), 0.5, vec![PanelTab::Texture]);
    let [_atlas, _image_edit] = surface.split_below(left, 0.7, vec![PanelTab::ImageEdit]);
    let [_texture, _rips] = surface.split_below(right, 0.7, vec![PanelTab::Rips]);
    state
}

/// All layout names: `"default"` first, then saved user layouts (alphabetical).
pub fn list_layouts() -> Vec<String> {
    let mut user = Vec::new();
    if let Some(dir) = layouts_dir() {
        if let Ok(read) = fs::read_dir(dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if !stem.eq_ignore_ascii_case(DEFAULT_LAYOUT) {
                            user.push(stem.to_string());
                        }
                    }
                }
            }
        }
    }
    user.sort();
    let mut names = vec![DEFAULT_LAYOUT.to_string()];
    names.extend(user);
    names
}

/// True when a user layout with this name is already saved on disk (the
/// reserved built-in `"default"` is reported as not-existing, since it can't be
/// overwritten anyway).
pub fn layout_exists(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(DEFAULT_LAYOUT) {
        return false;
    }
    layout_path(name).is_some_and(|p| p.exists())
}

pub fn save_layout(name: &str, state: &DockState<PanelTab>) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name cannot be empty.".to_string());
    }
    if name.eq_ignore_ascii_case(DEFAULT_LAYOUT) {
        return Err("\"default\" is reserved and cannot be overwritten.".to_string());
    }
    let path = layout_path(name).ok_or("Could not resolve the layouts folder.")?;
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_layout(name: &str) -> Result<DockState<PanelTab>, String> {
    if name.eq_ignore_ascii_case(DEFAULT_LAYOUT) {
        return Ok(builtin_default());
    }
    let path = layout_path(name).ok_or("Could not resolve the layouts folder.")?;
    let json = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

pub fn delete_layout(name: &str) -> Result<(), String> {
    if name.eq_ignore_ascii_case(DEFAULT_LAYOUT) {
        return Err("The \"default\" layout cannot be deleted.".to_string());
    }
    let path = layout_path(name).ok_or("Could not resolve the layouts folder.")?;
    fs::remove_file(path).map_err(|e| e.to_string())
}

/// App config persisted in `Documents/ricks-textureripper/config.json`.
#[derive(Serialize, Deserialize)]
pub struct Config {
    /// Layout new projects start from.
    pub default_layout: String,
    /// Whether to open the Info window on startup. Cleared once the user closes
    /// the Info window, so it doesn't reappear on the next launch.
    #[serde(default = "default_true")]
    pub show_info_on_startup: bool,
    /// Whether the UI uses the light theme (toggled in Edit > Preferences).
    #[serde(default)]
    pub light_mode: bool,
    /// Recently opened/saved project files, newest first (File > Open Recent).
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,
    /// Whether to confirm before overwriting an existing layout. Cleared by the
    /// "Don't ask again" checkbox in the overwrite-confirmation dialog.
    #[serde(default = "default_true")]
    pub confirm_layout_overwrite: bool,

    // --- Preferences (1.3.2) ------------------------------------------------
    /// Global interface scale (egui zoom factor), `0.4..=2.0` (40%–200%).
    #[serde(default = "default_ui_zoom")]
    pub ui_zoom: f32,
    /// Live perspective-warp preview scale, `0.1..=1.0` (global, applied to every
    /// project — lower is faster but coarser while dragging).
    #[serde(default = "default_preview_quality")]
    pub preview_quality: f32,
    /// Rip-handle grab margin in screen px, `1..=50` (global; Edit > Preferences).
    #[serde(default = "default_cursor_margin")]
    pub cursor_margin: f32,
    /// Autosave cadence in seconds; `0` disables autosave entirely.
    #[serde(default = "default_autosave_secs")]
    pub autosave_secs: u32,
    /// Whether to check for updates (and silently self-update) on startup.
    #[serde(default = "default_true")]
    pub check_updates: bool,
    /// Whether to reopen the most-recent project on startup.
    #[serde(default)]
    pub reopen_last: bool,
    /// Whether to confirm before discarding unsaved changes (closing a project
    /// tab or quitting the app).
    #[serde(default = "default_true")]
    pub confirm_close_modified: bool,
    /// When this config was last written (Unix epoch milliseconds). Stamped fresh
    /// by `save_config` on every write. `0` means unknown (a config saved before
    /// this field existed) — used to tell the user which of two competing config
    /// locations is newer. Not edited in memory; it's read back off disk.
    #[serde(default)]
    pub last_modified: u64,
}

fn default_true() -> bool {
    true
}

fn default_ui_zoom() -> f32 {
    1.0
}

fn default_preview_quality() -> f32 {
    0.4
}

fn default_cursor_margin() -> f32 {
    15.0
}

fn default_autosave_secs() -> u32 {
    30
}

/// How many entries the recent-files list keeps.
const MAX_RECENT: usize = 10;

impl Config {
    /// Records `path` as the most-recently-used project: moves it to the front,
    /// de-duplicated, and caps the list at `MAX_RECENT`.
    pub fn push_recent(&mut self, path: &std::path::Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(MAX_RECENT);
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_layout: DEFAULT_LAYOUT.to_string(),
            show_info_on_startup: true,
            light_mode: false,
            recent_files: Vec::new(),
            confirm_layout_overwrite: true,
            ui_zoom: default_ui_zoom(),
            preview_quality: default_preview_quality(),
            cursor_margin: default_cursor_margin(),
            autosave_secs: default_autosave_secs(),
            check_updates: true,
            reopen_last: false,
            confirm_close_modified: true,
            last_modified: 0,
        }
    }
}

/// Milliseconds since the Unix epoch, for stamping config saves.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Formats a `save_config` timestamp (Unix millis) for the storage-conflict
/// dialog: `"YYYY-MM-DD HH:MM UTC"`, or `"Unknown"` for `0` (a pre-stamping save).
pub fn format_modified(ms: u64) -> String {
    if ms == 0 {
        return "Unknown".to_string();
    }
    let secs = (ms / 1000) as i64;
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    let (hh, mm) = (sod / 3_600, (sod % 3_600) / 60);
    format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mm:02} UTC")
}

/// Day count since 1970-01-01 → `(year, month, day)` (Howard Hinnant's algorithm;
/// mirrors `build.rs`, so the app needs no date crate at runtime).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn config_path() -> Option<PathBuf> {
    let mut p = app_dir()?;
    p.push("config.json");
    Some(p)
}

pub fn load_config() -> Config {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    let path = config_path().ok_or("Could not resolve the config path.")?;
    // Stamp the save time so a "config in two places" conflict can show the user
    // which is newer. Injected into the serialized value (not the in-memory `cfg`)
    // so callers keep passing `&Config`.
    let mut value = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("last_modified".to_string(), serde_json::json!(now_millis()));
    }
    let json = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Reads just the `last_modified` stamp from a config in `dir` (0 if absent or
/// unparseable). Used to compare two competing config locations.
fn config_modified(dir: &Path) -> u64 {
    fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("last_modified").and_then(|x| x.as_u64()))
        .unwrap_or(0)
}

/// What [`resolve`] decided about where preferences live at startup.
pub enum Storage {
    /// No config in either location → show the first-run setup dialog.
    FirstRun,
    /// Exactly one location has a config (the override is set if it's portable).
    Resolved,
    /// Both Documents *and* a portable config exist. The override is set to the
    /// newer one (the sensible default), but the user is asked which to use; the
    /// stamps drive the chooser dialog (`0` = unknown).
    Conflict { doc_ms: u64, port_ms: u64 },
}

/// Resolves where preferences are read from and sets the active override
/// accordingly. Replaces the old "portable always wins" rule: a portable config
/// only wins outright when there's *no* Documents config — when both exist it's a
/// [`Storage::Conflict`] the user disambiguates.
pub fn resolve() -> Storage {
    let has_cfg = |d: &PathBuf| d.join("config.json").is_file();
    let doc = documents_dir().filter(has_cfg);
    let port = portable_dir().filter(has_cfg);
    match (doc, port) {
        (None, None) => Storage::FirstRun,
        (Some(_), None) => Storage::Resolved, // Documents is the default; no override
        (None, Some(p)) => {
            set_app_dir(p);
            Storage::Resolved
        }
        (Some(d), Some(p)) => {
            let (doc_ms, port_ms) = (config_modified(&d), config_modified(&p));
            // Provisionally use the newer one (the dialog's recommended default);
            // Documents stays active when it's newer or equal, so no override.
            if port_ms > doc_ms {
                set_app_dir(p);
            }
            Storage::Conflict { doc_ms, port_ms }
        }
    }
}
