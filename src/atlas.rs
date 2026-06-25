//! Atlas packing & preview.
//!
//! Rips are arranged into a single texture in one of two **sort modes**
//! ([`SortMode`], chosen like the aspect-ratio mode):
//! - **Automatic** — `rectangle-pack` bin-packs every rip. This never touches the
//!   user's aspect-ratio choice or per-rip stretching (`resize`).
//! - **Manual** — the user drags each rip in the preview; positions persist in
//!   `Rip::atlas_pos` and are seeded from the automatic layout on first switch.
//!
//! The preview is pan/zoomable (scroll to zoom, middle-drag to pan); the
//! transparency checkerboard is culled to the export-bounds canvas and clipped to
//! the panel. Repacks run live whenever `Project::atlas_dirty` is set.

use std::collections::{BTreeMap, HashMap};

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use image::RgbaImage;
use rectangle_pack::{
    contains_smallest_box, pack_rects, volume_heuristic, GroupedRectsToPlace, RectToInsert,
    TargetBin,
};

use crate::project::{AspectMode, AtlasPlacement, AtlasResult, AtlasView, Project, Rip, SortMode};

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

/// Packs all rip outputs into the atlas (positions depend on the sort mode).
///
/// This only computes per-rip *placements* and the natural packed size — it never
/// composites a pixel image (that happens lazily at export), so a repack stays
/// cheap enough to run live on every change.
pub fn repack(project: &mut Project) {
    let Project {
        rips,
        atlas,
        status,
        status_error,
        ..
    } = project;

    let settings = atlas.settings;

    // Collect rips that currently have an output: (rip index, natural w, h).
    let items: Vec<(usize, u32, u32)> = rips
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.output.as_ref().map(|o| (i, o.size[0] as u32, o.size[1] as u32)))
        .collect();

    if items.is_empty() {
        atlas.result = None;
        return;
    }

    let pad = settings.padding;

    // The size each rip *occupies* in the atlas. In Automatic sort, non-custom
    // rips are scaled toward a fair common size so a few big rips don't dominate
    // the layout; rips with an explicit output size (`resize`) are kept exact.
    // Manual sort always uses the natural sizes (the user arranges them).
    let pack_items: Vec<(usize, u32, u32)> = match settings.sort_mode {
        SortMode::Automatic => scaled_pack_items(rips, &items),
        SortMode::Manual => items.clone(),
    };

    // Per-rip top-left positions (atlas pixels), depending on the sort mode.
    let positions = match settings.sort_mode {
        SortMode::Automatic => {
            // Pack toward the user's chosen aspect so the rips fill it tightly:
            // Square -> 1:1, Custom -> the export W:H, Automatic -> square-ish
            // (the bounds then re-derive to the packed aspect below).
            let target_aspect = match settings.aspect_mode {
                AspectMode::Square => 1.0,
                AspectMode::Automatic => 1.0,
                AspectMode::Custom => {
                    let w = if settings.export_w == 0 { 1 } else { settings.export_w };
                    let h = if settings.export_h == 0 { 1 } else { settings.export_h };
                    w as f32 / h.max(1) as f32
                }
            };
            match pack_aspect(&pack_items, pad, target_aspect) {
                Some(p) => p,
                None => {
                    *status = Some("Atlas: too many rips to pack.".to_string());
                    *status_error = true;
                    atlas.result = None;
                    return;
                }
            }
        }
        SortMode::Manual => manual_positions(rips, &pack_items, pad),
    };

    let size_of: HashMap<usize, (u32, u32)> =
        pack_items.iter().map(|&(i, w, h)| (i, (w, h))).collect();

    let mut placements = Vec::new();
    let mut aw = 0u32;
    let mut ah = 0u32;
    for &(i, _, _) in &pack_items {
        let (w, h) = size_of[&i];
        let (x, y) = positions.get(&i).copied().unwrap_or((0, 0));
        aw = aw.max(x + w);
        ah = ah.max(y + h);
        placements.push(AtlasPlacement { rip: i, x, y, w, h });
    }

    atlas.result = Some(AtlasResult {
        size: [aw as usize, ah as usize],
        placements,
        used_count: items.len(),
    });

    // In Automatic *aspect* the bounds hug the packed atlas, so a repack (e.g.
    // after a rip resize or a manual move) must re-derive the stored height from
    // the kept width — otherwise stale padding lingers. Square is forced W=H in
    // the panel each frame; Custom is intentionally free. This only ever runs for
    // the user's own Automatic-aspect choice, so it never clobbers Custom/Square.
    if matches!(settings.aspect_mode, AspectMode::Automatic) && atlas.settings.export_w != 0 {
        let nat_aspect = aw as f32 / ah.max(1) as f32;
        atlas.settings.export_h =
            ((atlas.settings.export_w as f32 / nat_aspect.max(0.0001)).round() as u32).max(1);
    }
}

/// Scales the *non-custom* rips toward a fair common size so a tight automatic
/// pack distributes space evenly (a single huge rip no longer dominates). Rips
/// with an explicit output size (`resize`) are returned unchanged.
///
/// "Fair" means every non-custom rip is scaled (up or down, capped to a 4x range)
/// so its longest side approaches a reference dimension: the average longest side
/// of the custom rips when any exist (so non-custom rips match their scale), else
/// the median longest side of all rips.
fn scaled_pack_items(rips: &[Rip], items: &[(usize, u32, u32)]) -> Vec<(usize, u32, u32)> {
    let long = |w: u32, h: u32| w.max(h) as f32;
    let is_custom = |i: usize| rips.get(i).is_some_and(|r| r.resize.is_some());

    let custom_longs: Vec<f32> = items
        .iter()
        .filter(|&&(i, _, _)| is_custom(i))
        .map(|&(_, w, h)| long(w, h))
        .collect();
    let reference = if custom_longs.is_empty() {
        let all: Vec<f32> = items.iter().map(|&(_, w, h)| long(w, h)).collect();
        median(&all)
    } else {
        custom_longs.iter().sum::<f32>() / custom_longs.len() as f32
    }
    .max(1.0);

    items
        .iter()
        .map(|&(i, w, h)| {
            if is_custom(i) {
                return (i, w, h);
            }
            let factor = (reference / long(w, h).max(1.0)).clamp(0.25, 4.0);
            let nw = ((w as f32 * factor).round() as u32).clamp(1, 8192);
            let nh = ((h as f32 * factor).round() as u32).clamp(1, 8192);
            (i, nw, nh)
        })
        .collect()
}

/// Median of a slice (returns 1.0 for empty input).
fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 1.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) * 0.5
    }
}

/// Packs `items` (each inflated by `pad`) into the smallest bin of roughly
/// `target_aspect` (w/h) that fits, so the result is tight and fills the chosen
/// aspect instead of sprawling into a thin strip. Starts from the area lower
/// bound and grows geometrically until the pack succeeds.
fn pack_aspect(
    items: &[(usize, u32, u32)],
    pad: u32,
    target_aspect: f32,
) -> Option<HashMap<usize, (u32, u32)>> {
    let aspect = target_aspect.clamp(0.05, 20.0);
    let total: f64 = items
        .iter()
        .map(|&(_, w, h)| (w as f64 + pad as f64) * (h as f64 + pad as f64))
        .sum();
    let widest = items.iter().map(|&(_, w, _)| w + pad).max().unwrap_or(1);
    let tallest = items.iter().map(|&(_, _, h)| h + pad).max().unwrap_or(1);

    // height from area: w*h = (h*aspect)*h => h = sqrt(area / aspect).
    let mut bin_h = ((total / aspect as f64).sqrt().ceil() as u32).max(tallest);
    for _ in 0..64 {
        let h = bin_h.max(tallest);
        let w = (((h as f32) * aspect).ceil() as u32).max(widest);
        if w > MAX_BIN || h > MAX_BIN {
            break;
        }
        if let Some(p) = try_pack(items, pad, w, h) {
            return Some(p);
        }
        bin_h = ((h as f32) * 1.1).ceil() as u32 + 1;
    }
    // Last resort: one big square bin (matches the pre-1.2 behaviour).
    try_pack(items, pad, MAX_BIN, MAX_BIN)
}

/// One bin-pack attempt into a `bin_w`×`bin_h` bin; `None` if it doesn't fit.
/// Returns each rip's top-left (the `pad` becomes spacing to its right/bottom).
fn try_pack(
    items: &[(usize, u32, u32)],
    pad: u32,
    bin_w: u32,
    bin_h: u32,
) -> Option<HashMap<usize, (u32, u32)>> {
    let mut rects = GroupedRectsToPlace::<usize, ()>::new();
    for &(i, w, h) in items {
        rects.push_rect(i, None, RectToInsert::new(w + pad, h + pad, 1));
    }
    let mut bins = BTreeMap::new();
    bins.insert((), TargetBin::new(bin_w.max(1), bin_h.max(1), 1));
    let packed = pack_rects(&rects, &mut bins, &volume_heuristic, &contains_smallest_box).ok()?;
    Some(
        packed
            .packed_locations()
            .iter()
            .map(|(&i, (_bin, loc))| (i, (loc.x(), loc.y())))
            .collect(),
    )
}

/// Bin-packs `items` square-ish; used for the Manual fallback for un-positioned
/// rips.
fn bin_pack(items: &[(usize, u32, u32)], pad: u32) -> Option<HashMap<usize, (u32, u32)>> {
    pack_aspect(items, pad, 1.0)
}

/// Manual placements: each rip's stored `atlas_pos`, with a bin-packed fallback
/// for any rip not yet positioned (e.g. newly added while in Manual mode).
fn manual_positions(
    rips: &[Rip],
    items: &[(usize, u32, u32)],
    pad: u32,
) -> HashMap<usize, (u32, u32)> {
    let fallback = bin_pack(items, pad).unwrap_or_default();
    let mut map = HashMap::new();
    for &(i, _, _) in items {
        let pos = match rips.get(i).and_then(|r| r.atlas_pos) {
            Some([x, y]) => (x.max(0.0).round() as u32, y.max(0.0).round() as u32),
            None => fallback.get(&i).copied().unwrap_or((0, 0)),
        };
        map.insert(i, pos);
    }
    map
}

/// Snaps a Manual-drag position (`raw`, atlas-px top-left of a `w`×`h` rip) to
/// nearby edges and/or a grid. Edges win when one is within `edge_thresh`; each
/// axis falls back to the grid step otherwise. Edge targets are the canvas origin
/// and every *other* rip's left/right (x) and top/bottom (y) edges. Returns `raw`
/// (clamped to ≥0) unchanged when snapping is off.
#[allow(clippy::too_many_arguments)]
fn snap_manual_pos(
    raw: [f32; 2],
    w: f32,
    h: f32,
    moving: usize,
    placements: &[AtlasPlacement],
    snap_enabled: bool,
    step: u32,
    edge_thresh: f32,
) -> [f32; 2] {
    let mut pos = [raw[0].max(0.0), raw[1].max(0.0)];
    if !snap_enabled {
        return pos;
    }

    let mut xs = vec![0.0f32];
    let mut ys = vec![0.0f32];
    for p in placements {
        if p.rip == moving {
            continue;
        }
        xs.push(p.x as f32);
        xs.push((p.x + p.w) as f32);
        ys.push(p.y as f32);
        ys.push((p.y + p.h) as f32);
    }

    // For one axis: try to align either the near or far edge of the moving rip to
    // a target line (nearest within threshold), else snap to the grid.
    let snap_axis = |v: f32, size: f32, targets: &[f32]| -> f32 {
        let mut best: Option<f32> = None;
        for &m in &[v, v + size] {
            for &t in targets {
                let d = t - m;
                if d.abs() <= edge_thresh && best.map_or(true, |b: f32| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
        }
        match best {
            Some(d) => v + d,
            None if step > 0 => (v / step as f32).round() * step as f32,
            None => v,
        }
    };

    pos[0] = snap_axis(pos[0], w, &xs).max(0.0);
    pos[1] = snap_axis(pos[1], h, &ys).max(0.0);
    pos
}

/// Snaps a single coordinate `v` to the nearest `targets` line within
/// `edge_thresh`, else to the grid `step`. Used by the resize grip to align a
/// rip's right / bottom edge.
fn snap_coord(v: f32, targets: &[f32], step: u32, edge_thresh: f32) -> f32 {
    let mut best: Option<f32> = None;
    for &t in targets {
        let d = t - v;
        if d.abs() <= edge_thresh && best.map_or(true, |b: f32| d.abs() < b.abs()) {
            best = Some(d);
        }
    }
    match best {
        Some(d) => v + d,
        None if step > 0 => (v / step as f32).round() * step as f32,
        None => v,
    }
}

/// Seeds `atlas_pos` for any rip that hasn't got one from the current packed
/// placements, so switching to Manual starts exactly where Automatic left off.
/// Already-positioned rips are left untouched (toggling modes never loses a
/// hand-made layout).
pub fn seed_manual_positions(project: &mut Project) {
    let seeds: Vec<(usize, [f32; 2])> = match &project.atlas.result {
        Some(res) => res
            .placements
            .iter()
            .filter(|p| project.rips.get(p.rip).is_some_and(|r| r.atlas_pos.is_none()))
            .map(|p| (p.rip, [p.x as f32, p.y as f32]))
            .collect(),
        None => Vec::new(),
    };
    for (i, pos) in seeds {
        if let Some(r) = project.rips.get_mut(i) {
            r.atlas_pos = Some(pos);
        }
    }
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

/// Atlas View panel UI: settings, actions, and a pan/zoomable preview.
pub fn ui(ui: &mut egui::Ui, project: &mut Project) {
    // Natural (packed) size, snapshotted before we mutably borrow settings.
    let natural = project
        .atlas
        .result
        .as_ref()
        .map(|r| (r.size[0] as u32, r.size[1] as u32));
    // Mode is read from last frame's value so the row below (which can change it)
    // takes effect next frame — avoids editing it mid-row.
    let aspect_mode = project.atlas.settings.aspect_mode;

    // Row 1: Padding, Aspect ratio, Width, Height, Export, rip count.
    ui.horizontal(|ui| {
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

        if let Some((nat_w, nat_h)) = natural {
            // Current export dims (`0` follows the natural packed size).
            let cur_w = if project.atlas.settings.export_w == 0 {
                nat_w
            } else {
                project.atlas.settings.export_w
            };
            let cur_h = if project.atlas.settings.export_h == 0 {
                nat_h
            } else {
                project.atlas.settings.export_h
            };

            ui.separator();
            ui.label("Aspect Ratio");
            match aspect_mode {
                // Editable in Custom: changing it recomputes the height from width.
                AspectMode::Custom => {
                    let mut a = cur_w as f32 / cur_h.max(1) as f32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut a)
                                .range(0.05..=20.0)
                                .speed(0.01)
                                .max_decimals(2),
                        )
                        .changed()
                    {
                        project.atlas.settings.export_w = cur_w.max(1);
                        project.atlas.settings.export_h =
                            ((cur_w as f32 / a.max(0.01)).round() as u32).max(1);
                    }
                }
                // Locked (read-only) in Automatic/Square.
                AspectMode::Square => {
                    let mut a = 1.0_f32;
                    ui.add_enabled(false, egui::DragValue::new(&mut a).max_decimals(2));
                }
                AspectMode::Automatic => {
                    let mut a = nat_w as f32 / nat_h.max(1) as f32;
                    ui.add_enabled(false, egui::DragValue::new(&mut a).max_decimals(2));
                }
            }

            ui.separator();
            match aspect_mode {
                AspectMode::Automatic => {
                    // Aspect locked to packed atlas; editing one updates the other.
                    let aspect = nat_w as f32 / nat_h.max(1) as f32;
                    let mut w = cur_w;
                    ui.label("Width");
                    if ui
                        .add(egui::DragValue::new(&mut w).range(1..=16384).suffix(" px"))
                        .changed()
                    {
                        project.atlas.settings.export_w = w.max(1);
                        project.atlas.settings.export_h = ((w as f32 / aspect).round() as u32).max(1);
                    }
                    let mut h = cur_h;
                    ui.label("Height");
                    if ui
                        .add(egui::DragValue::new(&mut h).range(1..=16384).suffix(" px"))
                        .changed()
                    {
                        project.atlas.settings.export_h = h.max(1);
                        project.atlas.settings.export_w = ((h as f32 * aspect).round() as u32).max(1);
                    }
                }
                AspectMode::Square => {
                    // Both fields edit one value, kept equal.
                    let mut side = if project.atlas.settings.export_w != 0 {
                        project.atlas.settings.export_w
                    } else {
                        nat_w.max(nat_h)
                    };
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut side).range(1..=16384).suffix(" px"));
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut side).range(1..=16384).suffix(" px"));
                    project.atlas.settings.export_w = side.max(1);
                    project.atlas.settings.export_h = side.max(1);
                }
                AspectMode::Custom => {
                    // Independent width / height.
                    let mut w = cur_w;
                    ui.label("Width");
                    if ui
                        .add(egui::DragValue::new(&mut w).range(1..=16384).suffix(" px"))
                        .changed()
                    {
                        project.atlas.settings.export_w = w.max(1);
                        if project.atlas.settings.export_h == 0 {
                            project.atlas.settings.export_h = nat_h;
                        }
                    }
                    let mut h = cur_h;
                    ui.label("Height");
                    if ui
                        .add(egui::DragValue::new(&mut h).range(1..=16384).suffix(" px"))
                        .changed()
                    {
                        project.atlas.settings.export_h = h.max(1);
                        if project.atlas.settings.export_w == 0 {
                            project.atlas.settings.export_w = nat_w;
                        }
                    }
                }
            }

            ui.separator();
            if ui.button("Export").clicked() {
                export(project);
            }
            let used = project
                .atlas
                .result
                .as_ref()
                .map(|r| r.used_count)
                .unwrap_or(0);
            ui.label(format!("{used} rip(s)"));
        } else {
            ui.separator();
            ui.weak("no atlas (add some rips)");
            ui.label(format!("{} rip(s)", project.rips.len()));
        }
    });

    // Row 2: sort mode, aspect-ratio mode, and view controls.
    let prev_sort = project.atlas.settings.sort_mode;
    ui.horizontal(|ui| {
        ui.label("Sort");
        egui::ComboBox::from_id_salt("atlas_sort_mode")
            .selected_text(match project.atlas.settings.sort_mode {
                SortMode::Automatic => "Automatic",
                SortMode::Manual => "Manual",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut project.atlas.settings.sort_mode,
                    SortMode::Automatic,
                    "Automatic",
                );
                ui.selectable_value(
                    &mut project.atlas.settings.sort_mode,
                    SortMode::Manual,
                    "Manual",
                );
            });

        ui.separator();
        ui.label("Aspect Ratio");
        egui::ComboBox::from_id_salt("atlas_aspect_mode")
            .selected_text(match project.atlas.settings.aspect_mode {
                AspectMode::Automatic => "Automatic",
                AspectMode::Square => "Square",
                AspectMode::Custom => "Custom",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut project.atlas.settings.aspect_mode,
                    AspectMode::Automatic,
                    "Automatic",
                );
                ui.selectable_value(
                    &mut project.atlas.settings.aspect_mode,
                    AspectMode::Square,
                    "Square",
                );
                ui.selectable_value(
                    &mut project.atlas.settings.aspect_mode,
                    AspectMode::Custom,
                    "Custom",
                );
            });

        ui.separator();
        if ui.button("Reset view").clicked() {
            project.atlas.view = AtlasView::default();
        }
        ui.label(format!("{:.0}%", project.atlas.view.zoom * 100.0));
        if matches!(project.atlas.settings.sort_mode, SortMode::Manual) {
            ui.separator();
            ui.checkbox(&mut project.atlas.settings.snap_enabled, "Snap")
                .on_hover_text("Snap dragged rips to a grid and to nearby rip / canvas edges.");
            if project.atlas.settings.snap_enabled {
                ui.add(
                    egui::DragValue::new(&mut project.atlas.settings.snap_step)
                        .range(1..=256)
                        .suffix(" px"),
                )
                .on_hover_text("Grid step");
            }
            ui.separator();
            ui.weak("drag rips to arrange");
        }
    });

    // A sort-mode change repacks; entering Manual seeds positions from the
    // current automatic layout (existing manual positions are preserved).
    if project.atlas.settings.sort_mode != prev_sort {
        project.atlas_dirty = true;
        project.modified = true;
        if matches!(project.atlas.settings.sort_mode, SortMode::Manual) {
            seed_manual_positions(project);
        }
    }

    ui.separator();

    preview(ui, project);
}

/// Draws the pan/zoomable packed-atlas preview. Clicking a rip selects it; the
/// selected rip has a bottom-right resize grip (drag to set its output size). In
/// Manual sort, dragging a rip's body repositions it (committed on release).
fn preview(ui: &mut egui::Ui, project: &mut Project) {
    let dark = ui.visuals().dark_mode;
    let selected = project.editor.selected;
    let manual = matches!(project.atlas.settings.sort_mode, SortMode::Manual);
    // Grab tuning shared with the Texture View, plus Manual-snap settings.
    let margin = project.cursor_margin;
    let shift = ui.input(|i| i.modifiers.shift);
    let snap_enabled = project.atlas.settings.snap_enabled;
    let snap_step = project.atlas.settings.snap_step;

    let mut new_resize: Option<(usize, [u32; 2])> = None;
    let mut new_select: Option<usize> = None;
    let mut new_move: Option<(usize, [f32; 2])> = None;
    // Set when a body-drag in Automatic sort should flip the project to Manual.
    let mut switch_to_manual = false;

    // Pan/zoom is mutated on a copy so the atlas result can stay immutably
    // borrowed for drawing; it's written back after the borrow ends.
    let mut view = project.atlas.view;

    // Export dimensions define the output texture *bounds* (aspect ratio / W·H).
    let export = export_size(project);

    if let Some(res) = &project.atlas.result {
        let (nat_w, nat_h) = (res.size[0] as f32, res.size[1] as f32);
        let [ew, eh] = export.unwrap_or([res.size[0] as u32, res.size[1] as u32]);
        let (ew_f, eh_f) = (ew as f32, eh as f32);

        let avail = ui.available_size();
        let avail_w = avail.x.max(64.0);
        let avail_h = avail.y.max(64.0);
        let (outer_rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_w, avail_h), Sense::click_and_drag());
        // Clip to the panel so zoomed-in content / the background never paint out.
        let painter = ui.painter_at(outer_rect);

        // Baseline scale that fits the export bounds in the panel at zoom 1.
        let fit = (avail_w / ew_f).min(avail_h / eh_f).min(8.0);
        let half = Vec2::new(ew_f, eh_f) * 0.5;

        // --- Zoom around the pointer (scroll / pinch) ---
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
                    let s = fit * view.zoom;
                    let origin = outer_rect.center() + view.pan - half * s;
                    let world = (ptr - origin) / s;
                    let new_zoom = (view.zoom * factor).clamp(0.1, 40.0);
                    let new_s = fit * new_zoom;
                    let new_origin = ptr - world * new_s;
                    view.pan = (new_origin + half * new_s) - outer_rect.center();
                    view.zoom = new_zoom;
                }
            }
        }

        // --- Middle-drag pans ---
        if response.dragged_by(egui::PointerButton::Middle) {
            view.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        // Final transform after pan/zoom.
        let s = fit * view.zoom;
        let canvas_rect = Rect::from_center_size(
            outer_rect.center() + view.pan,
            Vec2::new(ew_f, eh_f) * s,
        );
        let origin = canvas_rect.min;
        // Uniform natural->screen scale: contain the packed atlas in the bounds.
        let u = s * (ew_f / nat_w).min(eh_f / nat_h);
        // Edge-snap threshold in atlas px (a fixed ~8 screen px).
        let edge_thresh = (8.0 / u).max(0.5);

        // Background checkerboard, culled to the atlas canvas (export bounds) and
        // clipped to the panel. Phase tracks the canvas top-left so the pattern
        // moves with the bounds as the preview pans/zooms.
        crate::texture_view::paint_checkerboard_clipped(
            &painter,
            canvas_rect,
            outer_rect,
            canvas_rect.min,
            dark,
        );
        painter.rect_stroke(
            canvas_rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(if dark { 90 } else { 120 })),
            StrokeKind::Outside,
        );

        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let cell_of = |p: &AtlasPlacement| {
            Rect::from_min_size(
                origin + Vec2::new(p.x as f32 * u, p.y as f32 * u),
                Vec2::new(p.w as f32 * u, p.h as f32 * u),
            )
        };

        // Draw each rip's own texture into its placement cell.
        for p in &res.placements {
            if let Some(out) = project.rips.get(p.rip).and_then(|r| r.output.as_ref()) {
                painter.image(out.texture.id(), cell_of(p), uv, Color32::WHITE);
            }
        }

        // Selected rip's placement + bottom-right resize grip. The grip is drawn
        // small (`handle_rect`) but grabbed over a larger zone (`handle_hit`,
        // sized from `cursor_margin`) so it's as easy to catch as the Texture
        // View's handles.
        let sel_placement = res.placements.iter().find(|p| selected == Some(p.rip));
        let grip_center = sel_placement.map(|p| {
            origin + Vec2::new((p.x + p.w) as f32 * u, (p.y + p.h) as f32 * u)
        });
        let handle_rect = grip_center.map(|br| Rect::from_center_size(br, Vec2::splat(12.0)));
        let handle_hit =
            grip_center.map(|br| Rect::from_center_size(br, Vec2::splat((margin * 2.0).max(12.0))));

        // Outline each rip; the selected one is highlighted.
        for p in &res.placements {
            let stroke = if selected == Some(p.rip) {
                Stroke::new(2.0, Color32::from_rgb(255, 120, 0))
            } else {
                Stroke::new(1.0, Color32::from_rgb(255, 180, 0))
            };
            painter.rect_stroke(cell_of(p), 0.0, stroke, StrokeKind::Inside);
        }

        // --- Interaction temp state ---
        let grab_id = ui.id().with("atlas_resize_grab"); // bool: resizing
        let size_id = ui.id().with("atlas_resize_size"); // [u32;2]: pending size
        let move_id = ui.id().with("atlas_move_rip"); // Option<usize>: moving
        let mpend_id = ui.id().with("atlas_move_pend"); // [f32;2]: pending pos

        let mut resizing = ui.data(|d| d.get_temp::<bool>(grab_id).unwrap_or(false));
        let mut moving = ui.data(|d| d.get_temp::<Option<usize>>(move_id).flatten());

        // --- Drag start: resize-grip wins, else (Manual) move the cell under it ---
        if response.drag_started_by(egui::PointerButton::Primary) {
            resizing = false;
            moving = None;
            let pt = response.interact_pointer_pos();
            let on_grip = matches!((handle_hit, pt), (Some(hr), Some(p)) if hr.contains(p));
            if on_grip {
                if let Some(p) = sel_placement {
                    resizing = true;
                    ui.data_mut(|d| d.insert_temp(size_id, [p.w, p.h]));
                }
            } else if let Some(p) = pt {
                // Dragging a rip's body moves it. In Automatic sort this flips the
                // project to Manual (so the hand-placement sticks) — the layout is
                // seeded from the current automatic pack after the borrow ends.
                if let Some(pl) = res.placements.iter().rev().find(|pl| cell_of(pl).contains(p)) {
                    moving = Some(pl.rip);
                    new_select = Some(pl.rip);
                    ui.data_mut(|d| d.insert_temp(mpend_id, [pl.x as f32, pl.y as f32]));
                    if !manual {
                        switch_to_manual = true;
                    }
                }
            }
            ui.data_mut(|d| {
                d.insert_temp(grab_id, resizing);
                d.insert_temp(move_id, moving);
            });
        }

        // --- Drag in progress ---
        if resizing && response.dragged() {
            if let (Some(p), Some(pt)) = (sel_placement, response.interact_pointer_pos()) {
                let mut nw = ((pt.x - origin.x) / u - p.x as f32).clamp(1.0, 8192.0);
                let mut nh = ((pt.y - origin.y) / u - p.y as f32).clamp(1.0, 8192.0);
                // Shift locks the rip's current aspect ratio: the axis dragged
                // furthest (relative to its size) drives a uniform scale.
                if shift && p.w > 0 && p.h > 0 {
                    let aspect = p.w as f32 / p.h as f32;
                    let scale = (nw / p.w as f32).max(nh / p.h as f32);
                    nw = (p.w as f32 * scale).clamp(1.0, 8192.0);
                    nh = (nw / aspect).clamp(1.0, 8192.0);
                } else if snap_enabled {
                    // Snap the dragged corner (right/bottom edge) to a grid step or
                    // a nearby other-rip / canvas edge, then derive the new size.
                    let mut xs = vec![0.0f32];
                    let mut ys = vec![0.0f32];
                    for pl in &res.placements {
                        if pl.rip == p.rip {
                            continue;
                        }
                        xs.push(pl.x as f32);
                        xs.push((pl.x + pl.w) as f32);
                        ys.push(pl.y as f32);
                        ys.push((pl.y + pl.h) as f32);
                    }
                    let right = snap_coord(p.x as f32 + nw, &xs, snap_step, edge_thresh);
                    let bottom = snap_coord(p.y as f32 + nh, &ys, snap_step, edge_thresh);
                    nw = (right - p.x as f32).clamp(1.0, 8192.0);
                    nh = (bottom - p.y as f32).clamp(1.0, 8192.0);
                }
                ui.data_mut(|d| d.insert_temp(size_id, [nw.round() as u32, nh.round() as u32]));
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        } else if moving.is_some() && response.dragged() {
            let mut pend = ui.data(|d| d.get_temp::<[f32; 2]>(mpend_id)).unwrap_or([0.0, 0.0]);
            let dd = response.drag_delta() / u;
            pend[0] = (pend[0] + dd.x).max(0.0);
            pend[1] = (pend[1] + dd.y).max(0.0);
            ui.data_mut(|d| d.insert_temp(mpend_id, pend));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        // --- Hover cursor (when not dragging) ---
        if !response.dragged() {
            if let Some(hover) = response.hover_pos() {
                if handle_hit.is_some_and(|hr| hr.contains(hover)) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                } else if res.placements.iter().any(|pl| cell_of(pl).contains(hover)) {
                    // Body drag moves a rip in either sort mode (Automatic flips to
                    // Manual on drag), so show the grab cursor regardless.
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }

        // --- Resize grip + its live ghost outline ---
        if let (Some(p), Some(hr)) = (sel_placement, handle_rect) {
            painter.rect_filled(hr, 1.0, Color32::WHITE);
            painter.rect_stroke(
                hr,
                1.0,
                Stroke::new(1.0, Color32::from_gray(40)),
                StrokeKind::Middle,
            );
            if resizing {
                if let Some([gw, gh]) = ui.data(|d| d.get_temp::<[u32; 2]>(size_id)) {
                    let ghost = Rect::from_min_size(
                        origin + Vec2::new(p.x as f32 * u, p.y as f32 * u),
                        Vec2::new(gw as f32 * u, gh as f32 * u),
                    );
                    painter.rect_stroke(
                        ghost,
                        0.0,
                        Stroke::new(1.5, Color32::from_rgb(0, 200, 255)),
                        StrokeKind::Outside,
                    );
                }
            }
        }

        // --- Manual-move ghost: the moving rip drawn at its (snapped) position ---
        if let Some(mr) = moving {
            if let (Some(pl), Some(raw)) = (
                res.placements.iter().find(|p| p.rip == mr),
                ui.data(|d| d.get_temp::<[f32; 2]>(mpend_id)),
            ) {
                let pend = snap_manual_pos(
                    raw,
                    pl.w as f32,
                    pl.h as f32,
                    mr,
                    &res.placements,
                    snap_enabled,
                    snap_step,
                    edge_thresh,
                );
                let gr = Rect::from_min_size(
                    origin + Vec2::new(pend[0] * u, pend[1] * u),
                    Vec2::new(pl.w as f32 * u, pl.h as f32 * u),
                );
                if let Some(out) = project.rips.get(mr).and_then(|r| r.output.as_ref()) {
                    painter.image(out.texture.id(), gr, uv, Color32::from_white_alpha(200));
                }
                painter.rect_stroke(
                    gr,
                    0.0,
                    Stroke::new(1.5, Color32::from_rgb(0, 200, 255)),
                    StrokeKind::Outside,
                );
            }
        }

        // --- Commit on release ---
        if response.drag_stopped() {
            if resizing {
                if let (Some(p), Some(sz)) =
                    (sel_placement, ui.data(|d| d.get_temp::<[u32; 2]>(size_id)))
                {
                    new_resize = Some((p.rip, sz));
                }
            } else if let Some(mr) = moving {
                if let (Some(pl), Some(raw)) = (
                    res.placements.iter().find(|p| p.rip == mr),
                    ui.data(|d| d.get_temp::<[f32; 2]>(mpend_id)),
                ) {
                    let pend = snap_manual_pos(
                        raw,
                        pl.w as f32,
                        pl.h as f32,
                        mr,
                        &res.placements,
                        snap_enabled,
                        snap_step,
                        edge_thresh,
                    );
                    new_move = Some((mr, pend));
                }
            }
            ui.data_mut(|d| {
                d.insert_temp(grab_id, false);
                d.insert_temp(move_id, None::<usize>);
            });
        }

        // --- Click selects the top-most rip under the cursor ---
        if response.clicked() {
            if let Some(pt) = response.interact_pointer_pos() {
                if let Some(pl) = res.placements.iter().rev().find(|p| cell_of(p).contains(pt)) {
                    new_select = Some(pl.rip);
                }
            }
        }
    }

    project.atlas.view = view;

    // A body-drag in Automatic flips to Manual, seeding positions from the
    // current automatic pack so nothing jumps; the drag itself then repositions
    // the grabbed rip (committed on release).
    if switch_to_manual {
        project.atlas.settings.sort_mode = SortMode::Manual;
        seed_manual_positions(project);
        project.atlas_dirty = true;
        project.modified = true;
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
            project.modified = true;
        }
    }

    if let Some((rip_idx, pos)) = new_move {
        if rip_idx < project.rips.len() {
            project.rips[rip_idx].atlas_pos = Some(pos);
            project.atlas_dirty = true;
            project.modified = true;
        }
    }
}

/// Composites the packed rips into a single RGBA image at the natural packed
/// size. Built on demand (export only) — each rip's pixels are resized to its
/// packed cell, so scaled (non-custom) rips land at the right size. Returns
/// `None` when there's no atlas.
fn composite(project: &Project) -> Option<RgbaImage> {
    let res = project.atlas.result.as_ref()?;
    let mut img = RgbaImage::new(res.size[0].max(1) as u32, res.size[1].max(1) as u32);
    for p in &res.placements {
        if let Some(out) = project.rips.get(p.rip).and_then(|r| r.output.as_ref()) {
            let cell = crate::image_edit::resize_to(&out.pixels, [p.w, p.h]);
            image::imageops::overlay(&mut img, &cell, p.x as i64, p.y as i64);
        }
    }
    Some(img)
}

/// Uniformly scales `atlas` to *contain* it inside `[ew, eh]` (preserving aspect,
/// so nothing is stretched), draws it top-left, and returns an `ew×eh` RGBA image
/// with any leftover area left transparent. When the bounds already match the
/// atlas aspect (the Automatic case), this fills exactly with no padding.
fn fit_atlas(atlas: &RgbaImage, [ew, eh]: [u32; 2]) -> RgbaImage {
    let (nw, nh) = (atlas.width() as f32, atlas.height() as f32);
    let fit = (ew as f32 / nw).min(eh as f32 / nh);
    let cw = (nw * fit).round().clamp(1.0, ew as f32) as u32;
    let ch = (nh * fit).round().clamp(1.0, eh as f32) as u32;
    let scaled = crate::image_edit::resize_to(atlas, [cw, ch]);
    let mut canvas = RgbaImage::new(ew.max(1), eh.max(1));
    image::imageops::overlay(&mut canvas, &scaled, 0, 0);
    canvas
}

/// Saves the packed atlas to a PNG at the export resolution. The packed atlas is
/// scaled *uniformly* to fit inside the export bounds (preserving aspect, so rips
/// are never stretched) and placed top-left; any leftover area is transparent.
pub fn export(project: &mut Project) {
    let outcome: Option<Result<String, String>> =
        if let Some([ew, eh]) = export_size(project) {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Export Atlas")
                .add_filter("PNG", &["png"])
                .set_file_name("atlas.png")
                .save_file()
            {
                // Composite the packed rips on demand, then fit into the bounds.
                let img = composite(project).map(|atlas| fit_atlas(&atlas, [ew, eh]));
                Some(match img {
                    Some(img) => img
                        .save(&path)
                        .map_err(|e| e.to_string())
                        .map(|_| path.display().to_string()),
                    None => Err("No atlas to export.".to_string()),
                })
            } else {
                None // cancelled
            }
        } else {
            Some(Err("No atlas to export.".to_string()))
        };

    if let Some(result) = outcome {
        match result {
            Ok(p) => project.set_status(format!("Exported atlas to {p}")),
            Err(e) => project.set_error(format!("Export failed: {e}")),
        }
    }
}
