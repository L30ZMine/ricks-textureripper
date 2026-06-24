//! Per-tab project state.
//!
//! Each Photoshop-style tab in the UI owns one `Project`. A project holds all
//! the images, rips, and the generated atlas for that workspace. Switching tabs
//! swaps the whole `Project` that the panels render.

use std::path::PathBuf;

use egui::{TextureHandle, Vec2};
use egui_dock::DockState;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::history::History;
use crate::rip_tool::{RipEditor, RipShape};
use crate::ui::docking::PanelTab;

/// Live, non-destructive image adjustments (brightness / contrast / saturation).
/// Each value is in `-1.0..=1.0`, where `0.0` means "no change".
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adjustments {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
        }
    }
}

impl Adjustments {
    /// True when no adjustment is applied (lets the pipeline skip work).
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0
    }
}

/// An image loaded into the Texture View.
pub struct LoadedImage {
    pub name: String,
    /// Pixel dimensions `[width, height]` of the current working image.
    pub size: [usize; 2],
    /// GPU texture handle for drawing.
    pub texture: TextureHandle,
    /// Working RGBA pixels (resized + adjusted); rips crop/warp from these.
    pub pixels: RgbaImage,
    /// Downscaled copies of `pixels` (mip chain, each ~half the previous), used
    /// to sample cheaper, anti-aliased source data during live rip previews.
    pub mips: Vec<RgbaImage>,
    /// The originally decoded pixels, kept so adjustments stay non-destructive.
    pub original: RgbaImage,
    /// Live brightness/contrast/saturation adjustments (Phase 5).
    pub adjust: Adjustments,
    /// Set when `adjust`/`size` changed and the working pixels need rebuilding.
    pub dirty: bool,
    /// World-space position of the image's top-left corner on the canvas.
    pub pos: Vec2,
    /// File the image was loaded from (used to reload on project open).
    pub source_path: PathBuf,
}

impl LoadedImage {
    /// Pixel size as a `Vec2`.
    pub fn size_vec(&self) -> Vec2 {
        Vec2::new(self.size[0] as f32, self.size[1] as f32)
    }

    /// Picks the source to sample for a live preview: the smallest mip whose
    /// linear scale is still `>= target_scale` (so we don't undersample),
    /// falling back to the full-resolution pixels. Returns `(scale, image)`.
    pub fn preview_source(&self, target_scale: f32) -> (f32, &RgbaImage) {
        let full_w = self.pixels.width().max(1) as f32;
        let mut best: Option<(f32, &RgbaImage)> = None;
        for m in &self.mips {
            let s = m.width() as f32 / full_w;
            if s >= target_scale {
                best = Some((s, m));
            } else {
                break;
            }
        }
        best.unwrap_or((1.0, &self.pixels))
    }
}

/// The recomputed output of a rip (the flattened/masked sub-image).
pub struct RipOutput {
    pub size: [usize; 2],
    pub texture: TextureHandle,
    pub pixels: RgbaImage,
}

/// A live selection on a source image. Its geometry can be edited at any time;
/// when it changes, `dirty` is set and the `output` is recomputed.
pub struct Rip {
    pub name: String,
    /// Index of the source image in `Project::images`.
    pub image: usize,
    /// Selection geometry (image-local coords).
    pub shape: RipShape,
    /// Live brightness/contrast/saturation adjustments applied to the output.
    pub adjust: Adjustments,
    /// Optional output size override `[w, h]`; `None` keeps the natural size.
    pub resize: Option<[u32; 2]>,
    /// Set when geometry changed and the output needs recomputing.
    pub dirty: bool,
    /// Cached flattened output (texture + pixels), recomputed live.
    pub output: Option<RipOutput>,
}

/// Atlas packing settings (exposed in the Atlas View).
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AtlasSettings {
    /// Padding between rips, in pixels.
    pub padding: u32,
    /// Export resolution `[w, h]`. Aspect-locked to the packed atlas: editing one
    /// dimension updates the other. `0` means "follow the natural packed size".
    pub export_w: u32,
    pub export_h: u32,
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            padding: 0,
            export_w: 0,
            export_h: 0,
        }
    }
}

/// Where a rip ended up in the packed atlas.
pub struct AtlasPlacement {
    pub rip: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The packed atlas output.
pub struct AtlasResult {
    pub size: [usize; 2],
    /// Composited atlas pixels (used for export; the live preview draws each rip
    /// texture per-cell instead of a single composited texture).
    pub pixels: RgbaImage,
    pub placements: Vec<AtlasPlacement>,
    pub used_count: usize,
}

#[derive(Default)]
pub struct Atlas {
    pub settings: AtlasSettings,
    pub result: Option<AtlasResult>,
}

/// Subdivision guide lines drawn inside the selected quad rip, to help align
/// a perspective selection with features in the image (toggled in Edit menu or
/// the Texture View's top-right icon).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Guides {
    pub enabled: bool,
    /// Number of interior vertical lines.
    pub vertical: u32,
    /// Number of interior horizontal lines.
    pub horizontal: u32,
}

impl Default for Guides {
    fn default() -> Self {
        Self {
            enabled: true,
            vertical: 3,
            horizontal: 3,
        }
    }
}

/// Pan/zoom state for a canvas view.
pub struct ViewState {
    /// Screen-space pan offset (pixels).
    pub pan: Vec2,
    /// Zoom factor (1.0 = 100%).
    pub zoom: f32,
    /// Index of the image currently being dragged, if any.
    pub dragging_image: Option<usize>,
    /// True while a Shift-drag over empty canvas is panning the view.
    pub panning: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            dragging_image: None,
            panning: false,
        }
    }
}

pub struct Project {
    pub name: String,
    /// This project's dock layout (each project can be arranged independently).
    pub dock_state: DockState<PanelTab>,
    /// Images loaded into the Texture View.
    pub images: Vec<LoadedImage>,
    /// Index of the currently selected image (default rip target).
    pub active_image: Option<usize>,
    /// Live rips.
    pub rips: Vec<Rip>,
    /// Rip selection / drag state.
    pub editor: RipEditor,
    /// Texture View pan/zoom.
    pub view: ViewState,
    /// Subdivision guide overlay settings.
    pub guides: Guides,
    /// Packed atlas + settings.
    pub atlas: Atlas,
    /// Set when the rip set changed and the atlas needs repacking.
    pub atlas_dirty: bool,
    /// Set when a low-resolution preview was produced during interaction, so a
    /// full-resolution recompute can run once the user stops adjusting.
    pub needs_full: bool,
    /// Undo/redo history of the editable document state.
    pub history: History,
    /// True when the document has unsaved changes (shown as a `*` on the tab).
    pub modified: bool,
    /// Hit-test margin (screen px) for rip handles: corner grab radius, edge
    /// dead-zone around vertices, and move-region inset. Tunable in Edit menu.
    pub cursor_margin: f32,
    /// Transient status / error message shown in the Texture View.
    pub status: Option<String>,
}

impl Project {
    pub fn new(name: impl Into<String>, dock_state: DockState<PanelTab>) -> Self {
        let mut project = Self {
            name: name.into(),
            dock_state,
            images: Vec::new(),
            active_image: None,
            rips: Vec::new(),
            editor: RipEditor::default(),
            view: ViewState::default(),
            guides: Guides::default(),
            atlas: Atlas::default(),
            atlas_dirty: false,
            needs_full: false,
            history: History::default(),
            modified: true,
            cursor_margin: 15.0,
            status: None,
        };
        project.reset_history();
        project
    }

    /// A unique-ish default name for a new rip.
    pub fn next_rip_name(&self) -> String {
        format!("rip {}", self.rips.len() + 1)
    }

    /// Re-baselines undo/redo to the current state (used after open / on init).
    pub fn reset_history(&mut self) {
        let now = crate::snapshot::capture(self);
        self.history.reset(now);
    }

    /// Pushes an undo step if the document changed since the last commit, and
    /// flags the project as modified (unsaved) when it did.
    pub fn commit_history_if_changed(&mut self) {
        let now = crate::snapshot::capture(self);
        if self.history.commit(now) {
            self.modified = true;
        }
    }
}
