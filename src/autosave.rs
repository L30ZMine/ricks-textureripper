//! Autosave + crash recovery.
//!
//! Every ~30s each *modified* project is written (off the UI thread) to
//! `<app_dir>/autosaves/` as a self-contained `.rtrpf`, named
//! `<name>__<project-id>__<unix-millis>.rtrpf`. The newest two per project are
//! kept; older ones go to the Recycle Bin.
//!
//! A `session.lock` marker exists while the app runs and is removed on a clean
//! shutdown. If it's still present at the next start, the previous run crashed,
//! and the user is offered the latest autosaves to recover.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project::Project;

/// `<app_dir>/autosaves`, created if missing.
pub fn dir() -> Option<PathBuf> {
    let p = crate::layouts::app_dir()?.join("autosaves");
    fs::create_dir_all(&p).ok()?;
    Some(p)
}

fn lock_path() -> Option<PathBuf> {
    Some(dir()?.join("session.lock"))
}

/// One recoverable autosave (the newest file for a project group).
pub struct Recoverable {
    pub path: PathBuf,
    /// The project's name, parsed back out of the file name.
    pub name: String,
}

/// Begins a session: returns `(crashed, recoverable)` where `crashed` is true if
/// the previous run didn't shut down cleanly, and `recoverable` lists the newest
/// autosave per project to offer. Always (re)creates the running marker.
pub fn start_session() -> (bool, Vec<Recoverable>) {
    let crashed = lock_path().is_some_and(|p| p.exists());
    let recoverable = if crashed { latest_per_group() } else { Vec::new() };
    if let Some(p) = lock_path() {
        let _ = fs::write(&p, b"running");
    }
    (crashed, recoverable)
}

/// Marks a clean shutdown (removes the running marker), so the next start does
/// not treat it as a crash.
pub fn mark_clean_shutdown() {
    if let Some(p) = lock_path() {
        let _ = fs::remove_file(p);
    }
}

/// Milliseconds since the Unix epoch (monotonic enough for ordering autosaves).
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Makes `name` safe to use inside a file name.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "unnamed".to_string() } else { s }
}

/// The group key of an autosave file name (everything before the final
/// `__<millis>`), i.e. `<name>__<id>`. `None` if it doesn't match the pattern.
fn group_of(file_name: &str) -> Option<String> {
    let stem = file_name.strip_suffix(".rtrpf")?;
    let last = stem.rfind("__")?;
    Some(stem[..last].to_string())
}

/// Autosaves every modified project off the UI thread, then prunes old files.
/// Best-effort: failures are silently ignored (autosave must never disrupt work).
pub fn autosave_modified(projects: &[Project]) {
    let Some(dir) = dir() else { return };
    let mut wrote = false;
    for project in projects.iter().filter(|p| p.modified) {
        let file = format!("{}__{}__{}.rtrpf", sanitize(&project.name), project.id, now_millis());
        let path = dir.join(file);
        let job = crate::proj_io::capture_for_save(project);
        std::thread::spawn(move || {
            let _ = crate::proj_io::write_job(&path, job);
        });
        wrote = true;
    }
    if wrote {
        // Give the writer threads a moment so pruning sees the new files; pruning
        // itself only ever removes *older* ones, so a race just defers a cleanup.
        retain_latest_two();
    }
}

/// Keeps the two newest autosaves per project group; sends the rest to the
/// Recycle Bin.
pub fn retain_latest_two() {
    let Some(dir) = dir() else { return };
    let Ok(read) = fs::read_dir(&dir) else { return };

    // Collect (group, millis, path) for every autosave file.
    let mut files: Vec<(String, u128, PathBuf)> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.ends_with(".rtrpf") {
            continue;
        }
        let Some(group) = group_of(name) else { continue };
        let millis = name
            .strip_suffix(".rtrpf")
            .and_then(|s| s.rsplit("__").next())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);
        files.push((group, millis, path));
    }

    // Per group, keep the two newest; gather the rest to recycle.
    use std::collections::HashMap;
    let mut by_group: HashMap<String, Vec<(u128, PathBuf)>> = HashMap::new();
    for (g, m, p) in files {
        by_group.entry(g).or_default().push((m, p));
    }
    let mut to_recycle: Vec<PathBuf> = Vec::new();
    for (_g, mut list) in by_group {
        list.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
        for (_m, p) in list.into_iter().skip(2) {
            to_recycle.push(p);
        }
    }
    if !to_recycle.is_empty() {
        recycle(&to_recycle);
    }
}

/// The newest autosave file for each project group (for the recovery dialog).
fn latest_per_group() -> Vec<Recoverable> {
    let Some(dir) = dir() else { return Vec::new() };
    let Ok(read) = fs::read_dir(&dir) else { return Vec::new() };

    use std::collections::HashMap;
    // group -> (millis, path, display-name)
    let mut newest: HashMap<String, (u128, PathBuf, String)> = HashMap::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.ends_with(".rtrpf") {
            continue;
        }
        let Some(group) = group_of(name) else { continue };
        let millis = name
            .strip_suffix(".rtrpf")
            .and_then(|s| s.rsplit("__").next())
            .and_then(|s| s.parse::<u128>().ok())
            .unwrap_or(0);
        // Display name = the part of the group before the last "__<id>".
        let display = group.rsplitn(2, "__").last().unwrap_or(&group).to_string();
        let slot = newest.entry(group).or_insert((0, path.clone(), display.clone()));
        if millis >= slot.0 {
            *slot = (millis, path, display);
        }
    }
    newest
        .into_values()
        .map(|(_m, path, name)| Recoverable { path, name })
        .collect()
}

/// Sends `paths` to the Recycle Bin (Windows) so the user can review/restore
/// them; on other platforms it deletes them. Best-effort and headless.
fn recycle(paths: &[PathBuf]) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // Build a script that recycles each path via Microsoft.VisualBasic.
        let mut script = String::from("Add-Type -AssemblyName Microsoft.VisualBasic\n");
        for p in paths {
            // Single-quote and escape any embedded quotes for PowerShell.
            let escaped = p.display().to_string().replace('\'', "''");
            script.push_str(&format!(
                "[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{escaped}','OnlyErrorDialogs','SendToRecycleBin')\n"
            ));
        }
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Loads an autosave file into a `Project` (marked unsaved, with no on-disk path
/// so the user is prompted to Save As). Thin wrapper over `proj_io::open`.
pub fn open_recovered(ctx: &egui::Context, path: &Path) -> Result<Project, String> {
    let mut project = crate::proj_io::open(ctx, path)?;
    project.path = None;
    project.modified = true;
    Ok(project)
}
