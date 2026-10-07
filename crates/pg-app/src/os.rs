//! The few things that need the operating system: where data lives, opening a link, showing a file.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The folder for saves, settings and bundles: the platform's per-user data folder, then `Playground`.
pub fn default_data_dir() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
    };
    base.map_or_else(|| PathBuf::from("pg-data"), |b| b.join("Playground"))
}

/// Opens `url` in the default browser. Callers only pass links the sign-in code has already checked
/// (https, the provider's domain, no spaces or control characters).
pub fn open_url(url: &str) {
    let result = if cfg!(windows) {
        // `rundll32` takes the link as one argument; `cmd /c start` would reinterpret `&` and `^`.
        Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        eprintln!("could not open the browser: {e}");
    }
}

/// Shows `path` in the system file manager.
pub fn reveal(path: &Path) {
    let result = if cfg!(windows) {
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()
    } else {
        Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()
    };
    if let Err(e) = result {
        eprintln!("could not open the file manager: {e}");
    }
}

/// Where the shipped content is: a `data/base` folder in the working directory, next to the program, or
/// two levels above it (running from `target/debug` inside the repository).
pub fn find_base_pack() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("data/base")];
    if let Ok(exe) = std::env::current_exe() {
        for up in 0..4 {
            let mut dir = exe.clone();
            for _ in 0..=up {
                dir.pop();
            }
            candidates.push(dir.join("data").join("base"));
        }
    }
    candidates
        .into_iter()
        .find(|c| c.join("pack.json").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_dir_ends_in_playground() {
        assert!(
            default_data_dir().ends_with("Playground") || default_data_dir().ends_with("pg-data")
        );
    }

    #[test]
    fn the_base_pack_is_found_from_the_repository() {
        // Tests run with the crate directory as the working directory; the exe is under target/.
        let found = find_base_pack().expect("data/base is reachable from target/debug");
        assert!(found.join("pack.json").is_file());
    }
}
