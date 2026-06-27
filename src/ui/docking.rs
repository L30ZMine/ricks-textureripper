//! The dockable panel system, built on `egui_dock`.
//!
//! The four panels (Atlas View, Texture View, Image Edit, Rips Gallery) are the
//! "tabs" of the dock. They can be dragged, snapped, split and re-docked
//! Blender-style, but are *not* allowed to detach into floating windows. Their
//! *content* is drawn against whichever `Project` is currently active, which is
//! how switching the top-level project tabs swaps the whole workspace.

use crate::project::Project;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use egui_dock::TabViewer;

/// Fixed cell width (px) of one rip gallery item's *content* (the group's inner
/// width), used to size the responsive column count and keep a uniform footprint.
const ITEM_WIDTH: f32 = 110.0;
/// Extra width an item's `ui.group` frame adds around `ITEM_WIDTH`: inner margin
/// (2×6) + stroke (2×1). Included in the column-count maths so the last column
/// isn't cut off (the previous calc ignored it, so columns dropped ~14px late).
const ITEM_FRAME: f32 = 14.0;
/// Fixed thumbnail box size (px); images are fit inside, centered.
const THUMB: f32 = 92.0;

/// A responsive, uniform-height gallery of extracted rips with per-item remove
/// buttons. Column count shrinks with the available width (down to one per row),
/// and every item has the same height regardless of its thumbnail's aspect.
fn rip_gallery(ui: &mut egui::Ui, project: &mut Project) {
    if project.rips.is_empty() {
        ui.weak("No rips yet. Use \"Add Rip\" in the Texture View.");
        return;
    }

    let selected = project.editor.selected;
    let mut select: Option<usize> = None;
    let mut remove: Option<usize> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            // Compute the column count *inside* the scroll area so the (possible)
            // scrollbar is already subtracted from the width. Each item occupies
            // `ITEM_WIDTH + ITEM_FRAME`, with `item_spacing.x` between items:
            // fit the most columns where `n*item + (n-1)*spacing <= avail`.
            let avail = ui.available_width().max(ITEM_WIDTH);
            let spacing = ui.spacing().item_spacing.x;
            let item = ITEM_WIDTH + ITEM_FRAME;
            let cols = (((avail + spacing) / (item + spacing)).floor() as usize).max(1);
            let n = project.rips.len();
            let mut i = 0;
            while i < n {
                ui.horizontal(|ui| {
                    for _ in 0..cols {
                        if i >= n {
                            break;
                        }
                        rip_item(
                            ui,
                            &project.rips[i],
                            i,
                            selected == Some(i),
                            &mut select,
                            &mut remove,
                        );
                        i += 1;
                    }
                });
            }
        });

    if let Some(i) = select {
        project.editor.selected = Some(i);
    }
    if let Some(i) = remove {
        project.rips.remove(i);
        project.editor.selected = None;
        project.atlas_dirty = true;
        project.modified = true;
    }
}

/// Draws one fixed-size gallery item: a thumbnail box, the rip name, and a
/// remove button. Clicking the thumbnail or name selects the rip.
fn rip_item(
    ui: &mut egui::Ui,
    rip: &crate::project::Rip,
    idx: usize,
    is_sel: bool,
    select: &mut Option<usize>,
    remove: &mut Option<usize>,
) {
    ui.group(|ui| {
        ui.set_width(ITEM_WIDTH);
        ui.vertical_centered(|ui| {
            // Fixed thumbnail box keeps every item the same height.
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::click());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
            match &rip.output {
                Some(out) => {
                    let (w, h) = (out.size[0] as f32, out.size[1] as f32);
                    let s = (THUMB / w.max(1.0)).min(THUMB / h.max(1.0)).min(1.0);
                    let isz = Vec2::new(w * s, h * s);
                    let img_rect = Rect::from_center_size(rect.center(), isz);
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    painter.image(out.texture.id(), img_rect, uv, Color32::WHITE);
                }
                None => {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "(empty)",
                        FontId::proportional(12.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            if resp.clicked() {
                *select = Some(idx);
            }

            let label = if is_sel {
                RichText::new(&rip.name).strong()
            } else {
                RichText::new(&rip.name)
            };
            if ui
                .add_sized(
                    [ITEM_WIDTH - 12.0, 18.0],
                    egui::SelectableLabel::new(is_sel, label),
                )
                .clicked()
            {
                *select = Some(idx);
            }
            if ui.small_button("remove").clicked() {
                *remove = Some(idx);
            }
        });
    });
}

/// One dockable panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PanelTab {
    Atlas,
    Texture,
    ImageEdit,
    Rips,
}

/// Renders dock panels against the active project for one frame.
pub struct DockViewer<'a> {
    pub project: &'a mut Project,
}

impl<'a> TabViewer for DockViewer<'a> {
    type Tab = PanelTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            PanelTab::Atlas => "Atlas View",
            PanelTab::Texture => "Texture View",
            PanelTab::ImageEdit => "Image Edit",
            PanelTab::Rips => "Rips Gallery",
        }
        .into()
    }

    /// Panels can be closed from their tab's X button; the default `on_close`
    /// (which returns true) removes the tab. This integrates with the Window menu,
    /// which tracks open panels via `find_tab` — re-checking a panel there brings
    /// it back. (Previously this was `false`, which still drew the X but disabled
    /// it, showing a "not allowed" cursor on hover.)
    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        true
    }

    /// Never let a tab be dragged out into a floating window — detaching is not
    /// supported here (it previously crashed); panels stay docked.
    fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool {
        false
    }

    /// Don't wrap a panel body in egui_dock's auto scroll area. When a panel's
    /// content (e.g. a zoomed-in Atlas preview) is bigger than the panel, it's
    /// simply **clipped** — the user resizes the panel or zooms out rather than
    /// getting a janky window-culling scrollbar. Panels that genuinely need to
    /// scroll (the Rips Gallery) use their *own* `ScrollArea` inside `ui`, which
    /// this doesn't affect.
    fn scroll_bars(&self, _tab: &Self::Tab) -> [bool; 2] {
        [false, false]
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            PanelTab::Atlas => {
                crate::atlas::ui(ui, self.project);
            }
            PanelTab::Texture => {
                crate::texture_view::ui(ui, self.project);
            }
            PanelTab::ImageEdit => {
                crate::image_edit::ui(ui, self.project);
            }
            PanelTab::Rips => {
                // No title / separator — the dock tab already names the panel.
                rip_gallery(ui, self.project);
            }
        }
    }
}
