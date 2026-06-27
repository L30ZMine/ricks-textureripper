//! The Rip tool: live, editable selections drawn on the source images.
//!
//! A selection is either a free **quad** (four corners that can be dragged
//! independently to warp perspective) or a **circle**. Geometry is stored in
//! image-local pixel coordinates. Editing marks the rip dirty; the output is
//! recomputed live (no explicit "extract" step).

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::project::{LoadedImage, Project, Rip, RipOutput};

/// Selection geometry, in image-local pixel coordinates.
#[derive(Clone, Debug)]
pub enum RipShape {
    /// Four free corners in order TL, TR, BR, BL (perspective quad).
    Quad([Pos2; 4]),
    Circle { center: Pos2, radius: f32 },
}

/// Which handle of the selected rip is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragHandle {
    None,
    QuadCorner(usize),
    QuadEdge(usize),
    QuadMove,
    CircleCenter,
    CircleRadius,
    CircleMove,
}

/// Rip selection / drag state held by a project.
pub struct RipEditor {
    /// Index of the selected rip (the one showing editable handles).
    pub selected: Option<usize>,
    pub drag: DragHandle,
    /// The handle under the cursor as of the last hover frame. A drag now
    /// hit-tests the press *origin* directly (see `texture_view`), so this is only
    /// a fallback for the rare case where that hits nothing.
    pub hover_handle: DragHandle,
}

impl Default for RipEditor {
    fn default() -> Self {
        Self {
            selected: None,
            drag: DragHandle::None,
            hover_handle: DragHandle::None,
        }
    }
}

impl RipEditor {
    pub fn is_dragging(&self) -> bool {
        self.drag != DragHandle::None
    }
}

/// Maps between image-local pixel coords and screen coords.
pub struct Xform {
    pub canvas_min: Pos2,
    pub pan: Vec2,
    pub zoom: f32,
    pub img_pos: Vec2,
    /// Per-image display scale (1.0 = natural). Image-local coords are multiplied
    /// by this before placement, so a scaled image (and its rips) draw and
    /// hit-test bigger/smaller without touching the underlying pixel geometry.
    pub img_scale: f32,
}

impl Xform {
    pub fn local_to_screen(&self, local: Pos2) -> Pos2 {
        self.canvas_min + self.pan + (self.img_pos + local.to_vec2() * self.img_scale) * self.zoom
    }

    pub fn screen_to_local(&self, screen: Pos2) -> Pos2 {
        ((((screen - self.canvas_min - self.pan) / self.zoom) - self.img_pos) / self.img_scale)
            .to_pos2()
    }
}

/// Closest point on the segment `a`–`b` to `p` (screen coords).
fn closest_point_on_segment(p: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return a;
    }
    let t = (((p - a).dot(ab)) / len2).clamp(0.0, 1.0);
    a + ab * t
}

// ---------------------------------------------------------------------------
// Construction / shape conversion
// ---------------------------------------------------------------------------

/// Default axis-aligned quad covering the middle ~half of an image.
pub fn default_quad(img: &LoadedImage) -> RipShape {
    let s = img.size_vec();
    RipShape::Quad([
        Pos2::new(s.x * 0.25, s.y * 0.25),
        Pos2::new(s.x * 0.75, s.y * 0.25),
        Pos2::new(s.x * 0.75, s.y * 0.75),
        Pos2::new(s.x * 0.25, s.y * 0.75),
    ])
}

/// Adds a new live rip on the active image (or the last image) and selects it.
pub fn add_rip(project: &mut Project) {
    let target = project
        .active_image
        .filter(|&i| i < project.images.len())
        .or_else(|| project.images.len().checked_sub(1));

    let Some(idx) = target else {
        project.set_error("Add an image first.");
        return;
    };

    let name = project.next_rip_name();
    let shape = default_quad(&project.images[idx]);
    project.rips.push(Rip {
        name,
        image: idx,
        shape,
        adjust: crate::project::Adjustments::default(),
        orient: crate::project::Orientation::default(),
        resize: None,
        atlas_pos: None,
        dirty: true,
        previewed: false,
        output: None,
    });
    project.active_image = Some(idx);
    project.editor.selected = Some(project.rips.len() - 1);
    project.atlas_dirty = true;
    project.modified = true;
    project.set_status("Drag the corners to warp; the rip updates live.");
}

/// Bounding box of the current shape (image-local).
fn shape_bounds(shape: &RipShape) -> Rect {
    match shape {
        RipShape::Quad(c) => {
            let mut r = Rect::from_two_pos(c[0], c[1]);
            r = r.union(Rect::from_two_pos(c[2], c[3]));
            r
        }
        RipShape::Circle { center, radius } => {
            Rect::from_center_size(*center, Vec2::splat(radius * 2.0))
        }
    }
}

/// Converts a rip to a quad (axis-aligned from its current bounds).
pub fn set_shape_quad(rip: &mut Rip) {
    if matches!(rip.shape, RipShape::Quad(_)) {
        return;
    }
    let b = shape_bounds(&rip.shape);
    rip.shape = RipShape::Quad([
        b.left_top(),
        b.right_top(),
        b.right_bottom(),
        b.left_bottom(),
    ]);
    rip.dirty = true;
}

/// Converts a rip to a circle (inscribed in its current bounds).
pub fn set_shape_circle(rip: &mut Rip) {
    if matches!(rip.shape, RipShape::Circle { .. }) {
        return;
    }
    let b = shape_bounds(&rip.shape);
    rip.shape = RipShape::Circle {
        center: b.center(),
        radius: b.size().min_elem() * 0.5,
    };
    rip.dirty = true;
}

// ---------------------------------------------------------------------------
// Hit-testing & dragging
// ---------------------------------------------------------------------------

/// Returns the handle under `ptr` (screen coords) for editing this rip.
///
/// `margin` (screen px, user-tunable) is the single grab tolerance used for
/// every part: the corner grab radius, the perpendicular grab distance for an
/// edge, the edge dead-zone around each vertex, and the inset of the move region
/// from the selection's border. Driving them all from one value means the edge
/// band and the move region are exactly complementary (no dead band between
/// them) and edges are as easy to grab as corners.
pub fn hit_handle(rip: &Rip, x: &Xform, ptr: Pos2, margin: f32) -> Option<DragHandle> {
    match &rip.shape {
        RipShape::Quad(c) => {
            for (i, corner) in c.iter().enumerate() {
                if x.local_to_screen(*corner).distance(ptr) <= margin {
                    return Some(DragHandle::QuadCorner(i));
                }
            }
            // Edges are grabbable along their length, except within `margin` of
            // the vertices (so corner grabs win there).
            let mut min_edge = f32::INFINITY;
            for i in 0..4 {
                let a = x.local_to_screen(c[i]);
                let b = x.local_to_screen(c[(i + 1) % 4]);
                let proj = closest_point_on_segment(ptr, a, b);
                let d = proj.distance(ptr);
                min_edge = min_edge.min(d);
                if d <= margin
                    && proj.distance(a) > margin
                    && proj.distance(b) > margin
                {
                    return Some(DragHandle::QuadEdge(i));
                }
            }
            // Move only when grabbed at least `margin` inside the selection.
            if point_in_quad(c, x, ptr) && min_edge > margin {
                return Some(DragHandle::QuadMove);
            }
            None
        }
        RipShape::Circle { center, radius } => {
            let radius_handle = *center + Vec2::new(*radius, 0.0);
            if x.local_to_screen(radius_handle).distance(ptr) <= margin {
                return Some(DragHandle::CircleRadius);
            }
            if x.local_to_screen(*center).distance(ptr) <= margin {
                return Some(DragHandle::CircleCenter);
            }
            // Move only when grabbed `margin` inside the circle (in screen px).
            let inset = (*radius * x.zoom - margin).max(0.0);
            if x.local_to_screen(*center).distance(ptr) <= inset {
                return Some(DragHandle::CircleMove);
            }
            None
        }
    }
}

/// True if `ptr` (screen) lies inside the rip's selection.
pub fn contains_point(rip: &Rip, x: &Xform, ptr: Pos2) -> bool {
    match &rip.shape {
        RipShape::Quad(c) => point_in_quad(c, x, ptr),
        RipShape::Circle { center, radius } => x.screen_to_local(ptr).distance(*center) <= *radius,
    }
}

fn point_in_quad(c: &[Pos2; 4], x: &Xform, ptr: Pos2) -> bool {
    // Even-odd ray cast in screen space.
    let pts: [Pos2; 4] = [
        x.local_to_screen(c[0]),
        x.local_to_screen(c[1]),
        x.local_to_screen(c[2]),
        x.local_to_screen(c[3]),
    ];
    let mut inside = false;
    let mut j = 3;
    for i in 0..4 {
        let pi = pts[i];
        let pj = pts[j];
        if ((pi.y > ptr.y) != (pj.y > ptr.y))
            && (ptr.x < (pj.x - pi.x) * (ptr.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Applies a drag of the given handle to the rip and marks it dirty.
pub fn apply(rip: &mut Rip, handle: DragHandle, x: &Xform, ptr: Pos2, delta: Vec2) {
    let local = x.screen_to_local(ptr);
    let local_delta = delta / x.zoom;

    match (&mut rip.shape, handle) {
        (RipShape::Quad(c), DragHandle::QuadCorner(i)) => {
            c[i] = local;
        }
        (RipShape::Quad(c), DragHandle::QuadEdge(i)) => {
            c[i] += local_delta;
            c[(i + 1) % 4] += local_delta;
        }
        (RipShape::Quad(c), DragHandle::QuadMove) => {
            for corner in c.iter_mut() {
                *corner += local_delta;
            }
        }
        (RipShape::Circle { center, .. }, DragHandle::CircleCenter) => {
            *center = local;
        }
        (RipShape::Circle { center, .. }, DragHandle::CircleMove) => {
            *center += local_delta;
        }
        (RipShape::Circle { center, radius }, DragHandle::CircleRadius) => {
            *radius = (local - *center).length().max(1.0);
        }
        _ => {}
    }
    rip.dirty = true;
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draws a rip's outline; `selected` adds editable handles.
pub fn draw_rip(rip: &Rip, painter: &Painter, x: &Xform, selected: bool) {
    let color = if selected {
        Color32::from_rgb(255, 180, 0)
    } else {
        Color32::from_rgb(120, 200, 255)
    };
    let stroke = Stroke::new(if selected { 1.5 } else { 1.0 }, color);

    match &rip.shape {
        RipShape::Quad(c) => {
            let s: Vec<Pos2> = c.iter().map(|p| x.local_to_screen(*p)).collect();
            for i in 0..4 {
                painter.line_segment([s[i], s[(i + 1) % 4]], stroke);
            }
            if selected {
                // Only the corner vertices get handle dots; edges are dragged by
                // grabbing the edge line itself (no midpoint dot needed).
                for p in &s {
                    handle_dot(painter, *p);
                }
            }
        }
        RipShape::Circle { center, radius } => {
            let cs = x.local_to_screen(*center);
            painter.circle_stroke(cs, radius * x.zoom, stroke);
            if selected {
                handle_dot(painter, cs);
                handle_dot(painter, x.local_to_screen(*center + Vec2::new(*radius, 0.0)));
            }
        }
    }
}

/// Draws perspective-interpolated subdivision lines inside a quad selection.
pub fn draw_guides(c: &[Pos2; 4], painter: &Painter, x: &Xform, vertical: u32, horizontal: u32) {
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 180, 0, 110));
    // Interior vertical lines: interpolate along the top (TL->TR) and bottom
    // (BL->BR) edges and connect.
    for i in 1..=vertical {
        let t = i as f32 / (vertical + 1) as f32;
        let top = c[0] + (c[1] - c[0]) * t;
        let bot = c[3] + (c[2] - c[3]) * t;
        painter.line_segment([x.local_to_screen(top), x.local_to_screen(bot)], stroke);
    }
    // Interior horizontal lines: interpolate along the left (TL->BL) and right
    // (TR->BR) edges and connect.
    for i in 1..=horizontal {
        let t = i as f32 / (horizontal + 1) as f32;
        let left = c[0] + (c[3] - c[0]) * t;
        let right = c[1] + (c[2] - c[1]) * t;
        painter.line_segment([x.local_to_screen(left), x.local_to_screen(right)], stroke);
    }
}

/// The cursor to show while hovering or dragging a given handle.
pub fn handle_cursor(handle: DragHandle) -> egui::CursorIcon {
    use egui::CursorIcon;
    match handle {
        // Corner vertices use a precision (crosshair) cursor.
        DragHandle::QuadCorner(_) => CursorIcon::Crosshair,
        // Top/bottom edges resize vertically; left/right edges horizontally.
        DragHandle::QuadEdge(0) | DragHandle::QuadEdge(2) => CursorIcon::ResizeVertical,
        DragHandle::QuadEdge(_) => CursorIcon::ResizeHorizontal,
        DragHandle::CircleRadius => CursorIcon::ResizeHorizontal,
        DragHandle::CircleCenter | DragHandle::QuadMove | DragHandle::CircleMove => {
            CursorIcon::Move
        }
        DragHandle::None => CursorIcon::Default,
    }
}

fn handle_dot(painter: &Painter, p: Pos2) {
    let r = Rect::from_center_size(p, Vec2::splat(8.0));
    painter.rect_filled(r, 1.0, Color32::WHITE);
    painter.rect_stroke(
        r,
        1.0,
        Stroke::new(1.0, Color32::from_gray(40)),
        StrokeKind::Middle,
    );
}

// ---------------------------------------------------------------------------
// Live recomputation
// ---------------------------------------------------------------------------

/// Live-preview scale for cheap, appearance-only rip edits (brightness / filters
/// / colour key), derived from the geometry-warp `quality`. Recolouring is far
/// lighter than the perspective warp, so its live preview can afford more
/// resolution — a bit higher than `quality`, floored so it never gets too coarse,
/// and capped at full.
pub fn edit_preview_scale(quality: f32) -> f32 {
    (quality * 1.6).clamp(0.6, 1.0)
}

/// Renders a rip's full-resolution output — un-warp (quad) or crop (circle), then
/// colour, filters, resize and orientation. Pure CPU (no GPU / texture upload), so
/// it can run on a background thread (see [`crate::render::RipRenderer`]). Returns
/// `(size, pixels)`, or `None` for a degenerate selection.
pub fn render_full(
    src: &image::RgbaImage,
    shape: &RipShape,
    adjust: &crate::project::Adjustments,
    orient: &crate::project::Orientation,
    resize: Option<[u32; 2]>,
) -> Option<([usize; 2], image::RgbaImage)> {
    let target = match (shape, resize) {
        (_, Some(t)) => Some(t),
        (RipShape::Quad(c), None) => {
            let (w, h) = crate::warp::natural_size(*c);
            Some([w, h])
        }
        (RipShape::Circle { .. }, None) => None,
    };
    let mut rgba = match shape {
        RipShape::Quad(c) => crate::warp::unwarp_quad(src, *c, 1.0)?,
        RipShape::Circle { center, radius } => extract_circle(src, *center, *radius)?,
    };
    crate::image_edit::apply_adjustments(&mut rgba, adjust);
    rgba = crate::image_edit::apply_filters(rgba, adjust);
    if let Some(target) = target {
        rgba = crate::image_edit::resize_to(&rgba, target);
    }
    let base = [rgba.width() as usize, rgba.height() as usize];
    rgba = crate::image_edit::apply_orientation(rgba, orient);
    let size = crate::image_edit::oriented_size(base, orient);
    Some((size, rgba))
}

/// Recomputes the output of every dirty rip (un-warping quads, masking circles)
/// and marks the atlas dirty if anything changed.
///
/// `preview_scale` is `Some(scale)` to warp quads at a reduced output resolution
/// to keep interaction responsive — each previewed rip is flagged (`Rip::previewed`)
/// so `app` can render it at full resolution on a background thread once the user
/// settles — or `None` for an immediate full-resolution pass.
/// Returns true if any dirty rip was (re)computed this call — `app` uses it to
/// restart a background full-res render so a stale in-flight result can't
/// overwrite a freshly-edited rip's preview.
pub fn recompute_dirty(
    ctx: &egui::Context,
    project: &mut Project,
    preview_scale: Option<f32>,
) -> bool {
    let Project {
        rips,
        images,
        atlas_dirty,
        ..
    } = project;
    // Some(scale) → downscaled live preview; None → full resolution.
    let preview = preview_scale.is_some();
    let preview_scale = preview_scale.unwrap_or(1.0).clamp(0.05, 1.0);
    let mut changed = false;

    for rip in rips.iter_mut() {
        if !rip.dirty {
            continue;
        }
        rip.dirty = false;
        *atlas_dirty = true;
        changed = true;

        if rip.image >= images.len() {
            rip.output = None;
            continue;
        }
        let img = &images[rip.image];

        // The output size the atlas should see, independent of preview quality:
        // an explicit resize override, else the quad's natural un-warp size.
        let target = match (&rip.shape, rip.resize) {
            (_, Some(t)) => Some(t),
            (RipShape::Quad(c), None) => {
                let (w, h) = crate::warp::natural_size(*c);
                Some([w, h])
            }
            (RipShape::Circle { .. }, None) => None,
        };

        let result = match &rip.shape {
            RipShape::Quad(c) => {
                if preview {
                    // Sample a downscaled mip and warp at reduced output res. The
                    // result is scaled back up to `target` below, so the atlas
                    // footprint is unchanged while the per-pixel cost drops and
                    // the source read stays cache-friendly / anti-aliased.
                    let (ms, msrc) = img.preview_source(preview_scale);
                    let scaled = (*c).map(|p| Pos2::new(p.x * ms, p.y * ms));
                    crate::warp::unwarp_quad(msrc, scaled, preview_scale / ms)
                } else {
                    crate::warp::unwarp_quad(&img.pixels, *c, 1.0)
                }
            }
            RipShape::Circle { center, radius } => extract_circle(&img.pixels, *center, *radius),
        };

        rip.output = result.map(|mut rgba| {
            crate::image_edit::apply_adjustments(&mut rgba, &rip.adjust);
            rgba = crate::image_edit::apply_filters(rgba, &rip.adjust);
            // The atlas footprint is driven by the *declared* `size`, so we keep
            // that stable at `target` either way (no layout jump on drag). For a
            // full-res pass we actually resize the pixels to land there; during a
            // preview we leave the pixels small and only report `target`, skipping
            // the expensive per-frame upscale that made dragging lag.
            let base = if preview {
                target
                    .map(|t| [t[0] as usize, t[1] as usize])
                    .unwrap_or([rgba.width() as usize, rgba.height() as usize])
            } else {
                if let Some(target) = target {
                    rgba = crate::image_edit::resize_to(&rgba, target);
                }
                [rgba.width() as usize, rgba.height() as usize]
            };
            // Rotation / mirroring is applied last; a 90°/270° turn swaps the size.
            rgba = crate::image_edit::apply_orientation(rgba, &rip.orient);
            let size = crate::image_edit::oriented_size(base, &rip.orient);
            let texture = crate::texture_view::upload_texture(ctx, &rip.name, &rgba);
            RipOutput {
                size,
                texture,
                pixels: rgba,
            }
        });
        // Mark whether this output is a (low-res) preview awaiting a full-res
        // render. `app` kicks the background renderer for rips still flagged here.
        rip.previewed = preview;
    }
    changed
}

/// Crops a circular region and masks out everything outside the radius.
fn extract_circle(src: &image::RgbaImage, center: Pos2, radius: f32) -> Option<image::RgbaImage> {
    let iw = src.width() as i64;
    let ih = src.height() as i64;
    let x0 = ((center.x - radius).round() as i64).clamp(0, iw);
    let y0 = ((center.y - radius).round() as i64).clamp(0, ih);
    let x1 = ((center.x + radius).round() as i64).clamp(0, iw);
    let y1 = ((center.y + radius).round() as i64).clamp(0, ih);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut sub = image::imageops::crop_imm(src, x0 as u32, y0 as u32, w, h).to_image();

    let cx = center.x - x0 as f32;
    let cy = center.y - y0 as f32;
    let r2 = radius * radius;
    for (px, py, pixel) in sub.enumerate_pixels_mut() {
        let dx = px as f32 + 0.5 - cx;
        let dy = py as f32 + 0.5 - cy;
        if dx * dx + dy * dy > r2 {
            pixel[3] = 0;
        }
    }
    Some(sub)
}
