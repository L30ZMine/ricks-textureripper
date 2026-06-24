//! Project file I/O — the `.rtrpf` ("Rick's Texture Ripper Project File") format.
//!
//! A project file is JSON holding the document [`ProjectSnapshot`] plus the
//! project name and its dock layout. Images are stored by source path and
//! reloaded on open; rip outputs and the atlas are recomputed live.

use std::path::Path;

use egui_dock::DockState;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::snapshot::{self, ProjectSnapshot};
use crate::ui::docking::PanelTab;

/// File extension for project files.
pub const EXTENSION: &str = "rtrpf";

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    /// Format version, for forward compatibility.
    version: u32,
    name: String,
    snapshot: ProjectSnapshot,
    dock_state: DockState<PanelTab>,
}

/// Serializes `project` to `path`.
pub fn save(path: &Path, project: &Project) -> Result<(), String> {
    let file = ProjectFile {
        version: 1,
        name: project.name.clone(),
        snapshot: snapshot::capture(project),
        dock_state: project.dock_state.clone(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Loads a project from `path`, reloading its referenced images.
pub fn open(ctx: &egui::Context, path: &Path) -> Result<Project, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let mut project = Project::new(file.name, file.dock_state);
    snapshot::restore(ctx, &mut project, &file.snapshot);
    project.reset_history();
    project.modified = false; // freshly loaded from disk
    Ok(project)
}
