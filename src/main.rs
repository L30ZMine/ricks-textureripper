#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod atlas;
mod b64;
mod file_assoc;
mod history;
mod image_edit;
mod layouts;
mod proj_io;
mod project;
mod rip_tool;
mod snapshot;
mod texture_view;
mod ui;
mod warp;

fn main() -> eframe::Result<()> {
    // Associate `.rtrpf` files with this app so Explorer shows our icon for them.
    file_assoc::register();

    // A project file passed on the command line (e.g. by double-clicking a
    // `.rtrpf` in Explorer, which invokes `"<exe>" "%1"`). Opened on first frame.
    let startup_open = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(proj_io::EXTENSION))
        });

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0])
        .with_title("Rick's Texture Ripper 1.2.0");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Rick's Texture Ripper 1.2.0",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(startup_open)))),
    )
}

/// Decodes the embedded `logo_w.ico` into an eframe window/taskbar icon.
fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("logo_w.ico"))
        .ok()?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}
