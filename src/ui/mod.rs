//! UI building blocks: the dockable, Blender-style panel system.

pub mod docking;

/// Styles the scroll bars created *after* this call (on `ui` and its children) for
/// the panel toolbars and the Image Edit list:
/// - **half the default thickness** when hovered/grabbed (`bar_width` 10 → 5), and
/// - reserves a small **lane** (`floating_allocated_width`) so the bar sits beside
///   the content with a little padding instead of floating over the text.
///
/// Because egui only reserves that lane *while the bar is actually shown* (i.e. the
/// content overflows — see `current_bar_use` in egui's `scroll_area`), the padding
/// appears exactly when the scrollbar is needed and is gone otherwise, so a panel's
/// size doesn't change (no canvas jump) when no bar is required.
pub fn thin_scrollbar(ui: &mut egui::Ui) {
    let mut scroll = egui::style::ScrollStyle::floating();
    scroll.bar_width = 5.0;
    scroll.floating_allocated_width = 8.0;
    ui.style_mut().spacing.scroll = scroll;
}
