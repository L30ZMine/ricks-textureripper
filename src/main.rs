#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod atlas;
mod autosave;
mod b64;
mod file_assoc;
mod history;
mod image_edit;
mod install;
mod layouts;
mod proj_io;
mod project;
mod rip_tool;
mod snapshot;
mod texture_view;
mod ui;
mod update;
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

    // Resolve where preferences live and whether this is the first run (no config
    // in either candidate location yet → the app shows its setup dialog).
    let first_run = resolve_storage();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0])
        // Accept files dropped onto the window (OLE drag-and-drop on Windows).
        .with_drag_and_drop(true)
        .with_title("Rick's Texture Ripper 1.3.0");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Rick's Texture Ripper 1.3.0",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(startup_open, first_run)))),
    )
}

/// Decides where config/layouts are read from and whether this is a first run.
/// A portable install (config next to the exe) wins; otherwise Documents is used.
/// Returns `true` when no config exists in either place yet.
fn resolve_storage() -> bool {
    let portable = layouts::portable_dir();
    if portable
        .as_ref()
        .is_some_and(|d| d.join("config.json").exists())
    {
        if let Some(p) = portable {
            layouts::set_app_dir(p);
        }
        return false;
    }
    // Default to Documents (no override needed); first run if it has no config.
    !layouts::documents_dir()
        .is_some_and(|d| d.join("config.json").exists())
}

/// Decodes the embedded `logo_g.ico` into the eframe window/taskbar icon.
fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("logo_g.ico"))
        .ok()?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}
