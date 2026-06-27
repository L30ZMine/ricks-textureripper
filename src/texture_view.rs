//! The Texture View panel: a pannable / zoomable canvas that shows the loaded
//! source images, plus live, editable rip selections.

use std::path::{Path, PathBuf};

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::project::{LoadedImage, Project, ViewState};
use crate::rip_tool::{self, DragHandle, RipShape, Xform};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "gif", "tga", "tiff", "tif", "webp", "ico", "dds",
];

/// Opens a native file picker and returns the chosen image paths (the actual
/// loading is deferred by `app` so the wait cursor can be shown while decoding).
pub fn pick_image_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new()
        .set_title("Add Image")
        .add_filter("Images", SUPPORTED_EXTENSIONS)
        .pick_files()
}

/// True if `path` has a supported image extension (used for drag-and-drop).
pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.as_str()))
}

/// Paints a Photoshop-style transparency checkerboard over the part of `rect`
/// inside `clip`. Uses two dark greys in dark mode and two light greys in light
/// mode; the pattern is screen-fixed (it doesn't scroll with pan/zoom), like
/// Photoshop's. The phase is anchored to `phase` (the screen position the cell
/// grid lines up with): anchoring to a *stable* `phase` keeps the pattern from
/// jumping when `rect`'s top-left moves — e.g. the Texture View canvas shifting
/// as the contextual toolbar row pops in/out. Iteration is bounded to the visible
/// cells, so a huge `rect` (e.g. a zoomed-in atlas canvas larger than the panel)
/// doesn't cost a cell per off-screen square.
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

/// Uploads an RGBA buffer as a texture (shared by image loading and rip output).
pub fn upload_texture(
    ctx: &egui::Context,
    name: &str,
    rgba: &image::RgbaImage,
) -> egui::TextureHandle {
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

/// Builds a `LoadedImage` from already-decoded original RGBA pixels (at the
/// origin, no adjustments). Shared by disk loading and the self-contained project
/// open path, which decodes the pixels embedded in the `.rtrpf` file.
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

/// Decodes an image file into a `LoadedImage` (at the origin, no adjustments).
pub fn load_loaded_image(ctx: &egui::Context, path: &Path) -> Result<LoadedImage, String> {
    let dynimg = image::open(path).map_err(|e| e.to_string())?;
    let rgba = dynimg.to_rgba8();

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());

    Ok(loaded_image_from_pixels(ctx, name, path.to_path_buf(), rgba))
}

/// Assembles a `LoadedImage` from already-decoded pixels + a pre-built mip chain
/// (produced off the UI thread by [`crate::render::ImageLoader`]). The only
/// UI-thread cost is the texture upload, so adding big images stays responsive.
pub fn assemble_loaded_image(
    ctx: &egui::Context,
    name: String,
    source_path: PathBuf,
    original: image::RgbaImage,
    pixels: image::RgbaImage,
    mips: Vec<image::RgbaImage>,
) -> LoadedImage {
    let size = [pixels.width() as usize, pixels.height() as usize];
    let texture = upload_texture(ctx, &name, &pixels);
    LoadedImage {
        name,
        size,
        texture,
        mips,
        original,
        adjust: crate::project::Adjustments::default(),
        dirty: false,
        pixels,
        pos: Vec2::ZERO,
        scale: 1.0,
        source_path,
    }
}

/// Removes the image at `idx`, dropping its rips and re-indexing the rest so
/// remaining rips keep pointing at the right images.
pub fn remove_image(project: &mut Project, idx: usize) {
    if idx >= project.images.len() {
        return;
    }
    project.images.remove(idx);

    // Drop rips on the removed image; shift down rips that referenced a later one.
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

    // Fix up the active image and rip selection.
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

/// Removes the rip at `idx`, clearing the selection and repacking the atlas.
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

/// Draws the whole Texture View panel (top toolbar + canvas + bottom bar).
pub fn ui(ui: &mut egui::Ui, project: &mut Project) {
    // Primary controls in a top toolbar.
    egui::TopBottomPanel::top("texture_toolbar").show_inside(ui, |ui| {
        ui.add_space(2.0);
        // Horizontal scroll so a narrow panel scrolls the toolbar instead of
        // clipping its buttons (the dock no longer wraps panels in a scrollbar).
        crate::ui::thin_scrollbar(ui);
        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .id_salt("texture_toolbar_scroll")
            .show(ui, |ui| toolbar(ui, project));
        ui.add_space(2.0);
    });

    // The selected rip's shape controls move into a bottom bar (shown only when a
    // rip is selected) instead of stacking a second toolbar row at the top.
    if project
        .editor
        .selected
        .is_some_and(|s| s < project.rips.len())
    {
        egui::TopBottomPanel::bottom("texture_shape_bar").show_inside(ui, |ui| {
            ui.add_space(5.0);
            crate::ui::thin_scrollbar(ui);
            egui::ScrollArea::horizontal()
                .auto_shrink([false, true])
                .id_salt("texture_shape_bar_scroll")
                .show(ui, |ui| shape_bar(ui, project));
            ui.add_space(2.0);
        });
    }

    // The transient status message lives in the bottom chin bar (see
    // `App::status_bar`); the canvas fills the space between the two bars.
    egui::CentralPanel::default()
        .frame(egui::Frame::new())
        .show_inside(ui, |ui| canvas(ui, project));
}

fn toolbar(ui: &mut egui::Ui, project: &mut Project) {
    let mut remove_active_image: Option<usize> = None;
    let mut delete: Option<usize> = None;
    ui.horizontal(|ui| {
        if ui.button("Add Image").clicked() {
            // Deferred to `app` (which shows the dialog + load with a wait cursor).
            project.want_add_image = true;
        }
        if ui.button("Add Rip").clicked() {
            rip_tool::add_rip(project);
        }
        ui.separator();
        // Remove the active image (replaces the old Deselect button).
        let active = project.active_image.filter(|&i| i < project.images.len());
        if ui
            .add_enabled(active.is_some(), egui::Button::new("Remove Image"))
            .clicked()
        {
            remove_active_image = active;
        }
        // Remove the selected rip (sits next to Remove Image).
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
        // Reset the active image's display scale back to 1.0 (enabled only when it
        // has actually been scaled).
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
    if let Some(i) = delete {
        remove_rip(project, i);
    }
}

/// The selected rip's shape controls (Quad / Circle), shown in the Texture
/// View's bottom bar when a rip is selected.
fn shape_bar(ui: &mut egui::Ui, project: &mut Project) {
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
}

/// The pannable / zoomable image canvas with live rip editing.
fn canvas(ui: &mut egui::Ui, project: &mut Project) {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    // Anchor the checkerboard phase to a window-fixed origin (0,0) rather than the
    // canvas top-left, so the pattern stays put when the contextual "Shape" row
    // pops in/out and shifts this canvas down/up.
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

    // Anchor the view independently of the toolbar. The contextual "Shape" row
    // pops in/out when a rip is (de)selected, which changes this canvas's top —
    // and since the image is drawn at `rect.min + pan`, that would otherwise jump
    // the whole workspace. Compensate `pan` by the origin delta so it stays put.
    if let Some(prev) = view.last_origin {
        let shift = rect.min - prev;
        if shift != Vec2::ZERO {
            view.pan -= shift;
        }
    }
    view.last_origin = Some(rect.min);

    // Guide-toggle icon in the top-right corner of the canvas.
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

    // --- Zoom around the pointer ----------------------------------------
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

    // --- Left button: Shift = move image / pan, plain = rip handle ------
    // (plain left-drag only adjusts rip handles, so the cursor never lies.)
    if response.drag_started_by(egui::PointerButton::Primary) && !over_guide {
        editor.drag = DragHandle::None;
        view.dragging_image = None;
        view.scaling_image = None;
        view.panning = false;
        // Hit-test the *press origin* (where the button first went down), not the
        // live pointer. egui only flags a drag after the pointer travels a few px
        // past its threshold, by which point `interact_pointer_pos` has slipped off
        // the handle the user pressed on — so a fresh hit-test there can miss a
        // handle the cursor was clearly hovering. The press origin is exactly where
        // they aimed.
        let press = ui
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if ui.input(|i| i.modifiers.shift) {
            // Shift-drag moves the image under the cursor; empty space pans.
            view.dragging_image = press.and_then(|p| topmost_image_at(images, view, rect, p));
            view.panning = view.dragging_image.is_none();
        } else {
            // Rip handles take priority over the image scale-grip: only grab the
            // grip if the press didn't land on a rip handle.
            let mut grabbed_rip = false;
            if let (Some(ptr), Some(sel)) = (press, editor.selected) {
                if sel < rips.len() && rips[sel].image < images.len() {
                    let x = make_xform(rect.min, view, images[rips[sel].image].pos, images[rips[sel].image].scale);
                    // Fall back to the handle the cursor was showing just before the
                    // press only if the press origin somehow hits nothing.
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
                    // Grabbing the active image's corner grip scales it (aspect-locked).
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

    // --- Middle button: always pans the workspace ------------------------
    if response.dragged_by(egui::PointerButton::Middle) {
        view.pan += response.drag_delta();
    }

    if response.drag_stopped() {
        editor.drag = DragHandle::None;
        view.dragging_image = None;
        view.scaling_image = None;
        view.panning = false;
    }

    // --- Click selects a rip (or an image) ------------------------------
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
                        // Clicking empty space keeps the current image selected
                        // (so "Add Rip" still targets it); only switch when the
                        // click actually lands on another image.
                        if let Some(idx) = topmost_image_at(images, view, rect, ptr) {
                            *active_image = Some(idx);
                        }
                    }
                }
            }
        }
    }

    // --- Cursor feedback ------------------------------------------------
    // Remember the handle under the cursor this frame so a drag started next
    // frame can use it (see the drag-start handling above).
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
            // Shift hover = "move image / pan" mode.
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else {
            // Rip-handle hover takes priority over the scale-grip.
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
                // Over the active image's corner scale-grip.
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
        }
    }

    // --- Draw images -----------------------------------------------------
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

    // --- Draw rips (unselected first, selected on top) ------------------
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

    // --- Active image scale grip (bottom-right corner) ------------------
    // A small handle on the active image's corner; drag it to scale the image
    // (and its rips) in the workspace, aspect-locked. Display-only.
    if let Some(ai) = (*active_image).filter(|&i| i < images.len()) {
        let grip = image_screen_rect(&images[ai], view, rect).max;
        if rect.contains(grip) {
            let r = Rect::from_center_size(grip, Vec2::splat(10.0));
            painter.rect_filled(r, 2.0, Color32::from_rgb(90, 170, 255));
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        }
    }

    // --- Guide-toggle icon (four crossing lines, "#"-style) -------------
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

/// Builds the local<->screen transform for an image at `img_pos` with the given
/// per-image display `img_scale`.
fn make_xform(canvas_min: Pos2, view: &ViewState, img_pos: Vec2, img_scale: f32) -> Xform {
    Xform {
        canvas_min,
        pan: view.pan,
        zoom: view.zoom,
        img_pos,
        img_scale,
    }
}

/// Screen-space rectangle an image occupies given the current view transform.
fn image_screen_rect(img: &LoadedImage, view: &ViewState, canvas: Rect) -> Rect {
    let top_left = canvas.min + view.pan + img.pos * view.zoom;
    Rect::from_min_size(top_left, img.size_vec() * img.scale * view.zoom)
}

/// If `ptr` is within `margin` of the active image's bottom-right scale grip,
/// returns that image's index. Only the active image exposes a grip.
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

/// Uniformly (aspect-locked) scales `img` so its bottom-right corner tracks
/// `ptr`, keeping the top-left anchored. The factor is the cursor's projection
/// onto the image's diagonal, so dragging in any direction feels natural.
fn apply_image_scale(img: &mut LoadedImage, canvas_min: Pos2, view: &ViewState, ptr: Pos2) {
    let size = img.size_vec();
    let denom = size.dot(size);
    if denom <= f32::EPSILON || view.zoom <= 0.0 {
        return;
    }
    // World-space vector from the image's top-left to the cursor.
    let corner = (ptr - canvas_min - view.pan) / view.zoom - img.pos;
    img.scale = (corner.dot(size) / denom).clamp(0.02, 64.0);
}

/// Index of the top-most image containing `point` (later images draw on top).
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
