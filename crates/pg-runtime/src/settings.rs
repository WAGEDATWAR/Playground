//! The device settings registry and file (suggestion S-025, Blueprint §13.1).
//!
//! `device_registry()` declares every device-scope setting the app knows: the runtime's own (autosave,
//! focus-loss pause), the UI's (scale, window mode, vsync, language) and the AI settings from `pg-ai`.
//! [`DeviceFile`] reads and writes `settings/device.json` through it: bad entries keep their defaults and
//! are reported, and sections the registry does not know (written by a newer build) are preserved on save.

use pg_ai::settings::register_ai_settings;
use pg_content::schema::Field;
use pg_content::settings::{Scope, SettingsRegistry, SettingsValues};
use pg_content::ValidationReport;
use pg_core::canon::{json, Canon};
use pg_host::Storage;

pub const DEVICE_FILE: &str = pg_ai::settings::DEVICE_FILE;

/// Every device setting the app knows.
pub fn device_registry() -> SettingsRegistry {
    let mut r = SettingsRegistry::new();
    // Ids are fixed and distinct, so registration cannot fail; the tests prove it.
    let _ = r.register(
        "time.autosave_minutes",
        Field::int(1, 60, 5),
        Scope::Device,
        false,
    );
    let _ = r.register(
        "time.pause_on_focus_loss",
        Field::boolean(true),
        Scope::Device,
        false,
    );
    let _ = r.register(
        "ui.scale_percent",
        Field::int(50, 300, 100),
        Scope::Device,
        false,
    );
    let _ = r.register(
        "ui.window_mode",
        Field::enumeration(&["windowed", "borderless", "fullscreen"], "windowed"),
        Scope::Device,
        false,
    );
    let _ = r.register("ui.vsync", Field::boolean(true), Scope::Device, false);
    let _ = r.register("ui.language", Field::text(16, "en"), Scope::Device, false);
    let _ = register_ai_settings(&mut r);
    r
}

/// Reads and writes the device settings document.
pub struct DeviceFile<'a> {
    storage: &'a dyn Storage,
    registry: SettingsRegistry,
}

impl<'a> DeviceFile<'a> {
    pub fn new(storage: &'a dyn Storage, registry: SettingsRegistry) -> DeviceFile<'a> {
        DeviceFile { storage, registry }
    }

    pub fn registry(&self) -> &SettingsRegistry {
        &self.registry
    }

    fn read_doc(&self) -> Option<Canon> {
        let bytes = self.storage.read(DEVICE_FILE).ok()??;
        json::parse(&String::from_utf8(bytes).ok()?).ok()
    }

    /// The saved values (defaults where missing or invalid) and what was wrong with the file.
    pub fn load(&self) -> (SettingsValues, ValidationReport) {
        match self.read_doc() {
            Some(doc) => self.registry.parse(&doc),
            None => (self.registry.defaults(), ValidationReport::new()),
        }
    }

    /// Writes `values`, keeping any sections of the existing file that this build does not know.
    pub fn save(&self, values: &SettingsValues) -> Result<(), String> {
        let mut doc = self.registry.to_canon(values);
        if let (Some(Canon::Map(old)), Canon::Map(new)) = (self.read_doc(), &mut doc) {
            let known: std::collections::BTreeSet<String> = new.keys().cloned().collect();
            for (k, v) in old {
                if !known.contains(&k) {
                    new.insert(k, v);
                }
            }
        }
        self.storage
            .write_atomic(DEVICE_FILE, doc.to_canonical_string().as_bytes())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_host::MemStorage;

    #[test]
    fn the_registry_has_every_setting_with_valid_defaults() {
        let r = device_registry();
        let ids: Vec<&str> = r.iter().map(|d| d.id.as_str()).collect();
        for want in [
            "ai.custom_model",
            "ai.enabled",
            "ai.provider",
            "time.autosave_minutes",
            "time.pause_on_focus_loss",
            "ui.language",
            "ui.scale_percent",
            "ui.vsync",
            "ui.window_mode",
        ] {
            assert!(ids.contains(&want), "{want} missing from {ids:?}");
        }
        let (values, report) = r.parse(&r.to_canon(&r.defaults()));
        assert!(report.is_ok(), "{report}");
        assert_eq!(values, r.defaults());
    }

    #[test]
    fn values_round_trip_through_storage_and_unknown_sections_survive() {
        let mem = MemStorage::new();
        mem.put_raw(
            DEVICE_FILE,
            br#"{"version":1,"audio":{"volume":3},"ui":{"scale_percent":175}}"#.to_vec(),
        );
        let f = DeviceFile::new(&mem, device_registry());
        let (mut v, report) = f.load();
        assert!(report.is_ok(), "{report}");
        assert_eq!(v.int("ui.scale_percent"), Some(175));
        f.registry()
            .set(&mut v, "time.autosave_minutes", Canon::Int(10))
            .unwrap();
        f.save(&v).unwrap();
        let stored = String::from_utf8(mem.get_raw(DEVICE_FILE).unwrap()).unwrap();
        assert!(stored.contains(r#""audio":{"volume":3}"#), "{stored}");
        let (back, report) = DeviceFile::new(&mem, device_registry()).load();
        assert!(report.is_ok());
        assert_eq!(back, v);
    }

    #[test]
    fn a_missing_or_damaged_file_gives_defaults() {
        let mem = MemStorage::new();
        let f = DeviceFile::new(&mem, device_registry());
        assert_eq!(f.load().0, f.registry().defaults());
        mem.put_raw(DEVICE_FILE, b"{{{{".to_vec());
        assert_eq!(f.load().0, f.registry().defaults());
        // And saving over a damaged file repairs it.
        f.save(&f.registry().defaults()).unwrap();
        assert!(f.load().1.is_ok());
    }

    #[test]
    fn bad_entries_keep_their_defaults_and_are_reported() {
        let mem = MemStorage::new();
        mem.put_raw(
            DEVICE_FILE,
            br#"{"time":{"autosave_minutes":9999,"pause_on_focus_loss":false}}"#.to_vec(),
        );
        let f = DeviceFile::new(&mem, device_registry());
        let (v, report) = f.load();
        assert_eq!(report.error_count(), 1);
        assert_eq!(v.int("time.autosave_minutes"), Some(5));
        assert_eq!(v.bool("time.pause_on_focus_loss"), Some(false));
    }
}
