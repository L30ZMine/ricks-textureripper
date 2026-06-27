//! Boot-time update check + silent self-update against this project's GitHub
//! releases.
//!
//! Runs on a background thread (so it never blocks the UI) and reports its
//! result back over a channel. Uses the system `curl` rather than pulling in an
//! HTTP/TLS crate, matching the project's minimal-dependency style; if `curl`
//! isn't available the check simply reports [`Outcome::Failed`].
//!
//! When a newer release is found and (in a release build) it ships a Windows
//! `.exe` asset, the background thread downloads it to a temp file and the app
//! swaps it in via a small headless PowerShell worker that waits for this
//! process to exit, replaces the running exe, and relaunches — see
//! [`spawn_replace_worker`].
//!
//! NOTE: the GitHub API only returns releases for **public** repositories to an
//! unauthenticated `curl`. If `l30zmine/ricks-textureripper` is private the
//! check (and any download) will 404 → [`Outcome::Failed`]; making the repo
//! public (or attaching a token) is required for the updater to actually run.

use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

/// The GitHub "list releases" endpoint for this repository (owner casing matches
/// the canonical `github.com/L30ZMine/ricks-textureripper`, though the API is
/// case-insensitive). The list (not `/latest`) is used so the newest tag is found
/// even when it's a pre-release or `/latest` hasn't been promoted.
const RELEASES_API: &str =
    "https://api.github.com/repos/L30ZMine/ricks-textureripper/releases?per_page=30";

/// Result of the update check, consumed by the UI.
pub enum Outcome {
    /// The running version is the newest (or there are no releases yet).
    UpToDate,
    /// A newer release exists; carries its tag (e.g. `v1.3.1`) and, when a
    /// Windows `.exe` asset was downloaded ready to apply, its local temp path.
    Available {
        tag: String,
        /// `Some` when the new exe has been fetched and is ready to swap in.
        ready_exe: Option<std::path::PathBuf>,
    },
    /// The check couldn't be completed (offline, `curl` missing, private repo…).
    Failed,
}

/// Spawns the update check (and, for a newer release, the download) on a
/// background thread and returns the receiver to poll for its [`Outcome`].
pub fn spawn_check() -> Receiver<Outcome> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(check());
    });
    rx
}

fn check() -> Outcome {
    let current = env!("CARGO_PKG_VERSION");
    let releases = match fetch_releases() {
        Ok(r) => r,
        Err(()) => return Outcome::Failed,
    };

    // The newest non-draft release by parsed version (pre-releases included).
    let newest = releases
        .into_iter()
        .filter(|r| !r.draft)
        .filter_map(|r| r.tag_name.clone().and_then(|t| parse_version(&t).map(|v| (v, r))))
        .max_by_key(|(v, _)| *v);

    let Some((remote_v, release)) = newest else {
        return Outcome::UpToDate; // no usable release published yet
    };
    let Some(local_v) = parse_version(current) else {
        return Outcome::UpToDate;
    };
    if remote_v <= local_v {
        return Outcome::UpToDate;
    }

    let tag = release.tag_name.unwrap_or_default();

    // Only auto-download in a release build (a debug build's exe isn't a
    // distributable, and we never want a dev session to self-replace).
    let ready_exe = if cfg!(debug_assertions) {
        None
    } else {
        exe_asset_url(&release.assets).and_then(|url| download_exe(&url).ok())
    };

    Outcome::Available { tag, ready_exe }
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// GETs the releases list via `curl` and parses it. `Err(())` means the call
/// couldn't run or the repo wasn't reachable (offline / private / 404).
fn fetch_releases() -> Result<Vec<Release>, ()> {
    let out = curl(&[
        "-sL",
        "--max-time",
        "8",
        "-H",
        "User-Agent: ricks-textureripper",
        "-H",
        "Accept: application/vnd.github+json",
        RELEASES_API,
    ])?;
    if !out.status.success() {
        return Err(());
    }
    // A 404 body ("Not Found") isn't a JSON array, so this fails → Err, which the
    // caller surfaces as a failed check.
    serde_json::from_slice(&out.stdout).map_err(|_| ())
}

/// Picks the best Windows `.exe` download URL from a release's assets: prefer one
/// whose name mentions the app, else the first `.exe`.
fn exe_asset_url(assets: &[Asset]) -> Option<String> {
    let is_exe = |a: &&Asset| a.name.to_ascii_lowercase().ends_with(".exe");
    assets
        .iter()
        .find(|a| is_exe(a) && a.name.to_ascii_lowercase().contains("ricks"))
        .or_else(|| assets.iter().find(is_exe))
        .map(|a| a.browser_download_url.clone())
}

/// Downloads `url` to a temp file and returns its path. Verifies the payload is a
/// real Windows executable (PE `MZ` header) so a 404 HTML/JSON body is rejected.
fn download_exe(url: &str) -> Result<std::path::PathBuf, ()> {
    let mut dest = std::env::temp_dir();
    dest.push("ricks-textureripper-update.exe");
    let dest_str = dest.to_string_lossy().into_owned();
    let out = curl(&[
        "-sL",
        "--max-time",
        "120",
        "-H",
        "User-Agent: ricks-textureripper",
        "-o",
        &dest_str,
        url,
    ])?;
    if !out.status.success() {
        return Err(());
    }
    // Reject anything that isn't a PE executable (e.g. a 404 error page).
    let head = std::fs::read(&dest).map_err(|_| ())?;
    if head.len() < 2 || &head[..2] != b"MZ" {
        let _ = std::fs::remove_file(&dest);
        return Err(());
    }
    Ok(dest)
}

/// Runs `curl` with `args`, headless on Windows so it never flashes a console.
fn curl(args: &[&str]) -> Result<std::process::Output, ()> {
    let mut cmd = Command::new("curl");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output().map_err(|_| ())
}

/// Spawns a headless worker that waits for this process to exit, replaces the
/// running exe with `new_exe`, relaunches it, and cleans up. The app should call
/// this and then immediately close itself. Best-effort; Windows-only (a no-op
/// elsewhere). The worker is **not** elevated, so it silently succeeds for a
/// user-writable install (portable / per-user) and simply fails for a
/// Program Files install that needs admin (the version stays put, no prompt).
#[cfg(windows)]
pub fn spawn_replace_worker(new_exe: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let target = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = target.display().to_string();
    let new_exe = new_exe.display().to_string();
    let pid = std::process::id();

    // Wait for us to exit (so the exe unlocks), back up the old exe, swap in the
    // new one, relaunch from the same path, then delete the backup + temp copy.
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $new = '{new_exe}'\n\
         $target = '{target}'\n\
         Wait-Process -Id {pid} -Timeout 30\n\
         $bak = \"$target.old\"\n\
         Remove-Item -LiteralPath $bak -Force\n\
         Move-Item -LiteralPath $target -Destination $bak -Force\n\
         Move-Item -LiteralPath $new -Destination $target -Force\n\
         Start-Process -FilePath $target -WorkingDirectory (Split-Path $target)\n\
         Remove-Item -LiteralPath $bak -Force\n",
    );

    let mut path = std::env::temp_dir();
    path.push("ricks-textureripper-update.ps1");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(script.as_bytes()))
        .map_err(|e| e.to_string())?;

    Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &path.display().to_string(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Non-Windows stub (self-replace is wired up for Windows only).
#[cfg(not(windows))]
pub fn spawn_replace_worker(_new_exe: &std::path::Path) -> Result<(), String> {
    Err("Self-update is only supported on Windows.".to_string())
}

/// Parses a `major.minor.patch` version, tolerating a leading `v` and trailing
/// pre-release suffixes (e.g. `v1.2.0-beta` → `(1, 2, 0)`).
/// Orders two version strings (e.g. the running build vs. an installed copy).
/// `None` when either string can't be parsed as a version.
pub(crate) fn version_cmp(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    Some(parse_version(a)?.cmp(&parse_version(b)?))
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim();
    let v = v.strip_prefix('v').or_else(|| v.strip_prefix('V')).unwrap_or(v);
    let mut parts = v.split('.');
    let lead_digits = |s: &str| -> u32 {
        s.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    // The major component must contain at least one digit to be a valid version.
    let major_str = parts.next()?;
    if !major_str.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let major = lead_digits(major_str);
    let minor = parts.next().map(lead_digits).unwrap_or(0);
    let patch = parts.next().map(lead_digits).unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True when `remote` is a strictly higher version than `local`.
    fn is_newer(remote: &str, local: &str) -> bool {
        match (parse_version(remote), parse_version(local)) {
            (Some(r), Some(l)) => r > l,
            _ => false,
        }
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("1.3.1", "1.3.0"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(is_newer("1.2.1", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.9", "1.2.0"));
        assert!(!is_newer("v1.2.0", "1.2.0"));
        assert_eq!(parse_version("v1.2.0-beta"), Some((1, 2, 0)));
    }
}
