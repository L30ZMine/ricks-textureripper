#[cfg(windows)]
pub fn install_to_program_files() -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.display().to_string();
    let pid = std::process::id();
    let dest_dir = r"C:\Program Files\ricks-textureripper";
    let dest_exe = format!(r"{dest_dir}\ricks-textureripper.exe");

    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $src = '{exe}'\n\
         $destDir = '{dest_dir}'\n\
         $destExe = '{dest_exe}'\n\
         New-Item -ItemType Directory -Force -Path $destDir | Out-Null\n\
         Copy-Item -LiteralPath $src -Destination $destExe -Force\n\
         $lnk = \"$env:ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Rick's Texture Ripper.lnk\"\n\
         $ws = New-Object -ComObject WScript.Shell\n\
         $s = $ws.CreateShortcut($lnk)\n\
         $s.TargetPath = $destExe\n\
         $s.WorkingDirectory = $destDir\n\
         $s.Save()\n\
         Wait-Process -Id {pid} -Timeout 30\n\
         if ($src -ne $destExe) {{ Remove-Item -LiteralPath $src -Force }}\n\
         Start-Process -FilePath $destExe -WorkingDirectory $destDir\n",
    );

    let mut path = std::env::temp_dir();
    path.push("ricks-textureripper-install.ps1");
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

#[cfg(not(windows))]
pub fn install_to_program_files() -> Result<(), String> {
    Err("Installation is only supported on Windows.".to_string())
}
