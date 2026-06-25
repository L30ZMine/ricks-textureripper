//! Phase 5 — the Image Edit panel plus the non-destructive adjustment pipeline.
//!
//! Brightness / contrast / saturation adjustments and an optional resize can be
//! applied to either the active source image or the selected rip. Image
//! adjustments rebuild the working pixels from a kept original; rip adjustments
//! are folded into the live rip-extraction pass (see `rip_tool::recompute_dirty`).

use image::imageops::FilterType;
use image::RgbaImage;

use crate::project::{Adjustments, Project};
use crate::rip_tool::RipShape;
use crate::texture_view::upload_texture;

// ---------------------------------------------------------------------------
// Pixel pipeline
// ---------------------------------------------------------------------------

/// Applies brightness/contrast/saturation in place. No-op when identity. Alpha
/// is left untouched.
pub fn apply_adjustments(img: &mut RgbaImage, adj: &Adjustments) {
    if adj.is_identity() {
        return;
    }
    let bright = adj.brightness * 255.0;
    let contrast = 1.0 + adj.contrast; // 0 = flat gray, 1 = unchanged, 2 = double
    let sat = 1.0 + adj.saturation; // 0 = grayscale, 1 = unchanged, 2 = vivid

    for px in img.pixels_mut() {
        let mut c = [px[0] as f32, px[1] as f32, px[2] as f32];
        // Contrast around mid-gray, then brightness.
        for v in &mut c {
            *v = (*v - 128.0) * contrast + 128.0 + bright;
        }
        // Saturation: lerp each channel toward perceived luminance.
        let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        for v in &mut c {
            *v = l + (*v - l) * sat;
        }
        px[0] = c[0].clamp(0.0, 255.0) as u8;
        px[1] = c[1].clamp(0.0, 255.0) as u8;
        px[2] = c[2].clamp(0.0, 255.0) as u8;
    }
}

/// Returns `img` resized to `target` (clones unchanged when already that size).
pub fn resize_to(img: &RgbaImage, target: [u32; 2]) -> RgbaImage {
    if img.width() == target[0] && img.height() == target[1] {
        img.clone()
    } else {
        image::imageops::resize(img, target[0].max(1), target[1].max(1), FilterType::Triangle)
    }
}

/// Builds a downscale mip chain (each ~half the previous, a few levels) for a
/// source image, so live previews can sample a smaller, anti-aliased copy.
pub fn build_mips(base: &RgbaImage) -> Vec<RgbaImage> {
    let mut mips = Vec::new();
    let (mut w, mut h) = (base.width(), base.height());
    while w > 64 && h > 64 && mips.len() < 4 {
        w /= 2;
        h /= 2;
        mips.push(image::imageops::resize(
            base,
            w.max(1),
            h.max(1),
            FilterType::Triangle,
        ));
    }
    mips
}

/// Rebuilds every dirty image's working pixels (resize + adjustments) from its
/// kept original, re-uploads the texture, and marks dependent rips for re-extract.
pub fn recompute_dirty_images(ctx: &egui::Context, project: &mut Project) {
    let mut changed: Vec<usize> = Vec::new();
    for (idx, img) in project.images.iter_mut().enumerate() {
        if !img.dirty {
            continue;
        }
        img.dirty = false;
        let mut work = resize_to(&img.original, [img.size[0] as u32, img.size[1] as u32]);
        apply_adjustments(&mut work, &img.adjust);
        img.texture = upload_texture(ctx, &img.name, &work);
        img.mips = build_mips(&work);
        img.pixels = work;
        changed.push(idx);
    }
    if !changed.is_empty() {
        for rip in project.rips.iter_mut() {
            if changed.contains(&rip.image) {
                rip.dirty = true;
            }
        }
    }
}

/// Scales the geometry of every rip on `image_idx` (used when the source image
/// is resized so selections stay locked to the same image features).
fn scale_rips_on_image(project: &mut Project, image_idx: usize, sx: f32, sy: f32) {
    for rip in project.rips.iter_mut() {
        if rip.image != image_idx {
            continue;
        }
        match &mut rip.shape {
            RipShape::Quad(c) => {
                for p in c.iter_mut() {
                    p.x *= sx;
                    p.y *= sy;
                }
            }
            RipShape::Circle { center, radius } => {
                center.x *= sx;
                center.y *= sy;
                *radius *= (sx + sy) * 0.5;
            }
        }
        rip.dirty = true;
    }
}

// ---------------------------------------------------------------------------
// Panel UI
// ---------------------------------------------------------------------------

/// At or above this panel width the editor splits into two equal columns
/// (sliders on the left, the remaining tools on the right); below it everything
/// stacks in one column.
const WIDE_THRESHOLD: f32 = 470.0;

/// Draws the Image Edit panel. Edits the selected rip if one is selected,
/// otherwise the active image. No title/heading — the panel is title-only via
/// the dock tab.
pub fn ui(ui: &mut egui::Ui, project: &mut Project) {
    let wide = ui.available_width() >= WIDE_THRESHOLD;
    let rip_target = project.editor.selected.filter(|&i| i < project.rips.len());
    if let Some(ri) = rip_target {
        rip_editor(ui, project, ri, wide);
    } else if let Some(ii) = project.active_image.filter(|&i| i < project.images.len()) {
        image_editor(ui, project, ii, wide);
    } else {
        ui.weak("Select an image or a rip to edit.");
    }
}

/// Three adjustment sliders bound to `adj`; returns true if any changed.
fn adjustment_sliders(ui: &mut egui::Ui, adj: &mut Adjustments) -> bool {
    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut adj.brightness, -1.0..=1.0).text("Brightness"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut adj.contrast, -1.0..=1.0).text("Contrast"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut adj.saturation, -1.0..=1.0).text("Saturation"))
        .changed();
    changed
}

fn rip_editor(ui: &mut egui::Ui, project: &mut Project, ri: usize, wide: bool) {
    let dirty = if wide {
        ui.columns(2, |c| {
            let mut d = adjustment_sliders(&mut c[0], &mut project.rips[ri].adjust);
            d |= rip_tools(&mut c[1], &mut project.rips[ri]);
            d
        })
    } else {
        let mut d = adjustment_sliders(ui, &mut project.rips[ri].adjust);
        ui.separator();
        d |= rip_tools(ui, &mut project.rips[ri]);
        d
    };

    if dirty {
        project.rips[ri].dirty = true;
    }
}

/// The non-slider rip controls (output-size override + reset). Returns dirty.
fn rip_tools(ui: &mut egui::Ui, rip: &mut crate::project::Rip) -> bool {
    let mut dirty = false;

    let natural = rip.output.as_ref().map(|o| o.size);
    let mut resize = rip.resize;
    let mut custom = resize.is_some();
    if ui.checkbox(&mut custom, "Custom output size").changed() {
        resize = if custom {
            let base = natural.unwrap_or([256, 256]);
            Some([base[0] as u32, base[1] as u32])
        } else {
            None
        };
        dirty = true;
    }
    if let Some(sz) = &mut resize {
        ui.horizontal(|ui| {
            ui.label("Width");
            dirty |= ui
                .add(egui::DragValue::new(&mut sz[0]).range(1..=8192).suffix(" px"))
                .changed();
            ui.label("Height");
            dirty |= ui
                .add(egui::DragValue::new(&mut sz[1]).range(1..=8192).suffix(" px"))
                .changed();
        });
    } else if let Some(n) = natural {
        ui.weak(format!("Output: {}×{} px", n[0], n[1]));
    }
    rip.resize = resize;

    ui.separator();
    if ui.button("Reset adjustments").clicked() {
        rip.adjust = Adjustments::default();
        rip.resize = None;
        dirty = true;
    }

    dirty
}

fn image_editor(ui: &mut egui::Ui, project: &mut Project, ii: usize, wide: bool) {
    let dirty = if wide {
        ui.columns(2, |c| {
            let mut d = adjustment_sliders(&mut c[0], &mut project.images[ii].adjust);
            d |= image_tools(&mut c[1], project, ii);
            d
        })
    } else {
        let mut d = adjustment_sliders(ui, &mut project.images[ii].adjust);
        ui.separator();
        d |= image_tools(ui, project, ii);
        d
    };

    if dirty {
        project.images[ii].dirty = true;
    }
}

/// The non-slider image controls (resize + reset). Returns dirty. Rips on this
/// image are rescaled so they stay locked to the same image features.
fn image_tools(ui: &mut egui::Ui, project: &mut Project, ii: usize) -> bool {
    let mut dirty = false;

    let orig = (
        project.images[ii].original.width() as usize,
        project.images[ii].original.height() as usize,
    );
    let old = project.images[ii].size;
    let mut w = old[0];
    let mut h = old[1];
    ui.horizontal(|ui| {
        ui.label("Width");
        ui.add(egui::DragValue::new(&mut w).range(1..=16384).suffix(" px"));
        ui.label("Height");
        ui.add(egui::DragValue::new(&mut h).range(1..=16384).suffix(" px"));
    });
    ui.weak(format!("Original: {}×{} px", orig.0, orig.1));
    if ui.button("Reset size").clicked() {
        w = orig.0;
        h = orig.1;
    }

    // Commit each frame so a drag accumulates; rips are rescaled by the
    // incremental ratio to stay locked to the same image features.
    if w != old[0] || h != old[1] {
        let sx = w as f32 / old[0] as f32;
        let sy = h as f32 / old[1] as f32;
        scale_rips_on_image(project, ii, sx, sy);
        project.images[ii].size = [w, h];
        dirty = true;
    }

    if ui.button("Reset adjustments").clicked() {
        project.images[ii].adjust = Adjustments::default();
        dirty = true;
    }

    dirty
}
