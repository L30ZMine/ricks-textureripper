use std::path::Path;

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::project::{LoadedImage, Project, ViewState};
use crate::rip_tool::{self, DragHandle, RipShape, Xform};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "gif", "tga", "tiff", "tif", "webp", "ico", "dds",
];

pub fn open_add_image_dialog(ctx: &egui::Context, project: &mut Project) {
    let picked = rfd::FileDialog::new()
        .set_title("Add Image")
        .add_filter("Images", SUPPORTED_EXTENSIONS)
        .pick_files();

    let Some(paths) = picked else {
        return;
    };

    for path in paths {
        match load_image(ctx, project, &path) {
            Ok(name) => project.set_status(format!("Loaded {name}")),
            Err(e) => project.set_error(format!("Failed to load {}: {e}", path.display())),
        }
    }
}

pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.as_str()))
}

pub fn add_image_path(ctx: &egui::Context, project: &mut Project, path: &Path) {
    match load_image(ctx, project, path) {
        Ok(name) => project.set_status(format!("Loaded {name}")),
        Err(e) => project.set_error(format!("Failed to load {}: {e}", path.display())),
    }
}

pub fn paint_checkerboard_clipped(
    painter: &egui::Painter,
    rect: Rect,
    clip: Rect,
    phase: Pos2,
    dark: bool,
) {
    let vis = rect.intersect(clip);
    if vis.width() <= 0.0 || vis.height() <= 0.0 {
        return;
    }
    let (a, b) = if dark {
        (Color32::from_gray(38), Color32::from_gray(52))
    } else {
        (Color32::from_gray(190), Color32::from_gray(225))
    };
    painter.rect_filled(vis, 0.0, a);
    let cell = 16.0;
    let i0 = ((vis.min.x - phase.x) / cell).floor() as i32;
    let i1 = ((vis.max.x - phase.x) / cell).ceil() as i32;
    let j0 = ((vis.min.y - phase.y) / cell).floor() as i32;
    let j1 = ((vis.max.y - phase.y) / cell).ceil() as i32;
    for j in j0..j1 {
        for i in i0..i1 {
            if (i + j).rem_euclid(2) == 0 {
                continue;
            }
            let min = phase + Vec2::new(i as f32 * cell, j as f32 * cell);
            let r = Rect::from_min_size(min, Vec2::splat(cell)).intersect(vis);
            if r.width() > 0.0 && r.height() > 0.0 {
                painter.rect_filled(r, 0.0, b);
            }
        }
    }
}

pub fn upload_texture(
    ctx: &egui::Context,
    name: &str,
    rgba: &image::RgbaImage,
) -> egui::TextureHandle {
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

pub fn loaded_image_from_pixels(
    ctx: &egui::Context,
    name: String,
    source_path: std::path::PathBuf,
    original: image::RgbaImage,
) -> LoadedImage {
    let size = [original.width() as usize, original.height() as usize];
    let texture = upload_texture(ctx, &name, &original);
    let mips = crate::image_edit::build_mips(&original);
    LoadedImage {
        name,
        size,
        texture,
        mips,
        original: original.clone(),
        adjust: crate::project::Adjustments::default(),
        dirty: false,
        pixels: original,
        pos: Vec2::ZERO,
        scale: 1.0,
        source_path,
    }
}

pub fn load_loaded_image(ctx: &egui::Context, path: &Path) -> Result<LoadedImage, String> {
    let dynimg = image::open(path).map_err(|e| e.to_string())?;
    let rgba = dynimg.to_rgba8();

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());

    Ok(loaded_image_from_pixels(ctx, name, path.to_path_buf(), rgba))
}

fn load_image(ctx: &egui::Context, project: &mut Project, path: &Path) -> Result<String, String> {
    let mut img = load_loaded_image(ctx, path)?;
    let offset = project.images.len() as f32 * 32.0;
    img.pos = Vec2::new(offset, offset);
    let name = img.name.clone();
    let idx = project.images.len();
    project.images.push(img);
    project.active_image = Some(idx);

    project.modified = true;

    Ok(name)
}

pub fn remove_image(project: &mut Project, idx: usize) {
    if idx >= project.images.len() {
        return;
    }
    project.images.remove(idx);

    let mut keep = Vec::with_capacity(project.rips.len());
    for mut rip in project.rips.drain(..) {
        if rip.image == idx {
            continue;
        }
        if rip.image > idx {
            rip.image -= 1;
        }
        keep.push(rip);
    }
    project.rips = keep;

    project.active_image = match project.active_image {
        Some(a) if a == idx => None,
        Some(a) if a > idx => Some(a - 1),
        other => other,
    };
    if project.active_image.is_none() && !project.images.is_empty() {
        project.active_image = Some(project.images.len() - 1);
    }
    project.editor.selected = project
        .editor
        .selected
        .filter(|&s| s < project.rips.len());

    project.atlas_dirty = true;
    project.modified = true;
    project.set_status("Image removed.");
}

pub fn remove_rip(project: &mut Project, idx: usize) {
    if idx >= project.rips.len() {
        return;
    }
    project.rips.remove(idx);
    project.editor.selected = None;
    project.atlas_dirty = true;
    project.modified = true;
    project.set_status("Rip removed.");
}

pub fn ui(ui: &mut egui::Ui, project: &mut Project) {
    toolbar(ui, project);

    ui.separator();
    canvas(ui, project);
}

fn toolbar(ui: &mut egui::Ui, project: &mut Project) {
    let mut remove_active_image: Option<usize> = None;
    let mut delete: Option<usize> = None;
    ui.horizontal(|ui| {
        if ui.button("Add Image").clicked() {
            open_add_image_dialog(ui.ctx(), project);
        }
        if ui.button("Add Rip").clicked() {
            rip_tool::add_rip(project);
        }

        let active = project.active_image.filter(|&i| i < project.images.len());
        if ui
            .add_enabled(active.is_some(), egui::Button::new("Remove Image"))
            .clicked()
        {
            remove_active_image = active;
        }

        let sel_rip = project.editor.selected.filter(|&s| s < project.rips.len());
        if ui
            .add_enabled(sel_rip.is_some(), egui::Button::new("Remove Rip"))
            .clicked()
        {
            delete = sel_rip;
        }
        ui.separator();
        ui.label(format!("{} image(s)", project.images.len()));
        ui.label(format!("{} rip(s)", project.rips.len()));
        ui.separator();
        if ui.button("Reset view").clicked() {
            project.view.pan = Vec2::ZERO;
            project.view.zoom = 1.0;
        }

        let scale_target = project.active_image.filter(|&i| i < project.images.len());
        let can_reset_scale = scale_target.is_some_and(|i| project.images[i].scale != 1.0);
        if ui
            .add_enabled(can_reset_scale, egui::Button::new("Reset scale"))
            .clicked()
        {
            if let Some(i) = scale_target {
                project.images[i].scale = 1.0;
                project.modified = true;
            }
        }
        ui.label(format!("{:.0}%", project.view.zoom * 100.0));
    });
    if let Some(idx) = remove_active_image {
        remove_image(project, idx);
    }

    if let Some(sel) = project.editor.selected {
        if sel < project.rips.len() {
            ui.horizontal(|ui| {
                let rip = &mut project.rips[sel];
                ui.label("Shape:");
                let is_quad = matches!(rip.shape, RipShape::Quad(_));
                if ui.selectable_label(is_quad, "Quad (perspective)").clicked() {
                    rip_tool::set_shape_quad(rip);
                }
                if ui.selectable_label(!is_quad, "Circle").clicked() {
                    rip_tool::set_shape_circle(rip);
                }
            });
        }
    }

    if let Some(i) = delete {
        remove_rip(project, i);
    }
}

fn canvas(ui: &mut egui::Ui, project: &mut Project) {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    paint_checkerboard_clipped(&painter, rect, rect, Pos2::ZERO, dark);

    let margin = project.cursor_margin;
    let Project {
        images,
        view,
        rips,
        editor,
        active_image,
        guides,
        ..
    } = project;

    if let Some(prev) = view.last_origin {
        let shift = rect.min - prev;
        if shift != Vec2::ZERO {
            view.pan -= shift;
        }
    }
    view.last_origin = Some(rect.min);

    let guide_btn_rect = Rect::from_min_size(
        Pos2::new(rect.right() - 34.0, rect.top() + 8.0),
        Vec2::splat(26.0),
    );
    let guide_btn = ui.interact(guide_btn_rect, ui.id().with("guide_toggle"), Sense::click());
    if guide_btn.clicked() {
        guides.enabled = !guides.enabled;
    }
    let over_guide_btn = guide_btn
        .hover_pos()
        .map_or(false, |p| guide_btn_rect.contains(p));

    if response.hovered() {
        let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
        let pinch = ui.input(|i| i.zoom_delta());
        let factor = if pinch != 1.0 {
            pinch
        } else if scroll_y != 0.0 {
            (scroll_y * 0.0015).exp()
        } else {
            1.0
        };
        if factor != 1.0 {
            if let Some(ptr) = response.hover_pos() {
                let new_zoom = (view.zoom * factor).clamp(0.05, 40.0);
                let origin = rect.min.to_vec2();
                let world = (ptr.to_vec2() - origin - view.pan) / view.zoom;
                view.pan = ptr.to_vec2() - origin - world * new_zoom;
                view.zoom = new_zoom;
            }
        }
    }

    let over_guide = response
        .interact_pointer_pos()
        .is_some_and(|p| guide_btn_rect.contains(p));

    if response.drag_started_by(egui::PointerButton::Primary) && !over_guide {
        editor.drag = DragHandle::None;
        view.dragging_image = None;
        view.scaling_image = None;
        view.panning = false;

        let press = ui
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if ui.input(|i| i.modifiers.shift) {

            view.dragging_image = press.and_then(|p| topmost_image_at(images, view, rect, p));
            view.panning = view.dragging_image.is_none();
        } else {

            let mut grabbed_rip = false;
            if let (Some(ptr), Some(sel)) = (press, editor.selected) {
                if sel < rips.len() && rips[sel].image < images.len() {
                    let x = make_xform(rect.min, view, images[rips[sel].image].pos, images[rips[sel].image].scale);

                    let handle = rip_tool::hit_handle(&rips[sel], &x, ptr, margin)
                        .unwrap_or(editor.hover_handle);
                    if handle != DragHandle::None {
                        editor.drag = handle;
                        grabbed_rip = true;
                    }
                }
            }
            if !grabbed_rip {
                if let Some(ai) =
                    press.and_then(|p| scale_grip_at(*active_image, images, view, rect, p, margin))
                {

                    view.scaling_image = Some(ai);
                }
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(ai) = view.scaling_image.filter(|&i| i < images.len()) {
            if let Some(ptr) = response.interact_pointer_pos() {
                apply_image_scale(&mut images[ai], rect.min, view, ptr);
            }
        } else if editor.is_dragging() {
            if let Some(sel) = editor.selected {
                if sel < rips.len() && rips[sel].image < images.len() {
                    let x = make_xform(rect.min, view, images[rips[sel].image].pos, images[rips[sel].image].scale);
                    if let Some(ptr) = response.interact_pointer_pos() {
                        rip_tool::apply(&mut rips[sel], editor.drag, &x, ptr, response.drag_delta());
                    }
                }
            }
        } else if let Some(idx) = view.dragging_image.filter(|&i| i < images.len()) {
            images[idx].pos += response.drag_delta() / view.zoom;
        } else if view.panning {
            view.pan += response.drag_delta();
        }
    }

    if response.dragged_by(egui::PointerButton::Middle) {
        view.pan += response.drag_delta();
    }

    if response.drag_stopped() {
        editor.drag = DragHandle::None;
        view.dragging_image = None;
        view.scaling_image = None;
        view.panning = false;
    }

    if response.clicked() {
        if let Some(ptr) = response.interact_pointer_pos() {
            if !guide_btn_rect.contains(ptr) {
                let mut hit_rip = None;
                for i in (0..rips.len()).rev() {
                    if rips[i].image < images.len() {
                        let x = make_xform(rect.min, view, images[rips[i].image].pos, images[rips[i].image].scale);
                        if rip_tool::contains_point(&rips[i], &x, ptr) {
                            hit_rip = Some(i);
                            break;
                        }
                    }
                }
                match hit_rip {
                    Some(i) => {
                        editor.selected = Some(i);
                        *active_image = Some(rips[i].image);
                    }
                    None => {
                        editor.selected = None;

                        if let Some(idx) = topmost_image_at(images, view, rect, ptr) {
                            *active_image = Some(idx);
                        }
                    }
                }
            }
        }
    }

    editor.hover_handle = DragHandle::None;
    if response.dragged_by(egui::PointerButton::Middle) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if view.scaling_image.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    } else if editor.is_dragging() {
        ui.ctx().set_cursor_icon(rip_tool::handle_cursor(editor.drag));
    } else if view.dragging_image.is_some() || view.panning {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if response.hovered() && !over_guide_btn {
        let hover = response.hover_pos();
        if ui.input(|i| i.modifiers.shift) {

            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else {

            let mut over_handle = false;
            if let (Some(sel), Some(ptr)) = (editor.selected, hover) {
                if sel < rips.len() && rips[sel].image < images.len() {
                    let x = make_xform(rect.min, view, images[rips[sel].image].pos, images[rips[sel].image].scale);
                    if let Some(h) = rip_tool::hit_handle(&rips[sel], &x, ptr, margin) {
                        editor.hover_handle = h;
                        ui.ctx().set_cursor_icon(rip_tool::handle_cursor(h));
                        over_handle = true;
                    }
                }
            }
            if !over_handle
                && hover
                    .and_then(|p| scale_grip_at(*active_image, images, view, rect, p, margin))
                    .is_some()
            {

                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
        }
    }

    for (idx, img) in images.iter().enumerate() {
        let img_rect = image_screen_rect(img, view, rect);
        if !rect.intersects(img_rect) {
            continue;
        }
        painter.image(
            img.texture.id(),
            img_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let stroke = if *active_image == Some(idx) {
            Stroke::new(2.0, Color32::from_rgb(90, 170, 255))
        } else {
            Stroke::new(1.0, Color32::from_gray(90))
        };
        painter.rect_stroke(img_rect, 0.0, stroke, StrokeKind::Outside);
    }

    for (i, rip) in rips.iter().enumerate() {
        if editor.selected == Some(i) || rip.image >= images.len() {
            continue;
        }
        let x = make_xform(rect.min, view, images[rip.image].pos, images[rip.image].scale);
        rip_tool::draw_rip(rip, &painter, &x, false);
    }
    if let Some(sel) = editor.selected {
        if sel < rips.len() && rips[sel].image < images.len() {
            let x = make_xform(rect.min, view, images[rips[sel].image].pos, images[rips[sel].image].scale);
            rip_tool::draw_rip(&rips[sel], &painter, &x, true);
            if guides.enabled {
                if let RipShape::Quad(c) = &rips[sel].shape {
                    rip_tool::draw_guides(c, &painter, &x, guides.vertical, guides.horizontal);
                }
            }
        }
    }

    if let Some(ai) = (*active_image).filter(|&i| i < images.len()) {
        let grip = image_screen_rect(&images[ai], view, rect).max;
        if rect.contains(grip) {
            let r = Rect::from_center_size(grip, Vec2::splat(10.0));
            painter.rect_filled(r, 2.0, Color32::from_rgb(90, 170, 255));
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        }
    }

    let icon_bg = if over_guide_btn {
        Color32::from_gray(70)
    } else {
        Color32::from_gray(48)
    };
    painter.rect_filled(guide_btn_rect, 3.0, icon_bg);
    let icon_col = if guides.enabled {
        Color32::from_rgb(255, 180, 0)
    } else {
        Color32::from_gray(200)
    };
    let st = Stroke::new(1.5, icon_col);
    let r = guide_btn_rect;
    let (vx1, vx2) = (r.left() + r.width() * 0.38, r.left() + r.width() * 0.62);
    let (hy1, hy2) = (r.top() + r.height() * 0.38, r.top() + r.height() * 0.62);
    painter.line_segment([Pos2::new(vx1, r.top() + 5.0), Pos2::new(vx1, r.bottom() - 5.0)], st);
    painter.line_segment([Pos2::new(vx2, r.top() + 5.0), Pos2::new(vx2, r.bottom() - 5.0)], st);
    painter.line_segment([Pos2::new(r.left() + 5.0, hy1), Pos2::new(r.right() - 5.0, hy1)], st);
    painter.line_segment([Pos2::new(r.left() + 5.0, hy2), Pos2::new(r.right() - 5.0, hy2)], st);
    guide_btn.on_hover_text("Toggle guide lines");

    if images.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Add an image to get started",
            egui::FontId::proportional(16.0),
            Color32::from_gray(120),
        );
    }
}

fn make_xform(canvas_min: Pos2, view: &ViewState, img_pos: Vec2, img_scale: f32) -> Xform {
    Xform {
        canvas_min,
        pan: view.pan,
        zoom: view.zoom,
        img_pos,
        img_scale,
    }
}

fn image_screen_rect(img: &LoadedImage, view: &ViewState, canvas: Rect) -> Rect {
    let top_left = canvas.min + view.pan + img.pos * view.zoom;
    Rect::from_min_size(top_left, img.size_vec() * img.scale * view.zoom)
}

fn scale_grip_at(
    active: Option<usize>,
    images: &[LoadedImage],
    view: &ViewState,
    canvas: Rect,
    ptr: Pos2,
    margin: f32,
) -> Option<usize> {
    let ai = active?;
    let img = images.get(ai)?;
    let grip = image_screen_rect(img, view, canvas).max;
    (grip.distance(ptr) <= margin).then_some(ai)
}

fn apply_image_scale(img: &mut LoadedImage, canvas_min: Pos2, view: &ViewState, ptr: Pos2) {
    let size = img.size_vec();
    let denom = size.dot(size);
    if denom <= f32::EPSILON || view.zoom <= 0.0 {
        return;
    }

    let corner = (ptr - canvas_min - view.pan) / view.zoom - img.pos;
    img.scale = (corner.dot(size) / denom).clamp(0.02, 64.0);
}

fn topmost_image_at(
    images: &[LoadedImage],
    view: &ViewState,
    canvas: Rect,
    point: Pos2,
) -> Option<usize> {
    images
        .iter()
        .enumerate()
        .rev()
        .find(|(_, img)| image_screen_rect(img, view, canvas).contains(point))
        .map(|(idx, _)| idx)
}
