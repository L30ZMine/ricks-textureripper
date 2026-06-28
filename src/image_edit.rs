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

/// Applies all per-pixel, same-size adjustments in place: colour (brightness /
/// contrast / saturation / temperature / hue / gamma) then the optional
/// background colour-key (which only writes alpha). Size-changing work
/// (sharpen / blur, rotation) lives in [`apply_filters`] / [`apply_orientation`].
pub fn apply_adjustments(img: &mut RgbaImage, adj: &Adjustments) {
    apply_color(img, adj);
    apply_color_key(img, adj);
}

/// Copy bundle of precomputed colour-adjustment parameters, so the per-pixel work
/// can be handed to worker threads.
#[derive(Clone, Copy)]
struct ColorParams {
    bright: f32,
    contrast: f32,
    sat: f32,
    temp: f32,
    do_hue: bool,
    m: [f32; 9],
    do_gamma: bool,
    gamma_exp: f32,
    /// Per-channel Multiply tint factor (`1.0` = no change). Applied last.
    do_tint: bool,
    tint: [f32; 3],
}

/// Applies the colour math to a single RGBA pixel (alpha untouched).
#[inline]
fn color_pixel(px: &mut [u8], p: &ColorParams) {
    let mut c = [px[0] as f32, px[1] as f32, px[2] as f32];
    // Contrast around mid-gray, then brightness.
    for v in &mut c {
        *v = (*v - 128.0) * p.contrast + 128.0 + p.bright;
    }
    // Temperature.
    c[0] += p.temp;
    c[2] -= p.temp;
    // Saturation: lerp each channel toward perceived luminance.
    let l = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    for v in &mut c {
        *v = l + (*v - l) * p.sat;
    }
    // Hue rotation.
    if p.do_hue {
        let (r, g, b) = (c[0], c[1], c[2]);
        c[0] = r * p.m[0] + g * p.m[1] + b * p.m[2];
        c[1] = r * p.m[3] + g * p.m[4] + b * p.m[5];
        c[2] = r * p.m[6] + g * p.m[7] + b * p.m[8];
    }
    // Gamma on the [0,255] range.
    if p.do_gamma {
        for v in &mut c {
            *v = 255.0 * (v.clamp(0.0, 255.0) / 255.0).powf(p.gamma_exp);
        }
    }
    // Colour tint: Multiply blend toward the tint colour (each channel scaled by
    // its precomputed factor; strength is already folded into the factor).
    if p.do_tint {
        c[0] *= p.tint[0];
        c[1] *= p.tint[1];
        c[2] *= p.tint[2];
    }
    px[0] = c[0].clamp(0.0, 255.0) as u8;
    px[1] = c[1].clamp(0.0, 255.0) as u8;
    px[2] = c[2].clamp(0.0, 255.0) as u8;
}

/// Brightness / contrast / saturation / temperature / hue / gamma, applied to
/// RGB (alpha untouched). No-op when the colour adjustment is identity. The
/// per-pixel pass is spread across CPU cores for large images.
fn apply_color(img: &mut RgbaImage, adj: &Adjustments) {
    if adj.is_identity() {
        return;
    }
    // Luminance-preserving hue rotation matrix (built once; used per-pixel only
    // when hue is non-zero).
    let a = (adj.hue * 180.0).to_radians();
    let (ac, as_) = (a.cos(), a.sin());
    let p = ColorParams {
        bright: adj.brightness * 255.0,
        contrast: 1.0 + adj.contrast, // 0 = flat gray, 1 = unchanged, 2 = double
        sat: 1.0 + adj.saturation,    // 0 = grayscale, 1 = unchanged, 2 = vivid
        temp: adj.temperature * 30.0, // warm (+R/-B) ↔ cool (-R/+B)
        do_hue: adj.hue != 0.0,
        m: [
            0.213 + ac * 0.787 - as_ * 0.213,
            0.715 - ac * 0.715 - as_ * 0.715,
            0.072 - ac * 0.072 + as_ * 0.928,
            0.213 - ac * 0.213 + as_ * 0.143,
            0.715 + ac * 0.285 + as_ * 0.140,
            0.072 - ac * 0.072 - as_ * 0.283,
            0.213 - ac * 0.213 - as_ * 0.787,
            0.715 - ac * 0.715 + as_ * 0.715,
            0.072 + ac * 0.928 + as_ * 0.072,
        ],
        do_gamma: adj.gamma != 0.0,
        gamma_exp: 2f32.powf(-adj.gamma * 2.0), // 0 → 1.0 (no change)
        do_tint: adj.tint_enabled && adj.tint_strength > 0.0,
        // Multiply factor per channel: lerp 1.0 → (tint/255) by strength, so
        // `out = base * (1 - s*(1 - tint/255))`.
        tint: {
            let s = adj.tint_strength.clamp(0.0, 1.0);
            let f = |c: u8| 1.0 - s * (1.0 - c as f32 / 255.0);
            [f(adj.tint_color[0]), f(adj.tint_color[1]), f(adj.tint_color[2])]
        },
    };

    let px_count = (img.width() * img.height()) as usize;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if threads <= 1 || px_count < 100_000 {
        for px in img.chunks_exact_mut(4) {
            color_pixel(px, &p);
        }
    } else {
        let band = px_count.div_ceil(threads) * 4;
        std::thread::scope(|s| {
            for chunk in img.chunks_mut(band) {
                s.spawn(move || {
                    for px in chunk.chunks_exact_mut(4) {
                        color_pixel(px, &p);
                    }
                });
            }
        });
    }
}

/// Background removal: zeroes the alpha of pixels within `key_tol` of
/// `key_color`. No-op unless enabled.
fn apply_color_key(img: &mut RgbaImage, adj: &Adjustments) {
    if !adj.key_enabled {
        return;
    }
    let key = adj.key_color;
    // Tolerance as a fraction of the maximum RGB euclidean distance.
    let tol = adj.key_tol.clamp(0.0, 1.0) * (3.0f32 * 255.0 * 255.0).sqrt();
    let tol2 = tol * tol;
    for px in img.pixels_mut() {
        if px[3] == 0 {
            continue;
        }
        let dr = px[0] as f32 - key[0] as f32;
        let dg = px[1] as f32 - key[1] as f32;
        let db = px[2] as f32 - key[2] as f32;
        if dr * dr + dg * dg + db * db <= tol2 {
            px[3] = 0;
        }
    }
}

/// Applies the size-preserving filters (Gaussian blur, then unsharp-mask
/// sharpen), returning a possibly-new buffer. No-op when both are off.
pub fn apply_filters(mut img: RgbaImage, adj: &Adjustments) -> RgbaImage {
    if adj.blur > 0.0 {
        img = image::imageops::blur(&img, (adj.blur * 8.0).max(0.1));
    }
    if adj.sharpen > 0.0 {
        img = sharpen(&img, adj.sharpen * 2.0);
    }
    img
}

/// Unsharp-mask sharpen: `out = orig + amount * (orig - blurred)`.
fn sharpen(img: &RgbaImage, amount: f32) -> RgbaImage {
    let blurred = image::imageops::blur(img, 2.0);
    let mut out = img.clone();
    for (po, pb) in out.pixels_mut().zip(blurred.pixels()) {
        for ch in 0..3 {
            let o = po[ch] as f32;
            let b = pb[ch] as f32;
            po[ch] = (o + amount * (o - b)).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Applies a rip's rotation / mirroring, returning a possibly-new buffer.
pub fn apply_orientation(mut img: RgbaImage, o: &crate::project::Orientation) -> RgbaImage {
    if o.is_identity() {
        return img;
    }
    use image::imageops::{flip_horizontal, flip_vertical, rotate180, rotate270, rotate90};
    img = match o.rotate % 4 {
        1 => rotate90(&img),
        2 => rotate180(&img),
        3 => rotate270(&img),
        _ => img,
    };
    if o.flip_h {
        img = flip_horizontal(&img);
    }
    if o.flip_v {
        img = flip_vertical(&img);
    }
    img
}

/// The output size after a 90°/270° rotation swaps width and height (flips don't
/// change the size).
pub fn oriented_size(size: [usize; 2], o: &crate::project::Orientation) -> [usize; 2] {
    if o.rotate % 2 == 1 {
        [size[1], size[0]]
    } else {
        size
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
        let work = apply_filters(work, &img.adjust);
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
            RipShape::CurvedQuad {
                corners,
                out_handles,
                in_handles,
            } => {
                for p in corners.iter_mut() {
                    p.x *= sx;
                    p.y *= sy;
                }
                for v in out_handles.iter_mut().chain(in_handles.iter_mut()) {
                    v.x *= sx;
                    v.y *= sy;
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
    // Its own vertical scroll area (the dock no longer adds one): the slider stack
    // can be taller than a short panel, so it scrolls rather than clipping. `wide`
    // is measured inside so the column split accounts for the scrollbar width.
    crate::ui::thin_scrollbar(ui);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let wide = ui.available_width() >= WIDE_THRESHOLD;
            let rip_target = project.editor.selected.filter(|&i| i < project.rips.len());
            if let Some(ri) = rip_target {
                rip_editor(ui, project, ri, wide);
            } else if let Some(ii) = project.active_image.filter(|&i| i < project.images.len()) {
                image_editor(ui, project, ii, wide);
            } else {
                ui.weak("Select an image or a rip to edit.");
            }
        });
}

/// `#RRGGBB` hex web code for an opaque colour.
fn hex_of(c: egui::Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// Parses a `#rrggbb` / `rrggbb` (or short `#rgb` / `rgb`) hex web code.
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    let byte = |x: &str| u8::from_str_radix(x, 16).ok();
    match s.len() {
        6 => Some([byte(&s[0..2])?, byte(&s[2..4])?, byte(&s[4..6])?]),
        3 => {
            // "abc" → "aabbcc"
            let mut it = s.chars();
            let mut dup = || {
                let ch = it.next()?;
                u8::from_str_radix(&format!("{ch}{ch}"), 16).ok()
            };
            Some([dup()?, dup()?, dup()?])
        }
        _ => None,
    }
}

/// A normal-size colour swatch button that opens a colour picker popup scaled
/// down ~40% (egui's built-in `color_edit_button_srgb` hardcodes a large popup,
/// so we drive the swatch + popup ourselves). Returns true if the colour changed.
fn key_color_button(ui: &mut egui::Ui, color: &mut [u8; 3], salt: &str) -> bool {
    use egui::{Area, Color32, Frame, Key, Order, Sense, StrokeKind, UiKind};
    let mut changed = false;
    let popup_id = ui.id().with(salt);

    // Swatch button at the normal interact size.
    let (rect, resp) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
    let visuals = *ui.style().interact(&resp);
    ui.painter().rect_filled(
        rect,
        visuals.corner_radius,
        Color32::from_rgb(color[0], color[1], color[2]),
    );
    ui.painter()
        .rect_stroke(rect, visuals.corner_radius, visuals.bg_stroke, StrokeKind::Inside);
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }

    if ui.memory(|m| m.is_popup_open(popup_id)) {
        let area = Area::new(popup_id)
            .kind(UiKind::Picker)
            .order(Order::Foreground)
            .fixed_pos(resp.rect.max)
            .show(ui.ctx(), |ui| {
                // The built-in picker uses a 275px slider width; 60% of that makes
                // the whole popup ~40% smaller.
                ui.spacing_mut().slider_width = 275.0 * 0.6;
                Frame::popup(ui.style()).show(ui, |ui| {
                    let mut c = Color32::from_rgb(color[0], color[1], color[2]);
                    let mut edited = false;

                    // A `#hex` web-code field (with a Copy button) at the top of the
                    // popup, above egui's picker: type `#ffffff` / `#fff` to set the
                    // colour, and it reflects changes made via the square / sliders.
                    let hex_id = ui.id().with("hex_field");
                    let mut hex = ui
                        .data(|d| d.get_temp::<String>(hex_id))
                        .unwrap_or_else(|| hex_of(c));
                    ui.horizontal(|ui| {
                        ui.label("Hex");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut hex)
                                .desired_width(72.0)
                                .char_limit(7),
                        );
                        // Sync from the colour while not being typed in (so
                        // square/slider edits show up); keep the in-progress text
                        // while focused, then parse `#ffffff` / `#fff`.
                        if !resp.has_focus() {
                            hex = hex_of(c);
                        } else if resp.changed() {
                            if let Some(rgb) = parse_hex(&hex) {
                                c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                                edited = true;
                            }
                        }
                        if ui
                            .button("Copy")
                            .on_hover_text("Copy the hex code")
                            .clicked()
                        {
                            ui.ctx().copy_text(hex_of(c));
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(hex_id, hex));

                    // egui's colour picker (square + hue + R/G/B) below.
                    edited |= egui::widgets::color_picker::color_picker_color32(
                        ui,
                        &mut c,
                        egui::widgets::color_picker::Alpha::Opaque,
                    );

                    if edited {
                        *color = [c.r(), c.g(), c.b()];
                        changed = true;
                    }
                });
            })
            .response;
        // Don't let the click that *opened* the popup (which lands outside the
        // area) immediately close it again.
        if !resp.clicked()
            && (ui.input(|i| i.key_pressed(Key::Escape)) || area.clicked_elsewhere())
        {
            ui.memory_mut(|m| m.close_popup());
        }
    }
    changed
}

/// Colour + filter adjustment sliders bound to `adj`; returns true if any
/// changed. Shared by the rip and image editors.
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
    changed |= ui
        .add(egui::Slider::new(&mut adj.hue, -1.0..=1.0).text("Hue"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut adj.temperature, -1.0..=1.0).text("Temperature"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut adj.gamma, -1.0..=1.0).text("Gamma"))
        .changed();
    ui.separator();
    changed |= ui
        .add(egui::Slider::new(&mut adj.sharpen, 0.0..=1.0).text("Sharpen"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut adj.blur, 0.0..=1.0).text("Blur"))
        .changed();
    ui.separator();
    changed |= ui
        .checkbox(&mut adj.key_enabled, "Remove background colour")
        .changed();
    if adj.key_enabled {
        ui.horizontal(|ui| {
            ui.label("Key");
            // Normal-size swatch button; the picker popup it opens is scaled down.
            if key_color_button(ui, &mut adj.key_color, "key_colour_picker") {
                changed = true;
            }
            // Screen-wide pipette (Windows): sample any pixel on screen, incl. UI.
            #[cfg(windows)]
            {
                if pipette(ui, &mut adj.key_color) {
                    changed = true;
                }
            }
            ui.label("Tol");
            if ui
                .add(egui::Slider::new(&mut adj.key_tol, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
    }
    ui.separator();
    // Colour tint (Multiply blend) — overlays a colour onto the rip/image.
    changed |= ui
        .checkbox(&mut adj.tint_enabled, "Tint colour")
        .on_hover_text("Multiply the image by a colour (like Photoshop's Multiply blend).")
        .changed();
    if adj.tint_enabled {
        ui.horizontal(|ui| {
            ui.label("Colour");
            if key_color_button(ui, &mut adj.tint_color, "tint_colour_picker") {
                changed = true;
            }
            ui.label("Amount");
            if ui
                .add(egui::Slider::new(&mut adj.tint_strength, 0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });
    }
    changed
}

/// A screen-wide colour pipette (Windows only). Renders a toggle button; while
/// active it shows a live swatch of the colour under the cursor and, on the next
/// mouse click *anywhere on screen* (other windows or this app's own UI), writes
/// that colour to `out` and returns `true`. Esc cancels. Picking state lives in
/// egui temp data so it survives across frames.
#[cfg(windows)]
fn pipette(ui: &mut egui::Ui, out: &mut [u8; 3]) -> bool {
    let active_id = egui::Id::new("rtr_key_pipette_active");
    let armed_id = egui::Id::new("rtr_key_pipette_armed");
    let mut active = ui.data(|d| d.get_temp::<bool>(active_id).unwrap_or(false));

    if ui
        .selectable_label(active, "Pipette")
        .on_hover_text("Sample a colour from anywhere on screen, then click")
        .clicked()
    {
        active = !active;
        ui.data_mut(|d| {
            d.insert_temp(active_id, active);
            d.insert_temp(armed_id, false);
        });
    }

    if !active {
        return false;
    }

    // Keep ticking (even unfocused) so we can poll the global cursor / button.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);

    // Live swatch of the colour currently under the cursor.
    if let Some(rgb) = winpick::pixel_at_cursor() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(120)),
            egui::StrokeKind::Inside,
        );
    }
    ui.weak("Esc");

    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.insert_temp(active_id, false));
        return false;
    }

    // Arm only once the button that started picking has been released, so that
    // very click doesn't immediately sample; then sample on the next press.
    let armed = ui.data(|d| d.get_temp::<bool>(armed_id).unwrap_or(false));
    let down = winpick::lmb_down();
    if !armed {
        if !down {
            ui.data_mut(|d| d.insert_temp(armed_id, true));
        }
        return false;
    }
    if down {
        let sampled = winpick::pixel_at_cursor();
        ui.data_mut(|d| {
            d.insert_temp(active_id, false);
            d.insert_temp(armed_id, false);
        });
        if let Some(rgb) = sampled {
            *out = rgb;
            return true;
        }
    }
    false
}

/// Raw Win32 GDI screen-colour sampling for the pipette — no extra crate (same
/// raw-`#[link]` style as `file_assoc`'s shell32 binding). `GetPixel` on the
/// whole-screen DC reads the colour of any pixel currently on screen (including
/// other windows and this app's own UI); `GetAsyncKeyState` polls the left mouse
/// button globally so the pick-click works even when another window has focus.
#[cfg(windows)]
mod winpick {
    use std::os::raw::{c_int, c_void};

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetCursorPos(p: *mut Point) -> i32;
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> c_int;
        fn GetAsyncKeyState(v: c_int) -> i16;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetPixel(hdc: *mut c_void, x: c_int, y: c_int) -> u32;
    }

    /// The sRGB colour under the global mouse cursor, if it can be read.
    pub fn pixel_at_cursor() -> Option<[u8; 3]> {
        unsafe {
            let mut p = Point { x: 0, y: 0 };
            if GetCursorPos(&mut p) == 0 {
                return None;
            }
            // Null hWnd → the screen DC, so any on-screen pixel is readable.
            let hdc = GetDC(std::ptr::null_mut());
            if hdc.is_null() {
                return None;
            }
            let c = GetPixel(hdc, p.x, p.y);
            ReleaseDC(std::ptr::null_mut(), hdc);
            // COLORREF is 0x00BBGGRR; GetPixel returns CLR_INVALID on failure.
            if c == 0xFFFF_FFFF {
                return None;
            }
            Some([
                (c & 0xFF) as u8,
                ((c >> 8) & 0xFF) as u8,
                ((c >> 16) & 0xFF) as u8,
            ])
        }
    }

    /// True while the (logical) left mouse button is physically down — read
    /// globally, so it fires regardless of which window has focus.
    pub fn lmb_down() -> bool {
        const VK_LBUTTON: c_int = 0x01;
        unsafe { (GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000) != 0 }
    }
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

/// The non-slider rip controls (orientation + output-size override + reset).
/// Returns dirty.
fn rip_tools(ui: &mut egui::Ui, rip: &mut crate::project::Rip) -> bool {
    let mut dirty = false;

    // Rotation / mirroring of the extracted texture (rip-only).
    ui.horizontal(|ui| {
        ui.label("Orient:");
        if ui.button("Rotate L").on_hover_text("Rotate 90° left").clicked() {
            rip.orient.rotate = (rip.orient.rotate + 3) % 4;
            dirty = true;
        }
        if ui.button("Rotate R").on_hover_text("Rotate 90° right").clicked() {
            rip.orient.rotate = (rip.orient.rotate + 1) % 4;
            dirty = true;
        }
        if ui.selectable_label(rip.orient.flip_h, "Flip H").clicked() {
            rip.orient.flip_h = !rip.orient.flip_h;
            dirty = true;
        }
        if ui.selectable_label(rip.orient.flip_v, "Flip V").clicked() {
            rip.orient.flip_v = !rip.orient.flip_v;
            dirty = true;
        }
    });
    ui.separator();

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
    // `ui.columns` lays out justified (widgets fill the column width); wrap the
    // button so it keeps its natural size instead of stretching across the column.
    ui.horizontal(|ui| {
        if ui.button("Reset adjustments").clicked() {
            rip.adjust = Adjustments::default();
            rip.resize = None;
            rip.orient = crate::project::Orientation::default();
            dirty = true;
        }
    });

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
    // Keep the button at its natural size in the justified `ui.columns` layout.
    ui.horizontal(|ui| {
        if ui.button("Reset size").clicked() {
            w = orig.0;
            h = orig.1;
        }
    });

    // Commit each frame so a drag accumulates; rips are rescaled by the
    // incremental ratio to stay locked to the same image features.
    if w != old[0] || h != old[1] {
        let sx = w as f32 / old[0] as f32;
        let sy = h as f32 / old[1] as f32;
        scale_rips_on_image(project, ii, sx, sy);
        project.images[ii].size = [w, h];
        dirty = true;
    }

    ui.horizontal(|ui| {
        if ui.button("Reset adjustments").clicked() {
            project.images[ii].adjust = Adjustments::default();
            dirty = true;
        }
    });

    dirty
}
