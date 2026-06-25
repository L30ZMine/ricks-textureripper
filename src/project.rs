use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Pos2, TextureHandle, Vec2};
use egui_dock::DockState;
use image::RgbaImage;
use serde::{Deserialize, Serialize};

static NEXT_PROJECT_ID: AtomicU64 = AtomicU64::new(1);

use crate::history::History;
use crate::rip_tool::{RipEditor, RipShape};
use crate::ui::docking::PanelTab;

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

    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 0.0 && self.saturation == 0.0
    }
}

pub struct LoadedImage {
    pub name: String,

    pub size: [usize; 2],

    pub texture: TextureHandle,

    pub pixels: RgbaImage,

    pub mips: Vec<RgbaImage>,

    pub original: RgbaImage,

    pub adjust: Adjustments,

    pub dirty: bool,

    pub pos: Vec2,

    pub scale: f32,

    pub source_path: PathBuf,
}

impl LoadedImage {

    pub fn size_vec(&self) -> Vec2 {
        Vec2::new(self.size[0] as f32, self.size[1] as f32)
    }

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

pub struct RipOutput {
    pub size: [usize; 2],
    pub texture: TextureHandle,
    pub pixels: RgbaImage,
}

pub struct Rip {
    pub name: String,

    pub image: usize,

    pub shape: RipShape,

    pub adjust: Adjustments,

    pub resize: Option<[u32; 2]>,

    pub atlas_pos: Option<[f32; 2]>,

    pub dirty: bool,

    pub output: Option<RipOutput>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AspectMode {

    Automatic,

    Square,

    Custom,
}

impl Default for AspectMode {
    fn default() -> Self {
        AspectMode::Automatic
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {

    Automatic,

    Manual,
}

impl Default for SortMode {
    fn default() -> Self {
        SortMode::Automatic
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AtlasSettings {

    pub padding: u32,

    pub export_w: u32,
    pub export_h: u32,

    #[serde(default)]
    pub aspect_mode: AspectMode,

    #[serde(default)]
    pub sort_mode: SortMode,

    #[serde(default)]
    pub snap_enabled: bool,

    #[serde(default = "default_snap_step")]
    pub snap_step: u32,
}

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

pub struct AtlasPlacement {
    pub rip: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct AtlasResult {
    pub size: [usize; 2],
    pub placements: Vec<AtlasPlacement>,
    pub used_count: usize,
}

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

    pub view: AtlasView,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Guides {
    pub enabled: bool,

    pub vertical: u32,

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

pub struct ViewState {

    pub pan: Vec2,

    pub zoom: f32,

    pub dragging_image: Option<usize>,

    pub scaling_image: Option<usize>,

    pub panning: bool,

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

    pub id: u64,

    pub path: Option<PathBuf>,

    pub dock_state: DockState<PanelTab>,

    pub images: Vec<LoadedImage>,

    pub active_image: Option<usize>,

    pub rips: Vec<Rip>,

    pub editor: RipEditor,

    pub view: ViewState,

    pub guides: Guides,

    pub atlas: Atlas,

    pub atlas_dirty: bool,

    pub needs_full: bool,

    pub history: History,

    pub modified: bool,

    pub cursor_margin: f32,

    pub preview_quality: f32,

    pub status: Option<String>,

    pub status_error: bool,
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
            needs_full: false,
            history: History::default(),

            modified: false,
            cursor_margin: 15.0,
            preview_quality: 0.4,
            status: None,
            status_error: false,
        };
        project.reset_history();
        project
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_error = false;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.status_error = true;
    }

    pub fn next_rip_name(&self) -> String {
        format!("rip {}", self.rips.len() + 1)
    }

    pub fn reset_history(&mut self) {
        let now = crate::snapshot::capture(self);
        self.history.reset(now);
    }

    pub fn commit_history_if_changed(&mut self) {
        let now = crate::snapshot::capture(self);
        if self.history.commit(now) {
            self.modified = true;
        }
    }
}
