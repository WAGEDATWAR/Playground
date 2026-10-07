//! AI settings and key handling (Blueprint §10.2, Roadmap Stage 0 "options flow").
//!
//! Settings hold the provider choice, an optional custom model id and an on/off switch. They are written to
//! `settings/device.json`. **They never hold a key**: keys live only in the [`SecretStore`] under
//! `playground.ai.<provider>`. The player enters a key, it is validated and stored, and from then on only
//! the store ever sees it again.

use crate::provider::Provider;
use pg_core::canon::{json, Canon};
use pg_core::read::{ReadError, Reader, Root};
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

/// Settings stored per device (not per world). Only the AI section exists so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceSettings {
    pub ai: AiSettings,
}

impl DeviceSettings {
    pub fn to_canon(&self) -> Canon {
        Canon::map([
            ("version", Canon::Int(1)),
            (
                "ai",
                Canon::map([
                    ("enabled", Canon::Bool(self.ai.enabled)),
                    ("provider", Canon::str(self.ai.provider.id())),
                    (
                        "custom_model",
                        self.ai.custom_model.clone().map_or(Canon::Null, Canon::Str),
                    ),
                ]),
            ),
        ])
    }

    fn from_reader(r: Reader<'_>) -> Result<DeviceSettings, ReadError> {
        // Top level is lenient (other sections arrive later and an older build must not choke on them);
        // the AI section is strict so a typo is reported rather than ignored.
        let ai = match r.maybe("ai")? {
            None => return Ok(DeviceSettings::default()),
            Some(a) => a,
        };
        let a = ai.reader();
        a.only(&["enabled", "provider", "custom_model"])?;
        let provider_child = a.child("provider")?;
        let provider = Provider::from_id(provider_child.reader().str()?).ok_or_else(|| {
            provider_child
                .reader()
                .err("unknown provider (openai, deepseek, anthropic, openrouter or player2)")
        })?;
        let custom_model = match a.maybe("custom_model")? {
            Some(m) => {
                let id = m.reader().str()?;
                validate_model_id(id).map_err(|e| m.reader().err(e))?;
                Some(id.to_owned())
            }
            None => None,
        };
        if provider.chooses_own_model() && custom_model.is_some() {
            return Err(a.child("custom_model")?.reader().err(format!(
                "{provider} chooses its own model; leave this empty"
            )));
        }
        Ok(DeviceSettings {
            ai: AiSettings {
                enabled: a.child("enabled")?.reader().bool()?,
                provider,
                custom_model,
            },
        })
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
        let root = Root::new(json::parse(&text).map_err(|e| SettingsError::Parse(e.to_string()))?);
        DeviceSettings::from_reader(root.reader()).map_err(|e| SettingsError::Parse(e.to_string()))
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
        storage
            .write_atomic(
                DEVICE_FILE,
                self.to_canon().to_canonical_string().as_bytes(),
            )
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
            br#"{"ai":{"enabled":true}}"#,
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
        assert_eq!(log.lines().len(), 8);
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
}
