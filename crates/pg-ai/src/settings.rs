//! AI settings and key handling (Blueprint §10.2, Roadmap Stage 0 "options flow").
//!
//! Settings hold the provider choice, an optional custom model id and an on/off switch. They are written to
//! `settings/device.json`. **They never hold a key**: keys live only in the [`SecretStore`] under
//! `playground.ai.<provider>`. The player enters a key, it is validated and stored, and from then on only
//! the store ever sees it again.

use crate::provider::Provider;
use pg_content::schema::{Field, FieldSchema};
use pg_content::settings::{Scope, SettingsRegistry, SettingsValues};
use pg_core::canon::{json, Canon};
use pg_host::{Level, LogSink, Secret, SecretError, SecretStore, Storage};
use std::fmt;

pub const DEVICE_FILE: &str = "settings/device.json";
const MAX_MODEL_LEN: usize = 100;
const MIN_KEY_LEN: usize = 8;
const MAX_KEY_LEN: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: Provider,
    /// A model id typed by the player; `None` means the provider's recommended model.
    pub custom_model: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            enabled: false,
            provider: Provider::OpenAi,
            custom_model: None,
        }
    }
}

impl AiSettings {
    /// The model id to send (empty when the provider chooses its own).
    pub fn effective_model(&self) -> String {
        if self.provider.chooses_own_model() {
            return String::new();
        }
        self.custom_model
            .clone()
            .unwrap_or_else(|| self.provider.recommended_model().to_owned())
    }
}

/// A model id is a short token: letters, digits and `. _ - : /`.
pub fn validate_model_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("the model id is empty".to_owned());
    }
    if id.len() > MAX_MODEL_LEN {
        return Err(format!(
            "the model id is longer than {MAX_MODEL_LEN} characters"
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
    {
        return Err("the model id may only contain letters, digits and . _ - : /".to_owned());
    }
    Ok(())
}

/// Checks pasted key text and wraps it. Surrounding whitespace (a common paste artefact) is trimmed.
pub fn validate_key(text: &str) -> Result<Secret, String> {
    let t = text.trim();
    if t.len() < MIN_KEY_LEN {
        return Err(format!(
            "that is too short to be a key (at least {MIN_KEY_LEN} characters)"
        ));
    }
    if t.len() > MAX_KEY_LEN {
        return Err(format!(
            "that is too long to be a key (at most {MAX_KEY_LEN} characters)"
        ));
    }
    if !t.chars().all(|c| c.is_ascii_graphic()) {
        return Err(
            "a key contains only visible ASCII characters, with no spaces or line breaks"
                .to_owned(),
        );
    }
    Ok(Secret::new(t))
}

#[derive(Debug)]
pub enum SettingsError {
    Io(String),
    Parse(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(e) => write!(f, "could not read settings: {e}"),
            SettingsError::Parse(e) => write!(f, "settings file is invalid: {e}"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// Declares the AI settings in a registry (S-025).
pub fn register_ai_settings(r: &mut SettingsRegistry) -> Result<(), String> {
    r.register("ai.enabled", Field::boolean(false), Scope::Device, false)?;
    let ids: Vec<&str> = Provider::ALL.iter().map(|p| p.id()).collect();
    r.register(
        "ai.provider",
        Field::enumeration(&ids, "openai"),
        Scope::Device,
        false,
    )?;
    r.register(
        "ai.custom_model",
        Field::maybe(FieldSchema::Text {
            max_len: MAX_MODEL_LEN,
        }),
        Scope::Device,
        false,
    )?;
    r.add_rule("ai.custom_model", |v: &SettingsValues| {
        let provider = v.text("ai.provider").and_then(Provider::from_id);
        match v.text("ai.custom_model") {
            None => Ok(()),
            Some(m) => {
                validate_model_id(m)?;
                match provider {
                    Some(p) if p.chooses_own_model() => {
                        Err(format!("{p} chooses its own model; leave the model empty"))
                    }
                    _ => Ok(()),
                }
            }
        }
    });
    Ok(())
}

/// A registry holding only the AI settings.
pub fn ai_registry() -> SettingsRegistry {
    let mut r = SettingsRegistry::new();
    // The ids are fixed and distinct, so registration cannot fail.
    let _ = register_ai_settings(&mut r);
    r
}

impl AiSettings {
    /// Reads the AI settings out of registry values.
    pub fn from_values(v: &SettingsValues) -> AiSettings {
        AiSettings {
            enabled: v.bool("ai.enabled").unwrap_or(false),
            provider: v
                .text("ai.provider")
                .and_then(Provider::from_id)
                .unwrap_or(Provider::OpenAi),
            custom_model: v.text("ai.custom_model").map(str::to_owned),
        }
    }

    /// Writes these settings into `values` through the registry (so ranges and rules apply).
    pub fn write_into(
        &self,
        r: &SettingsRegistry,
        values: &mut SettingsValues,
    ) -> Result<(), String> {
        // Clear the model first so switching to a provider that takes none cannot trip the rule.
        r.set(values, "ai.custom_model", Canon::Null)?;
        r.set(values, "ai.provider", Canon::str(self.provider.id()))?;
        r.set(values, "ai.enabled", Canon::Bool(self.enabled))?;
        r.set(
            values,
            "ai.custom_model",
            self.custom_model.clone().map_or(Canon::Null, Canon::Str),
        )
    }
}

/// Settings stored per device (not per world), as far as this crate knows them. The file may hold other
/// sections (written by other parts of the app); loading ignores them and saving **keeps them untouched**.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceSettings {
    pub ai: AiSettings,
}

impl DeviceSettings {
    pub fn to_canon(&self) -> Canon {
        let r = ai_registry();
        let mut v = r.defaults();
        // Settings that fail the registry's rules cannot be built; fall back to the defaults.
        if self.ai.write_into(&r, &mut v).is_err() {
            v = r.defaults();
        }
        r.to_canon(&v)
    }

    pub fn load(storage: &dyn Storage) -> Result<DeviceSettings, SettingsError> {
        let Some(bytes) = storage
            .read(DEVICE_FILE)
            .map_err(|e| SettingsError::Io(e.to_string()))?
        else {
            return Ok(DeviceSettings::default());
        };
        let text =
            String::from_utf8(bytes).map_err(|_| SettingsError::Parse("not UTF-8".to_owned()))?;
        let doc = json::parse(&text).map_err(|e| SettingsError::Parse(e.to_string()))?;
        let (values, report) = ai_registry().parse(&doc);
        let first = report.errors().next().map(|e| {
            if e.path.is_empty() {
                e.message.clone()
            } else {
                format!("{}: {}", e.path, e.message)
            }
        });
        match first {
            Some(msg) => Err(SettingsError::Parse(msg)),
            None => Ok(DeviceSettings {
                ai: AiSettings::from_values(&values),
            }),
        }
    }

    /// Loads settings, falling back to defaults (and saying so in the log) if the file is damaged, so a bad
    /// settings file never stops the game from starting.
    pub fn load_or_default(storage: &dyn Storage, log: &dyn LogSink) -> DeviceSettings {
        match DeviceSettings::load(storage) {
            Ok(s) => s,
            Err(e) => {
                log.log(Level::Warn, &format!("{e}; using default settings"));
                DeviceSettings::default()
            }
        }
    }

    pub fn save(&self, storage: &dyn Storage) -> Result<(), SettingsError> {
        // Keep sections this crate does not own (a damaged or missing file simply starts fresh).
        let mut doc = match storage.read(DEVICE_FILE) {
            Ok(Some(bytes)) => String::from_utf8(bytes)
                .ok()
                .and_then(|t| json::parse(&t).ok())
                .unwrap_or(Canon::Null),
            _ => Canon::Null,
        };
        let mine = self.to_canon();
        let merged = match (&mut doc, mine) {
            (Canon::Map(existing), Canon::Map(new)) => {
                for (k, v) in new {
                    existing.insert(k, v);
                }
                doc
            }
            (_, new) => new,
        };
        storage
            .write_atomic(DEVICE_FILE, merged.to_canonical_string().as_bytes())
            .map_err(|e| SettingsError::Io(e.to_string()))
    }
}

/// Key operations for the options screen. Thin on purpose: validation, then the store.
pub struct KeyManager<'a> {
    store: &'a dyn SecretStore,
}

impl<'a> KeyManager<'a> {
    pub fn new(store: &'a dyn SecretStore) -> KeyManager<'a> {
        KeyManager { store }
    }

    /// Validates and stores a key typed or pasted by the player.
    pub fn set_key(&self, provider: Provider, text: &str) -> Result<(), String> {
        let key = validate_key(text)?;
        self.store
            .set(&provider.secret_name(), &key)
            .map_err(|e: SecretError| e.to_string())
    }

    pub fn clear_key(&self, provider: Provider) -> Result<(), String> {
        self.store
            .delete(&provider.secret_name())
            .map_err(|e| e.to_string())
    }

    pub fn has_key(&self, provider: Provider) -> bool {
        matches!(self.store.get(&provider.secret_name()), Ok(Some(_)))
    }

    /// Whether stored keys survive a restart (shown in the options screen).
    pub fn is_persistent(&self) -> bool {
        self.store.is_persistent()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_host::{MemLog, MemSecretStore, MemStorage};

    #[test]
    fn defaults_are_off_and_use_the_recommended_model() {
        let s = AiSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.effective_model(), Provider::OpenAi.recommended_model());
        let c = AiSettings {
            custom_model: Some("my-model".into()),
            ..s
        };
        assert_eq!(c.effective_model(), "my-model");
    }

    #[test]
    fn model_ids_are_validated() {
        for ok in [
            "gpt-4o-mini",
            "anthropic/claude-3.5-sonnet",
            "llama3:8b",
            "a",
        ] {
            assert!(validate_model_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "has space",
            "semi;colon",
            "new\nline",
            "quote\"",
            &"x".repeat(101),
            "é",
        ] {
            assert!(validate_model_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keys_are_validated_trimmed_and_never_echoed_in_errors() {
        assert_eq!(
            validate_key("  sk-abcdef123456\n").unwrap().expose(),
            "sk-abcdef123456"
        );
        for bad in [
            "",
            "abc1234",
            "has space inside1",
            "tab\tinside1234",
            "émoji-key-12345",
            &"k".repeat(401),
        ] {
            let e = validate_key(bad).unwrap_err();
            assert!(!e.contains(bad) || bad.len() < 3, "{e}");
        }
    }

    #[test]
    fn settings_round_trip_through_storage_and_never_contain_a_key() {
        let mem = MemStorage::new();
        assert_eq!(
            DeviceSettings::load(&mem).unwrap(),
            DeviceSettings::default()
        );
        let s = DeviceSettings {
            ai: AiSettings {
                enabled: true,
                provider: Provider::Anthropic,
                custom_model: Some("claude-x".into()),
            },
        };
        s.save(&mem).unwrap();
        assert_eq!(DeviceSettings::load(&mem).unwrap(), s);
        let stored = String::from_utf8(mem.get_raw(DEVICE_FILE).unwrap()).unwrap();
        assert_eq!(
            stored,
            r#"{"ai":{"custom_model":"claude-x","enabled":true,"provider":"anthropic"},"version":1}"#
        );
        assert!(!stored.to_ascii_lowercase().contains("key"));
    }

    #[test]
    fn damaged_settings_fall_back_to_defaults_with_a_warning() {
        let log = MemLog::new();
        for bad in [
            &b"{{{"[..],
            br#"{"ai":{"enabled":true,"provider":"nobody","custom_model":null}}"#,
            br#"{"ai":{"enabled":true,"provider":"openai","custom_model":"bad model!"}}"#,
            br#"{"ai":{"enabled":"yes","provider":"openai","custom_model":null}}"#,
            br#"{"ai":{"enabled":true,"provider":"openai","custom_model":null,"api_key":"sk-12345678"}}"#,
            br#"{"ai":{"enabled":true,"provider":"player2","custom_model":"gpt-4o"}}"#,
            &[0xFF, 0xFE][..],
        ] {
            let mem = MemStorage::new();
            mem.put_raw(DEVICE_FILE, bad.to_vec());
            assert!(DeviceSettings::load(&mem).is_err(), "{:?}", String::from_utf8_lossy(bad));
            assert_eq!(DeviceSettings::load_or_default(&mem, &log), DeviceSettings::default());
        }
        assert_eq!(log.lines().len(), 7);
        assert!(log.lines().iter().all(|(l, _)| *l == Level::Warn));
        // The stray key field is refused, and the refusal does not repeat the key.
        assert!(!log.text().contains("sk-12345678"));
    }

    #[test]
    fn other_sections_are_ignored_so_older_builds_can_read_newer_files() {
        let mem = MemStorage::new();
        mem.put_raw(DEVICE_FILE, br#"{"version":2,"audio":{"volume":3},"ai":{"enabled":true,"provider":"deepseek","custom_model":null}}"#.to_vec());
        let s = DeviceSettings::load(&mem).unwrap();
        assert_eq!(s.ai.provider, Provider::DeepSeek);
    }

    #[test]
    fn key_manager_stores_by_provider_and_reports_persistence() {
        let store = MemSecretStore::new();
        let k = KeyManager::new(&store);
        assert!(!k.has_key(Provider::OpenAi));
        assert!(k.set_key(Provider::OpenAi, "abc1234").is_err());
        k.set_key(Provider::OpenAi, "  sk-abcdef123456 ").unwrap();
        assert!(k.has_key(Provider::OpenAi) && !k.has_key(Provider::Anthropic));
        assert_eq!(store.names(), ["playground.ai.openai"]);
        assert!(k.is_persistent());
        k.clear_key(Provider::OpenAi).unwrap();
        assert!(!k.has_key(Provider::OpenAi));
        assert!(!KeyManager::new(&MemSecretStore::session_only()).is_persistent());
        store.fail_with(Some(SecretError::Denied("locked".into())));
        assert!(k.set_key(Provider::OpenAi, "sk-abcdef123456").is_err());
        assert!(!k.has_key(Provider::OpenAi));
    }

    #[test]
    fn missing_entries_take_their_defaults_and_saving_keeps_other_sections() {
        let mem = MemStorage::new();
        mem.put_raw(
            DEVICE_FILE,
            br#"{"version":1,"ui":{"scale_percent":150},"ai":{"enabled":true}}"#.to_vec(),
        );
        let s = DeviceSettings::load(&mem).unwrap();
        assert!(s.ai.enabled && s.ai.provider == Provider::OpenAi && s.ai.custom_model.is_none());
        let changed = DeviceSettings {
            ai: AiSettings {
                provider: Provider::DeepSeek,
                ..s.ai
            },
        };
        changed.save(&mem).unwrap();
        let stored = String::from_utf8(mem.get_raw(DEVICE_FILE).unwrap()).unwrap();
        assert!(stored.contains(r#""ui":{"scale_percent":150}"#), "{stored}");
        assert_eq!(DeviceSettings::load(&mem).unwrap(), changed);
    }

    #[test]
    fn the_registry_drives_the_ai_settings_and_enforces_the_model_rules() {
        let r = ai_registry();
        let mut v = r.defaults();
        assert_eq!(AiSettings::from_values(&v), AiSettings::default());
        r.set(&mut v, "ai.provider", Canon::str("player2")).unwrap();
        assert!(
            r.set(&mut v, "ai.custom_model", Canon::str("gpt-4o"))
                .is_err(),
            "player2 takes no model"
        );
        r.set(&mut v, "ai.provider", Canon::str("openai")).unwrap();
        r.set(&mut v, "ai.custom_model", Canon::str("gpt-4o"))
            .unwrap();
        assert!(
            r.set(&mut v, "ai.provider", Canon::str("player2")).is_err(),
            "switching would strand the model"
        );
        assert!(r
            .set(&mut v, "ai.custom_model", Canon::str("bad model!"))
            .is_err());
        assert!(r.set(&mut v, "ai.provider", Canon::str("nobody")).is_err());
        // write_into clears the model first, so a settings object for Player2 always applies cleanly.
        let p2 = AiSettings {
            enabled: true,
            provider: Provider::Player2,
            custom_model: None,
        };
        p2.write_into(&r, &mut v).unwrap();
        assert_eq!(AiSettings::from_values(&v), p2);
        assert_eq!(r.iter().count(), 3);
    }
}
