//! Windows file-type association for `.rtrpf` project files.
//!
//! On Windows we register a per-user (HKCU, no admin needed) association so
//! Explorer shows our icon for `.rtrpf` files and double-clicking one opens the
//! app. The icon is the bundled `logo_g.ico`, written out next to our app data
//! so Explorer can reference it by path. On other platforms `register` is a
//! no-op.

/// The document icon, embedded at build time.
#[cfg(windows)]
const ICON_ICO: &[u8] = include_bytes!("logo_g.ico");

/// Registers the `.rtrpf` association (icon + open command). Best-effort: any
/// failure is logged and ignored so it never blocks app start-up.
#[cfg(windows)]
pub fn register() {
    if let Err(e) = try_register() {
        eprintln!("ricks-textureripper: could not register .rtrpf association: {e}");
    }
}

/// No-op on non-Windows platforms.
#[cfg(not(windows))]
pub fn register() {}

#[cfg(windows)]
const PROG_ID: &str = "RicksTextureRipper.Project";

#[cfg(windows)]
fn try_register() -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    // Generate/refresh the document icon next to our other app data.
    let dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ricks-textureripper");
    std::fs::create_dir_all(&dir)?;
    let ico_path = dir.join("rtrpf.ico");
    write_icon(&ico_path)?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // `.rtrpf` -> our ProgID.
    let (ext, _) = hkcu.create_subkey(r"Software\Classes\.rtrpf")?;
    ext.set_value("", &PROG_ID.to_string())?;

    // ProgID description + icon.
    let (prog, _) = hkcu.create_subkey(format!(r"Software\Classes\{PROG_ID}"))?;
    prog.set_value("", &"Rick's Texture Ripper Project".to_string())?;

    let (icon, _) = hkcu.create_subkey(format!(r"Software\Classes\{PROG_ID}\DefaultIcon"))?;
    icon.set_value("", &ico_path.to_string_lossy().to_string())?;

    // Double-click opens the file with this executable.
    if let Ok(exe) = std::env::current_exe() {
        let (cmd, _) =
            hkcu.create_subkey(format!(r"Software\Classes\{PROG_ID}\shell\open\command"))?;
        cmd.set_value("", &format!("\"{}\" \"%1\"", exe.to_string_lossy()))?;
    }

    notify_shell();
    Ok(())
}

/// Writes the bundled `.ico` to `path` so Explorer can reference it. Rewritten
/// each launch so a new build's icon takes effect.
#[cfg(windows)]
fn write_icon(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, ICON_ICO)
}

/// Tells the shell that file associations changed so icons refresh promptly.
#[cfg(windows)]
fn notify_shell() {
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    const SHCNF_IDLIST: u32 = 0x0000;
    // SAFETY: passing null item pointers is the documented way to signal a
    // global association change.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn SHChangeNotify(
        event_id: i32,
        flags: u32,
        item1: *const std::ffi::c_void,
        item2: *const std::ffi::c_void,
    );
}
