//! The dockable panel system, built on `egui_dock`.
//!
//! The three panels (Atlas View, Texture View, Image Edit) are the "tabs" of the
//! dock. They can be dragged, snapped, split and re-docked Blender-style. Their
//! *content* is drawn against whichever `Project` is currently active, which is
//! how switching the top-level project tabs swaps the whole workspace.

use crate::project::Project;
use egui::{load::SizedTexture, Vec2};
use egui_dock::TabViewer;

/// A simple wrapped gallery of extracted rips with per-item remove buttons.
/// Replaced/augmented by the packed atlas view in Phase 4.
fn rip_gallery(ui: &mut egui::Ui, project: &mut Project) {
    if project.rips.is_empty() {
        ui.weak("No rips yet. Use \"Add Rip\" in the Texture View.");
        return;
    }

    let mut remove: Option<usize> = None;
    let selected = project.editor.selected;
    let mut select: Option<usize> = None;
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, rip) in project.rips.iter().enumerate() {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        match &rip.output {
                            Some(out) => {
                                let (w, h) = (out.size[0] as f32, out.size[1] as f32);
                                let scale = (96.0 / w).min(96.0 / h).min(1.0);
                                let size = Vec2::new(w * scale, h * scale);
                                // Clicking the thumbnail selects the rip too.
                                let img = egui::Image::new(SizedTexture::new(out.texture.id(), size))
                                    .sense(egui::Sense::click());
                                if ui.add(img).clicked() {
                                    select = Some(i);
                                }
                            }
                            None => {
                                ui.add_sized([96.0, 96.0], egui::Label::new("(empty)"));
                            }
                        }
                        let label = if selected == Some(i) {
                            egui::RichText::new(&rip.name).strong()
                        } else {
                            egui::RichText::new(&rip.name)
                        };
                        if ui.selectable_label(selected == Some(i), label).clicked() {
                            select = Some(i);
                        }
                        if ui.small_button("remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
    });

    if let Some(i) = select {
        project.editor.selected = Some(i);
    }
    if let Some(i) = remove {
        project.rips.remove(i);
        project.editor.selected = None;
        project.atlas_dirty = true;
    }
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

    /// Panels are part of the fixed workspace layout; don't let the user close
    /// them (there's no UI yet to bring them back).
    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        false
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
                ui.heading("Rips Gallery");
                ui.separator();
                rip_gallery(ui, self.project);
            }
        }
    }
}
