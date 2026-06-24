//! Atlas packing: bin-packs all rip outputs into a single texture using
//! `rectangle-pack`, composites them, and exposes packing settings. Repacks
//! live whenever `Project::atlas_dirty` is set.

use std::collections::{BTreeMap, HashMap};

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use image::RgbaImage;
use rectangle_pack::{
    contains_smallest_box, pack_rects, volume_heuristic, GroupedRectsToPlace, RectToInsert,
    TargetBin,
};

use crate::project::{AtlasPlacement, AtlasResult, Project};

/// Bin size used for packing. The atlas grows only as large as the rips need;
/// this is just a generous upper bound so packing never fails on size.
const MAX_BIN: u32 = 16384;

/// Repacks the atlas if it has been marked dirty.
pub fn repack_if_needed(project: &mut Project) {
    if project.atlas_dirty {
        repack(project);
        project.atlas_dirty = false;
    }
}

/// Packs all rip outputs into the atlas.
pub fn repack(project: &mut Project) {
    let Project {
        rips,
        atlas,
        status,
        ..
    } = project;

    let settings = atlas.settings;

    // Collect rips that currently have an output: (rip index, w, h).
    let items: Vec<(usize, u32, u32)> = rips
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.output.as_ref().map(|o| (i, o.size[0] as u32, o.size[1] as u32)))
        .collect();

    if items.is_empty() {
        atlas.result = None;
        return;
    }

    // Each rect is inflated by `padding` so neighbours are spaced apart.
    let pad = settings.padding;
    let mut rects = GroupedRectsToPlace::<usize, ()>::new();
    for &(i, w, h) in &items {
        rects.push_rect(i, None, RectToInsert::new(w + pad, h + pad, 1));
    }

    let mut bins = BTreeMap::new();
    bins.insert((), TargetBin::new(MAX_BIN, MAX_BIN, 1));

    let packed = match pack_rects(&rects, &mut bins, &volume_heuristic, &contains_smallest_box) {
        Ok(p) => p,
        Err(_) => {
            *status = Some("Atlas: too many rips to pack.".to_string());
            atlas.result = None;
            return;
        }
    };

    let size_of: HashMap<usize, (u32, u32)> =
        items.iter().map(|&(i, w, h)| (i, (w, h))).collect();

    let mut placements = Vec::new();
    let mut aw = 0u32;
    let mut ah = 0u32;
    for (&i, (_bin, loc)) in packed.packed_locations() {
        let (w, h) = size_of[&i];
        let (x, y) = (loc.x(), loc.y());
        aw = aw.max(x + w);
        ah = ah.max(y + h);
        placements.push(AtlasPlacement { rip: i, x, y, w, h });
    }

    // Composite the rips into the atlas image.
    let mut atlas_img = RgbaImage::new(aw.max(1), ah.max(1));
    for p in &placements {
        if let Some(out) = rips[p.rip].output.as_ref() {
            image::imageops::overlay(&mut atlas_img, &out.pixels, p.x as i64, p.y as i64);
        }
    }

    atlas.result = Some(AtlasResult {
        size: [aw as usize, ah as usize],
        pixels: atlas_img,
        placements,
        used_count: items.len(),
    });
}

/// Effective export resolution: the stored override, or the natural packed size
/// when unset (`0`).
pub fn export_size(project: &Project) -> Option<[u32; 2]> {
    project.atlas.result.as_ref().map(|res| {
        let s = project.atlas.settings;
        let w = if s.export_w == 0 { res.size[0] as u32 } else { s.export_w };
        let h = if s.export_h == 0 { res.size[1] as u32 } else { s.export_h };
        [w.max(1), h.max(1)]
    })
}

/// Atlas View panel UI: settings, actions, and a preview of the packed atlas.
pub fn ui(ui: &mut egui::Ui, project: &mut Project) {
    // Natural (packed) size, snapshotted before we mutably borrow settings.
    let natural = project
        .atlas
        .result
        .as_ref()
        .map(|r| (r.size[0] as u32, r.size[1] as u32));

    // One row: the packing/export options on the left, the actions and size
    // readout pushed to the right.
    ui.horizontal(|ui| {
        // --- Left: options ---
        ui.label("Padding");
        if ui
            .add(
                egui::DragValue::new(&mut project.atlas.settings.padding)
                    .range(0..=128)
                    .suffix(" px"),
            )
            .changed()
        {
            project.atlas_dirty = true;
        }

        // Export resolution: aspect-locked to the packed atlas (edit one, the
        // other follows). Defaults to the natural packed size.
        if let Some((nat_w, nat_h)) = natural {
            ui.separator();
            let aspect = nat_w as f32 / nat_h.max(1) as f32;
            let s = &mut project.atlas.settings;
            let mut w = if s.export_w == 0 { nat_w } else { s.export_w };
            let mut h = if s.export_h == 0 { nat_h } else { s.export_h };
            ui.label("Width");
            if ui
                .add(egui::DragValue::new(&mut w).range(1..=16384).suffix(" px"))
                .changed()
            {
                s.export_w = w.max(1);
                s.export_h = ((w as f32 / aspect).round() as u32).max(1);
            }
            ui.label("Height");
            if ui
                .add(egui::DragValue::new(&mut h).range(1..=16384).suffix(" px"))
                .changed()
            {
                s.export_h = h.max(1);
                s.export_w = ((h as f32 * aspect).round() as u32).max(1);
            }
        }

        // --- Right: actions + size info (laid out right-to-left) ---
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let (Some(res), Some([ew, eh])) = (&project.atlas.result, export_size(project)) {
                ui.label(format!("{}×{} px, {} rip(s)", ew, eh, res.used_count));
            } else {
                ui.weak("no atlas (add some rips)");
            }
            if ui.button("Export PNG…").clicked() {
                export(project);
            }
        });
    });

    ui.separator();

    preview(ui, project);
}

/// Draws the packed-atlas preview, with a live resize handle on the selected rip
/// (drag the bottom-right grip to set that rip's custom output size).
fn preview(ui: &mut egui::Ui, project: &mut Project) {
    let selected = project.editor.selected;
    let mut new_resize: Option<(usize, [u32; 2])> = None;
    let mut new_select: Option<usize> = None;

    if let Some(res) = &project.atlas.result {
        let (w, h) = (res.size[0] as f32, res.size[1] as f32);
        let avail_w = ui.available_width().max(64.0);
        let max_h = 360.0;
        let scale = (avail_w / w).min(max_h / h).min(8.0);
        let size = Vec2::new(w * scale, h * scale);

        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        // Dark backing so transparent areas read clearly.
        painter.rect_filled(rect, 0.0, Color32::from_gray(40));
        // Draw each rip's own texture stretched into its placement cell, rather
        // than the composited atlas texture. During a live drag the preview
        // pixels are low-res (small), so the GPU stretch keeps every cell filled
        // instead of leaving the image pinned in the cell's top-left corner.
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for p in &res.placements {
            if let Some(out) = project.rips.get(p.rip).and_then(|r| r.output.as_ref()) {
                let cell = Rect::from_min_size(
                    rect.min + Vec2::new(p.x as f32 * scale, p.y as f32 * scale),
                    Vec2::new(p.w as f32 * scale, p.h as f32 * scale),
                );
                painter.image(out.texture.id(), cell, uv, Color32::WHITE);
            }
        }

        // The selected rip's placement (for the resize handle).
        let sel_placement = res.placements.iter().find(|p| selected == Some(p.rip));
        let handle_rect = sel_placement.map(|p| {
            let br = rect.min + Vec2::new((p.x + p.w) as f32 * scale, (p.y + p.h) as f32 * scale);
            Rect::from_center_size(br, Vec2::splat(12.0))
        });

        // Outline each packed rip; the selected one is highlighted.
        for p in &res.placements {
            let r = Rect::from_min_size(
                rect.min + Vec2::new(p.x as f32 * scale, p.y as f32 * scale),
                Vec2::new(p.w as f32 * scale, p.h as f32 * scale),
            );
            let is_sel = selected == Some(p.rip);
            let stroke = if is_sel {
                Stroke::new(2.0, Color32::from_rgb(255, 120, 0))
            } else {
                Stroke::new(1.0, Color32::from_rgb(255, 180, 0))
            };
            painter.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
        }

        // Resize handle (drag the bottom-right grip of the selected rip).
        if let (Some(p), Some(hr)) = (sel_placement, handle_rect) {
            painter.rect_filled(hr, 1.0, Color32::WHITE);
            painter.rect_stroke(
                hr,
                1.0,
                Stroke::new(1.0, Color32::from_gray(40)),
                StrokeKind::Middle,
            );

            // Track whether the active drag grabbed the handle (across frames).
            let drag_id = ui.id().with("atlas_resize_drag");
            let mut grabbing = ui.data(|d| d.get_temp::<bool>(drag_id).unwrap_or(false));
            if response.drag_started() {
                grabbing = response
                    .interact_pointer_pos()
                    .is_some_and(|pt| hr.contains(pt));
            }
            if response.drag_stopped() {
                grabbing = false;
            }
            ui.data_mut(|d| d.insert_temp(drag_id, grabbing));

            if grabbing && response.dragged() {
                if let Some(pt) = response.interact_pointer_pos() {
                    // Clamp to a sane range so dragging past the edges (or to a
                    // sub-pixel size) doesn't make the cell — and the whole
                    // auto-scaled preview — jump around.
                    let nw = ((pt.x - rect.min.x) / scale - p.x as f32).round().clamp(1.0, 8192.0) as u32;
                    let nh = ((pt.y - rect.min.y) / scale - p.y as f32).round().clamp(1.0, 8192.0) as u32;
                    new_resize = Some((p.rip, [nw, nh]));
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            } else if response.hover_pos().is_some_and(|pt| hr.contains(pt)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
        }

        // Clicking a packed rip in the preview selects it (top-most cell wins).
        if response.clicked() {
            if let Some(pt) = response.interact_pointer_pos() {
                for p in res.placements.iter().rev() {
                    let cell = Rect::from_min_size(
                        rect.min + Vec2::new(p.x as f32 * scale, p.y as f32 * scale),
                        Vec2::new(p.w as f32 * scale, p.h as f32 * scale),
                    );
                    if cell.contains(pt) {
                        new_select = Some(p.rip);
                        break;
                    }
                }
            }
        }
    }

    if let Some(rip) = new_select {
        if rip < project.rips.len() {
            project.editor.selected = Some(rip);
            project.active_image = Some(project.rips[rip].image);
        }
    }

    if let Some((rip_idx, sz)) = new_resize {
        if rip_idx < project.rips.len() {
            project.rips[rip_idx].resize = Some(sz);
            project.rips[rip_idx].dirty = true;
            project.atlas_dirty = true;
        }
    }
}

/// Saves the packed atlas (scaled to the export resolution) to a PNG.
pub fn export(project: &mut Project) {
    let outcome: Option<Result<String, String>> =
        if let (Some(res), Some(size)) = (&project.atlas.result, export_size(project)) {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Export Atlas")
                .add_filter("PNG", &["png"])
                .set_file_name("atlas.png")
                .save_file()
            {
                let img = crate::image_edit::resize_to(&res.pixels, size);
                Some(
                    img.save(&path)
                        .map_err(|e| e.to_string())
                        .map(|_| path.display().to_string()),
                )
            } else {
                None // cancelled
            }
        } else {
            Some(Err("No atlas to export.".to_string()))
        };

    if let Some(result) = outcome {
        project.status = Some(match result {
            Ok(p) => format!("Exported atlas to {p}"),
            Err(e) => format!("Export failed: {e}"),
        });
    }
}
