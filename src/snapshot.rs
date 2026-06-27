//! A serializable snapshot of a project's editable document state.
//!
//! Used both for undo/redo (`history.rs`) and for project files (`proj_io.rs`).
//! Heavy pixel buffers are *not* stored — images are referenced by source path
//! and reloaded; rip outputs are recomputed live from geometry. Selection and
//! active-image are captured/restored but excluded from change detection.

use egui::{Pos2, Vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::project::{Adjustments, AtlasSettings, Guides, LoadedImage, Orientation, Project, Rip};
use crate::rip_tool::{DragHandle, RipShape};

/// Serializable form of a rip's geometry (avoids depending on egui type serde).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum SerShape {
    Quad([[f32; 2]; 4]),
    CurvedQuad {
        corners: [[f32; 2]; 4],
        out_handles: [[f32; 2]; 4],
        in_handles: [[f32; 2]; 4],
    },
    Circle { center: [f32; 2], radius: f32 },
}

impl SerShape {
    fn from_shape(s: &RipShape) -> Self {
        match s {
            RipShape::Quad(c) => SerShape::Quad(arr_pos(c)),
            RipShape::CurvedQuad {
                corners,
                out_handles,
                in_handles,
            } => SerShape::CurvedQuad {
                corners: arr_pos(corners),
                out_handles: arr_vec(out_handles),
                in_handles: arr_vec(in_handles),
            },
            RipShape::Circle { center, radius } => SerShape::Circle {
                center: [center.x, center.y],
                radius: *radius,
            },
        }
    }

    fn to_shape(&self) -> RipShape {
        match self {
            SerShape::Quad(c) => RipShape::Quad(arr_to_pos(c)),
            SerShape::CurvedQuad {
                corners,
                out_handles,
                in_handles,
            } => RipShape::CurvedQuad {
                corners: arr_to_pos(corners),
                out_handles: arr_to_vec(out_handles),
                in_handles: arr_to_vec(in_handles),
            },
            SerShape::Circle { center, radius } => RipShape::Circle {
                center: Pos2::new(center[0], center[1]),
                radius: *radius,
            },
        }
    }
}

fn arr_pos(c: &[Pos2; 4]) -> [[f32; 2]; 4] {
    [
        [c[0].x, c[0].y],
        [c[1].x, c[1].y],
        [c[2].x, c[2].y],
        [c[3].x, c[3].y],
    ]
}

fn arr_vec(v: &[Vec2; 4]) -> [[f32; 2]; 4] {
    [
        [v[0].x, v[0].y],
        [v[1].x, v[1].y],
        [v[2].x, v[2].y],
        [v[3].x, v[3].y],
    ]
}

fn arr_to_pos(c: &[[f32; 2]; 4]) -> [Pos2; 4] {
    [
        Pos2::new(c[0][0], c[0][1]),
        Pos2::new(c[1][0], c[1][1]),
        Pos2::new(c[2][0], c[2][1]),
        Pos2::new(c[3][0], c[3][1]),
    ]
}

fn arr_to_vec(v: &[[f32; 2]; 4]) -> [Vec2; 4] {
    [
        Vec2::new(v[0][0], v[0][1]),
        Vec2::new(v[1][0], v[1][1]),
        Vec2::new(v[2][0], v[2][1]),
        Vec2::new(v[3][0], v[3][1]),
    ]
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageState {
    pub source_path: PathBuf,
    pub name: String,
    pub pos: [f32; 2],
    pub size: [usize; 2],
    pub adjust: Adjustments,
    /// Display-only Texture View scale (older files default to 1.0).
    #[serde(default = "default_scale")]
    pub scale: f32,
}

fn default_scale() -> f32 {
    1.0
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct RipState {
    pub name: String,
    pub image: usize,
    pub shape: SerShape,
    pub adjust: Adjustments,
    /// Rotation / mirroring of the rip output (older files default to none).
    #[serde(default)]
    pub orient: Orientation,
    pub resize: Option<[u32; 2]>,
    /// Manual atlas placement (top-left, atlas px); `None` in Automatic sort.
    #[serde(default)]
    pub atlas_pos: Option<[f32; 2]>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub images: Vec<ImageState>,
    pub rips: Vec<RipState>,
    pub atlas: AtlasSettings,
    pub guides: Guides,
    pub active_image: Option<usize>,
    pub selected: Option<usize>,
}

impl ProjectSnapshot {
    /// Compares only the document (ignores selection/active-image), so that
    /// selecting or panning doesn't register as an undoable change.
    pub fn same_document(&self, other: &Self) -> bool {
        self.images == other.images
            && self.rips == other.rips
            && self.atlas == other.atlas
            && self.guides == other.guides
    }
}

/// Captures the current editable state of `project`.
pub fn capture(project: &Project) -> ProjectSnapshot {
    ProjectSnapshot {
        images: project
            .images
            .iter()
            .map(|img| ImageState {
                source_path: img.source_path.clone(),
                name: img.name.clone(),
                pos: [img.pos.x, img.pos.y],
                size: img.size,
                adjust: img.adjust,
                scale: img.scale,
            })
            .collect(),
        rips: project
            .rips
            .iter()
            .map(|r| RipState {
                name: r.name.clone(),
                image: r.image,
                shape: SerShape::from_shape(&r.shape),
                adjust: r.adjust,
                orient: r.orient,
                resize: r.resize,
                atlas_pos: r.atlas_pos,
            })
            .collect(),
        atlas: project.atlas.settings,
        guides: project.guides.clone(),
        active_image: project.active_image,
        selected: project.editor.selected,
    }
}

/// Restores `snap` into `project`, reusing already-loaded images (matched by
/// source path) and reloading any that are missing. Rips and images are marked
/// dirty so their outputs recompute live.
pub fn restore(ctx: &egui::Context, project: &mut Project, snap: &ProjectSnapshot) {
    let mut existing = std::mem::take(&mut project.images);
    let mut images: Vec<LoadedImage> = Vec::with_capacity(snap.images.len());
    for st in &snap.images {
        if let Some(pos) = existing.iter().position(|im| im.source_path == st.source_path) {
            let mut img = existing.remove(pos);
            img.name = st.name.clone();
            img.pos = Vec2::new(st.pos[0], st.pos[1]);
            img.size = st.size;
            img.adjust = st.adjust;
            img.scale = st.scale;
            img.dirty = true;
            images.push(img);
        } else if let Ok(mut img) = crate::texture_view::load_loaded_image(ctx, &st.source_path) {
            img.name = st.name.clone();
            img.pos = Vec2::new(st.pos[0], st.pos[1]);
            img.size = st.size;
            img.adjust = st.adjust;
            img.scale = st.scale;
            img.dirty = true;
            images.push(img);
        } else {
            project.set_error(format!("Could not reload {}", st.source_path.display()));
        }
    }
    project.images = images;

    project.rips = snap
        .rips
        .iter()
        .map(|rs| Rip {
            name: rs.name.clone(),
            image: rs.image,
            shape: rs.shape.to_shape(),
            adjust: rs.adjust,
            orient: rs.orient,
            resize: rs.resize,
            atlas_pos: rs.atlas_pos,
            dirty: true,
            previewed: false,
            output: None,
        })
        .collect();

    project.atlas.settings = snap.atlas;
    project.atlas.result = None;
    project.guides = snap.guides.clone();
    project.active_image = snap.active_image.filter(|&i| i < project.images.len());
    project.editor.selected = snap.selected.filter(|&i| i < project.rips.len());
    project.editor.drag = DragHandle::None;
    project.atlas_dirty = true;
}
