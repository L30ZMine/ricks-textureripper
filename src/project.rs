//! Per-tab project state.
//!
//! Each Photoshop-style tab in the UI owns one `Project`. A project holds all
//! the images, rips, and the generated atlas for that workspace. Switching tabs
//! swaps the whole `Project` that the panels render.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Pos2, TextureHandle, Vec2};
use egui_dock::DockState;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

/// Hands out a unique per-session id to each `Project` (used to name autosaves
/// so concurrent tabs don't collide). Not persisted; resets each run.
static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

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
    /// Hue rotation, `-1.0..=1.0` → `-180°..=180°`.
    #[serde(default)]
    pub hue: f32,
    /// Gamma / midtone lift, `-1.0..=1.0` (0 = unchanged; >0 brightens midtones).
    #[serde(default)]
    pub gamma: f32,
    /// Colour temperature, `-1.0..=1.0` (cool ↔ warm).
    #[serde(default)]
    pub temperature: f32,
    /// Sharpen amount, `0.0..=1.0` (unsharp mask; 0 = off).
    #[serde(default)]
    pub sharpen: f32,
    /// Gaussian blur amount, `0.0..=1.0` (scaled to a pixel radius; 0 = off).
    #[serde(default)]
    pub blur: f32,
    /// Background removal: make pixels near `key_color` transparent.
    #[serde(default)]
    pub key_enabled: bool,
    /// The colour keyed out when `key_enabled` (sRGB).
    #[serde(default = "default_key_color")]
    pub key_color: [u8; 3],
    /// Colour-key tolerance, `0.0..=1.0` (fraction of the full RGB distance).
    #[serde(default = "default_key_tol")]
    pub key_tol: f32,
    /// Colour tint (Multiply blend): tints the output toward `tint_color`.
    #[serde(default)]
    pub tint_enabled: bool,
    /// The tint colour (sRGB) multiplied into the image when `tint_enabled`.
    #[serde(default = "default_tint_color")]
    pub tint_color: [u8; 3],
    /// Tint strength, `0.0..=1.0` (0 = unchanged, 1 = full Multiply).
    #[serde(default = "default_tint_strength")]
    pub tint_strength: f32,
}

/// Default keyed-out colour (a green-screen green).
fn default_key_color() -> [u8; 3] {
    [0, 255, 0]
}

/// Default colour-key tolerance.
fn default_key_tol() -> f32 {
    0.1
}

/// Default tint colour (a neutral red, so the effect is visible when enabled).
fn default_tint_color() -> [u8; 3] {
    [255, 80, 80]
}

/// Default tint strength.
fn default_tint_strength() -> f32 {
    1.0
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
            gamma: 0.0,
            temperature: 0.0,
            sharpen: 0.0,
            blur: 0.0,
            key_enabled: false,
            key_color: default_key_color(),
            key_tol: default_key_tol(),
            tint_enabled: false,
            tint_color: default_tint_color(),
            tint_strength: default_tint_strength(),
        }
    }
}

impl Adjustments {
    /// True when no *colour* adjustment is applied (brightness / contrast /
    /// saturation / hue / gamma / temperature neutral and tint off), so the
    /// per-pixel colour loop can be skipped. Filters (sharpen / blur) and the
    /// colour key are handled separately.
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.hue == 0.0
            && self.gamma == 0.0
            && self.temperature == 0.0
            && !(self.tint_enabled && self.tint_strength > 0.0)
    }
}

/// Rotation / mirroring applied to a rip's output (rip-only — source images keep
/// their orientation so rip geometry stays aligned with the pixels).
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Orientation {
    /// Quarter-turns clockwise: 0–3 (= 0° / 90° / 180° / 270°).
    #[serde(default)]
    pub rotate: u8,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
}

impl Orientation {
    /// True when the rip is unrotated and unflipped.
    pub fn is_identity(&self) -> bool {
        self.rotate % 4 == 0 && !self.flip_h && !self.flip_v
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
    /// Display-only uniform scale for the Texture View (1.0 = natural size).
    /// Aspect-locked by being a single scalar; it changes only how big the image
    /// (and its rips) appear in the workspace — source pixels, rip extraction, and
    /// exported output are unaffected.
    pub scale: f32,
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
    /// For a [`RipShape::CurvedQuad`]: when `true` (Connected) dragging one of a
    /// corner's two bezier handles mirrors the other for a smooth corner; when
    /// `false` (Separate) the two handles move independently. Ignored for other
    /// shapes. Toggled from the Texture View's Shape bar.
    pub bezier_connected: bool,
    /// Live brightness/contrast/saturation adjustments applied to the output.
    pub adjust: Adjustments,
    /// Rotation / mirroring applied to the rip's output (rip-only).
    pub orient: Orientation,
    /// Optional output size override `[w, h]`; `None` keeps the natural size.
    pub resize: Option<[u32; 2]>,
    /// Manual placement (top-left, in atlas pixels) used when the atlas is in
    /// [`SortMode::Manual`]. `None` until the rip is positioned (the packer then
    /// seeds a fallback spot). Ignored in `Automatic` — the packer decides — so
    /// switching back and forth never loses the user's manual layout.
    pub atlas_pos: Option<[f32; 2]>,
    /// Set when geometry changed and the output needs recomputing.
    pub dirty: bool,
    /// True when `output` is a low-resolution live preview awaiting a full-res
    /// render (kicked off in the background once the user settles). Runtime-only.
    pub previewed: bool,
    /// Cached flattened output (texture + pixels), recomputed live.
    pub output: Option<RipOutput>,
}

/// How the export Width/Height relate to each other in the Atlas View.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AspectMode {
    /// Aspect locked to the packed atlas; editing one dimension updates the other.
    Automatic,
    /// Width and height stay equal (1:1); editing either updates both.
    Square,
    /// Free aspect; width and height are edited independently.
    Custom,
}

impl Default for AspectMode {
    fn default() -> Self {
        AspectMode::Automatic
    }
}

/// How rips are arranged inside the atlas (independent of [`AspectMode`]).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {
    /// The bin-packer arranges every rip automatically (the classic behaviour).
    /// This never touches the user's aspect-ratio choice or per-rip stretching.
    Automatic,
    /// The user positions each rip by dragging it in the Atlas preview; positions
    /// persist in [`Rip::atlas_pos`].
    Manual,
}

impl Default for SortMode {
    fn default() -> Self {
        SortMode::Automatic
    }
}

/// Atlas packing settings (exposed in the Atlas View).
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AtlasSettings {
    /// Padding between rips, in pixels.
    pub padding: u32,
    /// Export resolution `[w, h]`. In `Automatic`/`Square` modes the two are kept
    /// in lock-step; `0` means "follow the natural packed size".
    pub export_w: u32,
    pub export_h: u32,
    /// How Width/Height relate when edited (see [`AspectMode`]).
    #[serde(default)]
    pub aspect_mode: AspectMode,
    /// How rips are arranged inside the atlas (see [`SortMode`]).
    #[serde(default)]
    pub sort_mode: SortMode,
    /// In [`SortMode::Manual`], snap dragged rips to the grid + nearby edges.
    #[serde(default)]
    pub snap_enabled: bool,
    /// Grid step (atlas px) used by the Manual snap (when no edge is closer).
    #[serde(default = "default_snap_step")]
    pub snap_step: u32,
}

/// Default Manual-snap grid step (atlas pixels).
fn default_snap_step() -> u32 {
    16
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            padding: 0,
            export_w: 0,
            export_h: 0,
            aspect_mode: AspectMode::Automatic,
            sort_mode: SortMode::Automatic,
            snap_enabled: false,
            snap_step: default_snap_step(),
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
///
/// Holds only the per-rip placements and the natural packed size — never a
/// composited image. The live preview draws each rip texture into its cell, and
/// export composites on demand ([`crate::atlas::export`]), so a repack stays
/// cheap even mid-drag.
pub struct AtlasResult {
    pub size: [usize; 2],
    pub placements: Vec<AtlasPlacement>,
    pub used_count: usize,
}

/// Pan/zoom state for the Atlas preview (runtime-only, not serialized). `pan` is
/// the screen-space offset of the export-bounds *center* from the panel center.
#[derive(Clone, Copy)]
pub struct AtlasView {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for AtlasView {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

#[derive(Default)]
pub struct Atlas {
    pub settings: AtlasSettings,
    pub result: Option<AtlasResult>,
    /// Pan/zoom for the preview (runtime-only).
    pub view: AtlasView,
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
    /// Index of the image whose corner scale-grip is being dragged, if any.
    pub scaling_image: Option<usize>,
    /// True while a Shift-drag over empty canvas is panning the view.
    pub panning: bool,
    /// Screen position of the canvas top-left as of the last frame. Used to keep
    /// the view anchored when the toolbar's contextual row pops in/out (which
    /// would otherwise shift this canvas's origin and jump the whole image).
    pub last_origin: Option<Pos2>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            dragging_image: None,
            scaling_image: None,
            panning: false,
            last_origin: None,
        }
    }
}

pub struct Project {
    pub name: String,
    /// Unique per-session id (runtime-only), used to name this project's autosaves.
    pub id: u64,
    /// On-disk path this project was last saved to / opened from, if any. Drives
    /// "Save" (overwrite) vs "Save As" (prompt). Runtime-only (not serialized).
    pub path: Option<PathBuf>,
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
    /// Undo/redo history of the editable document state.
    pub history: History,
    /// True when the document has unsaved changes (shown as a `*` on the tab).
    pub modified: bool,
    /// Hit-test margin (screen px) for rip handles: corner grab radius, edge
    /// dead-zone around vertices, and move-region inset. Tunable in Edit menu.
    pub cursor_margin: f32,
    /// Live-preview output scale for the perspective warp (0,1]: lower = more
    /// mip downscaling = faster but coarser previews. Tunable in Edit menu.
    /// Runtime-only (not serialized).
    pub preview_quality: f32,
    /// Transient status / error message shown in the chin bar.
    pub status: Option<String>,
    /// True when `status` is an error (rendered on a soft red, selectable bg).
    pub status_error: bool,
    /// Set by the Texture View toolbar's "Add Image" button; `app` picks it up,
    /// shows the file dialog and starts the background load. Runtime-only.
    pub want_add_image: bool,
    /// Background renderer for full-resolution rip outputs (runtime-only).
    pub renderer: crate::render::RipRenderer,
}

impl Project {
    pub fn new(name: impl Into<String>, dock_state: DockState<PanelTab>) -> Self {
        let mut project = Self {
            name: name.into(),
            id: NEXT_PROJECT_ID.fetch_add(1, Ordering::Relaxed),
            path: None,
            dock_state,
            images: Vec::new(),
            active_image: None,
            rips: Vec::new(),
            editor: RipEditor::default(),
            view: ViewState::default(),
            guides: Guides::default(),
            atlas: Atlas::default(),
            atlas_dirty: false,
            history: History::default(),
            // A brand-new, empty project is "clean": the tab shows no `*` until
            // the user actually changes something (e.g. adds an image).
            modified: false,
            cursor_margin: 15.0,
            preview_quality: 0.4,
            status: None,
            status_error: false,
            want_add_image: false,
            renderer: crate::render::RipRenderer::default(),
        };
        project.reset_history();
        project
    }

    /// Sets an informational status message (shown in the chin bar).
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_error = false;
    }

    /// Sets an error status message (shown on a soft red, selectable background).
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_error = true;
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
