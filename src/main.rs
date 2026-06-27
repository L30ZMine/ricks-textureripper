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
mod render;
mod rip_tool;
mod snapshot;
mod texture_view;
mod ui;
mod update;
mod warp;

fn main() -> eframe::Result<()> {
    // `--uninstall` (from the Apps & features entry's UninstallString) runs the
    // removal flow instead of launching the app, then exits.
    if handle_uninstall_arg() {
        return Ok(());
    }

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

    // Resolve where preferences live: first run (setup dialog), a single resolved
    // location, or a conflict (config in both Documents and a portable folder →
    // the app asks the user which to use).
    let storage = layouts::resolve();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 500.0])
        // Accept files dropped onto the window (OLE drag-and-drop on Windows).
        .with_drag_and_drop(true)
        .with_title("Rick's Texture Ripper 1.3.4");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Rick's Texture Ripper 1.3.4",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app::App::new(startup_open, storage)))),
    )
}

/// Handles the `--uninstall` command line (the Apps & features `UninstallString`):
/// runs the removal flow and returns `true` so `main` exits without starting the
/// app. `--quiet` keeps user data without prompting. No-op (returns `false`) when
/// the flag isn't present or off Windows.
fn handle_uninstall_arg() -> bool {
    if std::env::args().any(|a| a == "--uninstall") {
        install::run_uninstall(std::env::args().any(|a| a == "--quiet"));
        return true;
    }
    false
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
