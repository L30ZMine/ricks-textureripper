//! Boot-time update check against the GitHub releases of this project.
//!
//! Runs on a background thread (so it never blocks the UI) and reports its
//! result back over a channel. Uses the system `curl` rather than pulling in an
//! HTTP/TLS crate, matching the project's minimal-dependency style; if `curl`
//! isn't available the check simply reports [`Outcome::Failed`].

use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

/// The GitHub "latest release" endpoint for this repository.
const RELEASES_API: &str =
    "https://api.github.com/repos/l30zmine/ricks-textureripper/releases/latest";

/// Result of the update check, consumed by the UI.
pub enum Outcome {
    /// The running version is the newest (or there are no releases yet).
    UpToDate,
    /// A newer release exists; carries its tag (e.g. `v1.3.0`).
    Available(String),
    /// The check couldn't be completed (offline, `curl` missing, etc.).
    Failed,
}

/// Spawns the update check on a background thread and returns the receiver to
/// poll for its [`Outcome`].
pub fn spawn_check() -> Receiver<Outcome> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(check());
    });
    rx
}

fn check() -> Outcome {
    let current = env!("CARGO_PKG_VERSION");
    match fetch_latest_tag() {
        Ok(Some(tag)) if is_newer(&tag, current) => Outcome::Available(tag),
        Ok(_) => Outcome::UpToDate, // newest, or no release published yet
        Err(()) => Outcome::Failed,
    }
}

/// Fetches the latest release `tag_name` via `curl`. `Ok(None)` means the call
/// succeeded but no release was found; `Err(())` means it couldn't run.
fn fetch_latest_tag() -> Result<Option<String>, ()> {
    let out = Command::new("curl")
        .args([
            "-sL",
            "--max-time",
            "8",
            "-H",
            "User-Agent: ricks-textureripper",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASES_API,
        ])
        .output()
        .map_err(|_| ())?;
    if !out.status.success() {
        return Err(());
    }

    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: Option<String>,
    }
    // A 404 ("no releases") still parses — `tag_name` is just absent → None.
    let release: Release = serde_json::from_slice(&out.stdout).map_err(|_| ())?;
    Ok(release.tag_name)
}

/// True when `remote` is a strictly higher version than `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

/// Parses a `major.minor.patch` version, tolerating a leading `v` and trailing
/// pre-release suffixes (e.g. `v1.2.0-beta` → `(1, 2, 0)`).
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

    #[test]
    fn version_compare() {
        assert!(is_newer("1.3.0", "1.2.0"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(is_newer("1.2.1", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.9", "1.2.0"));
        assert!(!is_newer("v1.2.0", "1.2.0"));
        assert_eq!(parse_version("v1.2.0-beta"), Some((1, 2, 0)));
    }
}
