use std::process::Command;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

const RELEASES_API: &str =
    "https://api.github.com/repos/l30zmine/ricks-textureripper/releases/latest";

pub enum Outcome {

    UpToDate,

    Available(String),

    Failed,
}

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
        Ok(_) => Outcome::UpToDate,
        Err(()) => Outcome::Failed,
    }
}

fn fetch_latest_tag() -> Result<Option<String>, ()> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sL",
        "--max-time",
        "8",
        "-H",
        "User-Agent: ricks-textureripper",
        "-H",
        "Accept: application/vnd.github+json",
        RELEASES_API,
    ]);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|_| ())?;
    if !out.status.success() {
        return Err(());
    }

    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: Option<String>,
    }

    let release: Release = serde_json::from_slice(&out.stdout).map_err(|_| ())?;
    Ok(release.tag_name)
}

fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
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
