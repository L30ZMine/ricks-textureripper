//! Project file I/O — the `.rtrpf` ("Rick's Texture Ripper Project File") format.
//!
//! A project file is JSON holding the document [`ProjectSnapshot`], the project
//! name, its dock layout, and (since format **version 2**) the **embedded source
//! pixels** of every image. Embedding makes a project self-contained: moving or
//! deleting the original image files no longer breaks reload. Version-1 files
//! carried no pixels and reloaded images from `source_path`; that disk fallback
//! is still used for any image whose embedded data is absent or unreadable.

use std::path::Path;

use egui_dock::DockState;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::snapshot::{self, ProjectSnapshot};
use crate::ui::docking::PanelTab;

/// File extension for project files.
pub const EXTENSION: &str = "rtrpf";

/// Current on-disk format version (2 = images embedded).
const FORMAT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    /// Format version, for forward compatibility.
    version: u32,
    name: String,
    snapshot: ProjectSnapshot,
    dock_state: DockState<PanelTab>,
    /// Embedded source pixels (format v2+). Empty for v1 files (reload from disk).
    #[serde(default)]
    images: Vec<EmbeddedImage>,
}

/// One image's original pixels embedded in the project file, keyed by the same
/// `source_path` the snapshot references so it can be matched up on open.
#[derive(Serialize, Deserialize)]
struct EmbeddedImage {
    source_path: std::path::PathBuf,
    /// Base64 of the PNG-encoded original (pre-adjustment, pre-resize) pixels.
    png: String,
}

/// PNG-encodes `img` and Base64s it for embedding.
fn encode_image(img: &RgbaImage) -> Result<String, String> {
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(crate::b64::encode(&buf))
}

/// Reverses [`encode_image`].
fn decode_image(png_b64: &str) -> Result<RgbaImage, String> {
    let bytes = crate::b64::decode(png_b64).ok_or("invalid base64 in project file")?;
    Ok(image::load_from_memory(&bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8())
}

/// A snapshot of everything needed to write a project file, captured cheaply on
/// the UI thread (image *pixels* are memcpy-cloned, but the expensive PNG/Base64
/// encoding happens later in [`write_job`]). It is `Send`, so autosave can encode
/// and write off the main thread.
pub struct SaveJob {
    name: String,
    snapshot: ProjectSnapshot,
    dock_state: DockState<PanelTab>,
    originals: Vec<(std::path::PathBuf, RgbaImage)>,
}

/// Captures a [`SaveJob`] from `project` (cheap: clones metadata + raw pixels,
/// no encoding).
pub fn capture_for_save(project: &Project) -> SaveJob {
    SaveJob {
        name: project.name.clone(),
        snapshot: snapshot::capture(project),
        dock_state: project.dock_state.clone(),
        originals: project
            .images
            .iter()
            .map(|img| (img.source_path.clone(), img.original.clone()))
            .collect(),
    }
}

/// Encodes and writes a [`SaveJob`] to `path` (the heavy part — safe to call off
/// the UI thread).
pub fn write_job(path: &Path, job: SaveJob) -> Result<(), String> {
    let images = job
        .originals
        .iter()
        .map(|(source_path, img)| {
            Ok(EmbeddedImage {
                source_path: source_path.clone(),
                png: encode_image(img)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let file = ProjectFile {
        version: FORMAT_VERSION,
        name: job.name,
        snapshot: job.snapshot,
        dock_state: job.dock_state,
        images,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Serializes `project` to `path`, embedding each image's original pixels.
pub fn save(path: &Path, project: &Project) -> Result<(), String> {
    write_job(path, capture_for_save(project))
}

/// Loads a project from `path`. Embedded pixels are decoded first so `restore`
/// reuses them (matched by `source_path`); any image without embedded data falls
/// back to reloading from its original file on disk.
pub fn open(ctx: &egui::Context, path: &Path) -> Result<Project, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: ProjectFile = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let mut project = Project::new(file.name, file.dock_state);

    // Pre-populate the loaded images from the embedded pixels. `restore` matches
    // images by `source_path` and reuses these (only its name/pos/size/adjust are
    // applied), so it never has to touch the disk for an embedded image.
    for emb in &file.images {
        match decode_image(&emb.png) {
            Ok(rgba) => {
                let name = emb
                    .source_path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_string());
                project.images.push(crate::texture_view::loaded_image_from_pixels(
                    ctx,
                    name,
                    emb.source_path.clone(),
                    rgba,
                ));
            }
            Err(e) => project.set_error(format!(
                "Embedded image {} unreadable ({e}); trying disk.",
                emb.source_path.display()
            )),
        }
    }

    snapshot::restore(ctx, &mut project, &file.snapshot);
    project.reset_history();
    project.modified = false; // freshly loaded from disk
    Ok(project)
}
