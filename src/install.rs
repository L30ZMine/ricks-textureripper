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
//! process to exit, optionally deletes the old exe, and relaunches from the new
//! path. Best-effort (it only reports whether the elevated process launched);
//! Windows-only (`#[cfg]` stub elsewhere).

/// Installs to `C:\Program Files\ricks-textureripper`, adds a Start Menu
/// shortcut, then (after this process exits) relaunches from there and, when
/// `delete_old` is set, removes the previous exe. Windows-only.
#[cfg(windows)]
pub fn install_to_program_files(delete_old: bool) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    /// Don't pop a console window for the launcher PowerShell.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.display().to_string();
    let pid = std::process::id();
    let dest_dir = r"C:\Program Files\ricks-textureripper";
    let dest_exe = format!(r"{dest_dir}\ricks-textureripper.exe");
    let delete = if delete_old { "$true" } else { "$false" };

    // Elevated worker script. Order matters: copy + shortcut first (the source
    // exe is still running, which is fine to read), then wait for this process to
    // exit so the old exe is unlocked, then optionally delete it, then relaunch.
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $src = '{exe}'\n\
         $destDir = '{dest_dir}'\n\
         $destExe = '{dest_exe}'\n\
         $deleteOld = {delete}\n\
         New-Item -ItemType Directory -Force -Path $destDir | Out-Null\n\
         Copy-Item -LiteralPath $src -Destination $destExe -Force\n\
         $lnk = \"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Rick's Texture Ripper.lnk\"\n\
         $ws = New-Object -ComObject WScript.Shell\n\
         $s = $ws.CreateShortcut($lnk)\n\
         $s.TargetPath = $destExe\n\
         $s.WorkingDirectory = $destDir\n\
         $s.Save()\n\
         Wait-Process -Id {pid} -Timeout 30\n\
         if ($deleteOld -and ($src -ne $destExe)) {{ Remove-Item -LiteralPath $src -Force }}\n\
         Start-Process -FilePath $destExe -WorkingDirectory $destDir\n",
    );

    // Write the script to a temp file so the elevated launch only needs a path.
    let mut path = std::env::temp_dir();
    path.push("ricks-textureripper-install.ps1");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(script.as_bytes()))
        .map_err(|e| e.to_string())?;

    // Relaunch PowerShell elevated to run the script — headless on both sides.
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

/// Non-Windows stub (the Setup dialog hides the option off Windows).
#[cfg(not(windows))]
pub fn install_to_program_files(_delete_old: bool) -> Result<(), String> {
    Err("Installation is only supported on Windows.".to_string())
}
