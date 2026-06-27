//! Build script: on Windows, embed the app icon as a PE resource so Explorer
//! shows the icon for the executable file itself (even when it isn't running).
//! The runtime `with_icon` in `main.rs` only covers the live window/taskbar.
//! Also exposes the build date to the binary via the `BUILD_DATE` env var.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Make the build date available as env!("BUILD_DATE") (UTC, YYYY-MM-DD).
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    println!("cargo:rustc-env=BUILD_DATE={y:04}-{m:02}-{d:02}");
    // Time-of-day (UTC) so the About window can show the build hour:minute.
    let sod = secs.rem_euclid(86_400);
    let (hh, mm) = (sod / 3_600, (sod % 3_600) / 60);
    println!("cargo:rustc-env=BUILD_TIME={hh:02}:{mm:02}");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        // The first icon in the group becomes the executable's default icon.
        res.set_icon("src/logo_g.ico");
        // Best-effort: don't fail the whole build if the resource compiler is
        // unavailable on this machine.
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon embed skipped: {e}");
        }
    }
}

/// Converts a day count since 1970-01-01 to a `(year, month, day)` civil date
/// (Howard Hinnant's algorithm); avoids pulling in a date crate.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}
