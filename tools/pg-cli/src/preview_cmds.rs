//! `pg preview`: a ready-to-run build of the game in the project folder, named for the commit it came from.
//!
//! Builds `pg-app` in release mode and copies it to `preview/Playground-preview_<buildstring>.exe`. The build
//! string is the short commit hash; with uncommitted changes it is `<hash>-dirty-<MMDD-HHMM>` (UTC) so two
//! builds of different work never share a name. The program finds `data/base` by walking up from where it
//! sits, so it runs from `preview/` as it is. Only the newest few previews are kept.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many previews to keep in the folder.
const KEEP: usize = 8;
const FOLDER: &str = "preview";
const PREFIX: &str = "Playground-preview_";

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// Month, day, hour and minute (UTC) of a Unix time, as `MMDD-HHMM`.
fn stamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{month:02}{day:02}-{:02}{:02}", rem / 3600, rem % 3600 / 60)
}

/// The build string for the current state of the repository.
pub fn build_string() -> String {
    let hash = git(&["rev-parse", "--short", "HEAD"]);
    let hash = if hash.is_empty() {
        "nogit".to_owned()
    } else {
        hash
    };
    if git(&["status", "--porcelain"]).is_empty() {
        hash
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        format!("{hash}-dirty-{}", stamp(now))
    }
}

/// Builds the app and copies it into the preview folder; returns the new file.
pub fn make_preview() -> Result<PathBuf, String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "pg-app"])
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err("the build failed".to_owned());
    }
    let built = Path::new("target")
        .join("release")
        .join(format!("pg-app{}", std::env::consts::EXE_SUFFIX));
    let dir = Path::new(FOLDER);
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {FOLDER}/: {e}"))?;
    let out = dir.join(format!(
        "{PREFIX}{}{}",
        build_string(),
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(&built, &out)
        .map_err(|e| format!("cannot copy {} to {}: {e}", built.display(), out.display()))?;
    prune(dir, &out);
    Ok(out)
}

/// Removes all but the newest [`KEEP`] previews (never the one just made).
fn prune(dir: &Path, keep_this: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<(SystemTime, PathBuf)> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, p) in found.into_iter().skip(KEEP) {
        if p != keep_this {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn preview_cmd(_args: &[String]) -> Result<ExitCode, String> {
    let out = make_preview()?;
    println!("preview build: {}", out.display());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_is_month_day_hour_minute_in_utc() {
        assert_eq!(stamp(0), "0101-0000");
        // 2026-10-28 21:54:00 UTC.
        assert_eq!(stamp(1_793_224_440), "1028-2154");
        // A leap day.
        assert_eq!(stamp(1_709_210_100), "0229-1235");
    }

    #[test]
    fn the_build_string_is_never_empty_and_has_no_spaces() {
        let s = build_string();
        assert!(!s.is_empty() && !s.contains(' '));
    }
}
