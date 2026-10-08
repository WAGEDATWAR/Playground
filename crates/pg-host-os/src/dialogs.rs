//! The operating system's own open and save dialogs (Stage 1, milestone 1.7; suggestion S-037), through the
//! `rfd` crate. Windows and macOS use their native pickers; on Linux the desktop portal is asked, so it works
//! under both X11 and Wayland without GTK being installed.
//!
//! The dialogs are blocking and are meant to be called from the thread that owns the window.

use pg_host::Dialogs;
use std::path::PathBuf;

/// What the player sees when choosing a file.
pub struct NativeDialogs {
    title: String,
}

impl NativeDialogs {
    pub fn new(title: &str) -> NativeDialogs {
        NativeDialogs {
            title: title.to_owned(),
        }
    }
}

impl Dialogs for NativeDialogs {
    fn pick_file_to_read(&self, extensions: &[&str]) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title(format!("{}: open", self.title));
        if !extensions.is_empty() {
            d = d.add_filter(self.title.clone(), extensions);
        }
        d.pick_file()
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(format!("{}: choose the pack folder", self.title))
            .pick_folder()
    }

    fn pick_file_to_write(&self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(format!("{}: save", self.title))
            .set_file_name(suggested)
            .save_file()
    }
}
