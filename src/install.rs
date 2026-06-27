//! Optional install of the running executable into `C:\Program Files` plus an
//! all-users Start Menu shortcut (offered in the Setup dialog).
//!
//! Writing to Program Files and the all-users Start Menu needs administrator
//! rights, so this builds a small PowerShell script and relaunches it **elevated**
//! via `Start-Process -Verb RunAs`. That is the standard UAC mechanism: a
//! medium-integrity (normal) process gets a UAC consent prompt; an already-
//! elevated process (e.g. launched from an admin terminal / debugger) runs the
//! child elevated with no prompt. Both the launcher and the elevated worker run
//! **headless** (no console window). After copying, the script waits for this
//! process to exit, deletes the old exe, and relaunches from the new path.
//! Best-effort (it only reports whether the elevated process launched);
//! Windows-only (`#[cfg]` stub elsewhere).

/// The all-users install location.
#[cfg(windows)]
const INSTALL_DIR: &str = r"C:\Program Files\ricks-textureripper";

/// Installs to `C:\Program Files\ricks-textureripper`, adds a Start Menu
/// shortcut, then (after this process exits) removes the previous exe and
/// relaunches from the new path. Windows-only.
#[cfg(windows)]
pub fn install_to_program_files() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.display().to_string();
    let pid = std::process::id();
    let dest_dir = INSTALL_DIR;
    let dest_exe = format!(r"{dest_dir}\ricks-textureripper.exe");
    // Stamp the install with our version so a later run can detect it and offer to
    // update an older install (read back by `installed_version`).
    let ver = env!("CARGO_PKG_VERSION");

    // Elevated worker script. Order matters: copy + shortcut first (the source
    // exe is still running, which is fine to read), then wait for this process to
    // exit so the old exe is unlocked, then delete it, then relaunch.
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $src = '{exe}'\n\
         $destDir = '{dest_dir}'\n\
         $destExe = '{dest_exe}'\n\
         New-Item -ItemType Directory -Force -Path $destDir | Out-Null\n\
         Copy-Item -LiteralPath $src -Destination $destExe -Force\n\
         Set-Content -LiteralPath \"$destDir\\version.txt\" -Value '{ver}' -NoNewline\n\
         $lnk = \"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Rick's Texture Ripper.lnk\"\n\
         $ws = New-Object -ComObject WScript.Shell\n\
         $s = $ws.CreateShortcut($lnk)\n\
         $s.TargetPath = $destExe\n\
         $s.WorkingDirectory = $destDir\n\
         $s.Save()\n\
         $uk = 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ricks-textureripper'\n\
         New-Item -Path $uk -Force | Out-Null\n\
         Set-ItemProperty -Path $uk -Name DisplayName -Value \"Rick's Texture Ripper\"\n\
         Set-ItemProperty -Path $uk -Name DisplayVersion -Value '{ver}'\n\
         Set-ItemProperty -Path $uk -Name Publisher -Value 'l30z'\n\
         Set-ItemProperty -Path $uk -Name DisplayIcon -Value $destExe\n\
         Set-ItemProperty -Path $uk -Name InstallLocation -Value $destDir\n\
         Set-ItemProperty -Path $uk -Name UninstallString -Value ('\"' + $destExe + '\" --uninstall')\n\
         Set-ItemProperty -Path $uk -Name QuietUninstallString -Value ('\"' + $destExe + '\" --uninstall --quiet')\n\
         Set-ItemProperty -Path $uk -Name NoModify -Value 1 -Type DWord\n\
         Set-ItemProperty -Path $uk -Name NoRepair -Value 1 -Type DWord\n\
         Wait-Process -Id {pid} -Timeout 30\n\
         if ($src -ne $destExe) {{ Remove-Item -LiteralPath $src -Force }}\n\
         Start-Process -FilePath $destExe -WorkingDirectory $destDir\n",
    );

    run_elevated_script("ricks-textureripper-install.ps1", &script)
}

/// Writes `script` to a temp `.ps1` and relaunches it via an **elevated, headless**
/// PowerShell (`Start-Process -Verb RunAs` — the standard UAC path). Both the
/// launcher and the elevated worker run with no console window. Shared by install
/// and uninstall. Windows-only.
#[cfg(windows)]
fn run_elevated_script(file_name: &str, script: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    /// Don't pop a console window for the launcher PowerShell.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut path = std::env::temp_dir();
    path.push(file_name);
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(script.as_bytes()))
        .map_err(|e| e.to_string())?;

    let launch = format!(
        "Start-Process -FilePath powershell -Verb RunAs -WindowStyle Hidden -ArgumentList \
         '-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File','{}'",
        path.display()
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &launch])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs the uninstall flow, invoked via the Apps & features entry's
/// `UninstallString` (`"<exe>" --uninstall`). Unless `quiet`, asks whether to also
/// delete user data; then removes the per-user file association (and optionally
/// data) **in this non-elevated process** — HKCU/Documents are per-user — and
/// spawns an elevated worker to remove the Program Files install, the Start Menu
/// shortcut, and the Apps & features key. Returns once the worker is launched so
/// the caller can exit (the worker waits for this PID before deleting the exe).
/// Windows-only.
#[cfg(windows)]
pub fn run_uninstall(quiet: bool) {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

    // Keeping files is the safe default — it's mapped to "Yes" (the dialog's
    // default/Enter button) so an accidental confirm never deletes user data.
    // Yes = uninstall + keep, No = uninstall + delete, Cancel = abort.
    let remove_data = if quiet {
        false // QuietUninstallString keeps user data (matches the interactive default).
    } else {
        match MessageDialog::new()
            .set_title("Uninstall Rick's Texture Ripper")
            .set_description(
                "Uninstall Rick's Texture Ripper?\n\n\
                 Any open windows will be closed first (you'll get a chance to save).\n\
                 Your settings & layouts are kept by default.\n\n\
                 Yes  -  uninstall, keep my settings & layouts\n\
                 No  -  uninstall AND delete my settings & layouts\n\
                 Cancel  -  don't uninstall",
            )
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNoCancel)
            .show()
        {
            MessageDialogResult::Yes => false, // keep data (safe default button)
            MessageDialogResult::No => true,   // explicitly delete data
            _ => return,                       // Cancel / dialog closed → abort
        }
    };

    // Per-user cleanup, as the invoking user (must not run elevated — that hive
    // belongs to a different user).
    crate::file_assoc::unregister();
    if remove_data {
        crate::layouts::purge_user_data();
    }

    // Elevated removal of the admin-owned bits (program dir, all-users shortcut,
    // HKLM Apps & features key). The app then exits so the worker can delete the
    // (possibly still-running) exe.
    //
    // Crucially, before deleting the program folder the worker makes sure **no**
    // instance still has the exe locked: it waits for this `--uninstall` process,
    // then asks any *other* open window to close (`CloseMainWindow` triggers the
    // app's own unsaved-changes prompt so work can be saved), waits a grace period,
    // and force-stops any straggler as a last resort. Without this, a normal app
    // window left open would lock the exe and the delete would fail.
    let dest_dir = INSTALL_DIR;
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Wait-Process -Id {pid} -Timeout 30\n\
         $others = Get-Process -Name ricks-textureripper -ErrorAction SilentlyContinue\n\
         if ($others) {{ $others.CloseMainWindow() | Out-Null; $others | Wait-Process -Timeout 30 -ErrorAction SilentlyContinue }}\n\
         Get-Process -Name ricks-textureripper -ErrorAction SilentlyContinue | Stop-Process -Force\n\
         Start-Sleep -Milliseconds 500\n\
         Remove-Item -LiteralPath \"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Rick's Texture Ripper.lnk\" -Force\n\
         Remove-Item -Path 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ricks-textureripper' -Recurse -Force\n\
         Remove-Item -LiteralPath '{dest_dir}' -Recurse -Force\n",
        pid = std::process::id(),
    );
    if let Err(e) = run_elevated_script("ricks-textureripper-uninstall.ps1", &script) {
        eprintln!("ricks-textureripper: uninstall worker failed to launch: {e}");
    }
}

/// True when the running executable already lives in the Program Files install
/// location, so the Setup dialog can grey out the "install" option. Windows-only.
#[cfg(windows)]
pub fn is_installed() -> bool {
    let dest_dir = INSTALL_DIR.to_lowercase();
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.to_string_lossy().to_lowercase()))
        .is_some_and(|dir| dir == dest_dir)
}

/// True when an install exists in Program Files (whether or not *this* exe is it),
/// so the Setup dialog can offer to update it. Windows-only.
#[cfg(windows)]
pub fn installed_exists() -> bool {
    std::path::Path::new(INSTALL_DIR)
        .join("ricks-textureripper.exe")
        .exists()
}

/// The version of the existing Program Files install, read from the `version.txt`
/// the installer stamps alongside the exe. `None` when there's no install, or it
/// predates version stamping (treated as an unknown, updatable version).
#[cfg(windows)]
pub fn installed_version() -> Option<String> {
    let v = std::fs::read_to_string(std::path::Path::new(INSTALL_DIR).join("version.txt")).ok()?;
    let v = v.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Non-Windows stub (the Setup dialog hides the option off Windows).
#[cfg(not(windows))]
pub fn install_to_program_files() -> Result<(), String> {
    Err("Installation is only supported on Windows.".to_string())
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub fn is_installed() -> bool {
    false
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub fn installed_exists() -> bool {
    false
}

/// Non-Windows stub.
#[cfg(not(windows))]
pub fn installed_version() -> Option<String> {
    None
}

/// Non-Windows stub (the Apps & features uninstall path is Windows-only).
#[cfg(not(windows))]
pub fn run_uninstall(_quiet: bool) {}
