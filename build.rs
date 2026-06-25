//! Build script: on Windows, embed the app icon as a PE resource so Explorer
//! shows the icon for the executable file itself (even when it isn't running).
//! The runtime `with_icon` in `main.rs` only covers the live window/taskbar.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // The first icon in the group becomes the executable's default icon.
        res.set_icon("src/logo_w.ico");
        // Best-effort: don't fail the whole build if the resource compiler is
        // unavailable on this machine.
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon embed skipped: {e}");
        }
    }
}
