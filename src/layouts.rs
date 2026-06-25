//! Workspace layout persistence.
//!
//! A *layout* is a named, saved dock arrangement (`DockState<PanelTab>`). The
//! built-in `"default"` layout is read-only and never written to disk. User
//! layouts and config live under `Documents/ricks-textureripper/`.

use std::fs;
use std::path::PathBuf;
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

/// True when preferences are currently stored in the portable (next-to-exe)
/// location rather than Documents.
pub fn is_portable() -> bool {
    let active = APP_DIR_OVERRIDE.read().ok().and_then(|g| g.clone());
    match (active, portable_dir()) {
        (Some(a), Some(p)) => a == p,
        _ => false,
    }
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
    /// Whether the UI uses the light theme (toggled in the Window menu).
    #[serde(default)]
    pub light_mode: bool,
    /// Recently opened/saved project files, newest first (File > Open Recent).
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,
}

fn default_true() -> bool {
    true
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
        }
    }
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
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}
