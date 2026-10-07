//! The settings registry (suggestion S-025, Blueprint §13.1).
//!
//! Every setting is declared once: an id of the form `section.name`, a typed [`Field`] (type, range and
//! default, from the same schema machinery as components), a label key into the string tables, whether it
//! belongs to the device or to a world, and whether a change needs a restart. One declaration then gives:
//! validation of `settings/device.json`, defaults, `pg settings list|get|set`, and (milestone 0.10) the
//! Options screen, generated rather than hand-built. Secrets are never settings.
//!
//! **File shape.** `{ "version": 1, "<section>": { "<name>": value } }`. Unknown *sections* are ignored (a
//! newer build may have added them); an unknown *name inside a known section* is an error, so a typo or a
//! stray credential is reported rather than silently kept.

use crate::report::ValidationReport;
use crate::schema::Field;
use pg_canon::Canon;
use std::collections::{BTreeMap, BTreeSet};

pub const SETTINGS_VERSION: i128 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Per machine (`settings/device.json`).
    Device,
    /// Saved with a world.
    World,
}

#[derive(Clone, Debug)]
pub struct SettingDef {
    /// `section.name`, lower-case.
    pub id: String,
    pub field: Field,
    /// Key into the string tables for the label shown in the UI (`settings.<id>`).
    pub label_key: String,
    pub scope: Scope,
    pub restart_required: bool,
}

/// A check across several settings, run after every change (for example "this provider takes no custom
/// model"). Returns a plain sentence on failure.
pub type Rule = fn(&SettingsValues) -> Result<(), String>;

/// Current values by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsValues {
    values: BTreeMap<String, Canon>,
}

impl SettingsValues {
    pub fn get(&self, id: &str) -> Option<&Canon> {
        self.values.get(id)
    }

    pub fn bool(&self, id: &str) -> Option<bool> {
        self.get(id).and_then(Canon::as_bool)
    }

    pub fn text(&self, id: &str) -> Option<&str> {
        self.get(id).and_then(Canon::as_str)
    }

    pub fn int(&self, id: &str) -> Option<i64> {
        self.get(id).and_then(Canon::as_i64)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Canon)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }
}

#[derive(Clone, Debug, Default)]
pub struct SettingsRegistry {
    defs: BTreeMap<String, SettingDef>,
    rules: Vec<(&'static str, Rule)>,
}

fn valid_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    [a, b].iter().all(|p| {
        !p.is_empty()
            && p.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
}

impl SettingsRegistry {
    pub fn new() -> SettingsRegistry {
        SettingsRegistry::default()
    }

    /// Declares a setting. Refused for a bad id, a duplicate, a field with no default, or a default that
    /// does not satisfy the field's own schema.
    pub fn register(
        &mut self,
        id: &str,
        field: Field,
        scope: Scope,
        restart_required: bool,
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err(format!(
                "'{id}' is not a setting id (section.name, lower-case)"
            ));
        }
        if self.defs.contains_key(id) {
            return Err(format!("setting '{id}' is already registered"));
        }
        let Some(default) = field.default.clone() else {
            return Err(format!("setting '{id}' needs a default"));
        };
        let mut r = ValidationReport::new();
        field.schema.check(&default, id, &mut r);
        if !r.is_ok() {
            return Err(format!(
                "the default of '{id}' is invalid: {}",
                r.errors()
                    .next()
                    .map_or(String::new(), |i| i.message.clone())
            ));
        }
        self.defs.insert(
            id.to_owned(),
            SettingDef {
                id: id.to_owned(),
                field,
                label_key: format!("settings.{id}"),
                scope,
                restart_required,
            },
        );
        Ok(())
    }

    pub fn add_rule(&mut self, name: &'static str, rule: Rule) {
        self.rules.push((name, rule));
    }

    pub fn get(&self, id: &str) -> Option<&SettingDef> {
        self.defs.get(id)
    }

    /// Definitions in id order.
    pub fn iter(&self) -> impl Iterator<Item = &SettingDef> {
        self.defs.values()
    }

    fn sections(&self) -> BTreeSet<&str> {
        self.defs
            .keys()
            .filter_map(|k| k.split('.').next())
            .collect()
    }

    pub fn defaults(&self) -> SettingsValues {
        SettingsValues {
            values: self
                .defs
                .iter()
                .filter_map(|(id, d)| Some((id.clone(), d.field.default.clone()?)))
                .collect(),
        }
    }

    fn check_rules(&self, values: &SettingsValues) -> Result<(), String> {
        for (_, rule) in &self.rules {
            rule(values)?;
        }
        Ok(())
    }

    /// Sets one value, validating its type and range and then the cross-setting rules. The values are
    /// unchanged on failure.
    pub fn set(&self, values: &mut SettingsValues, id: &str, value: Canon) -> Result<(), String> {
        let def = self.defs.get(id).ok_or_else(|| {
            format!(
                "no setting '{id}'{}",
                crate::hints::hint(id, self.defs.keys().map(String::as_str))
            )
        })?;
        let mut r = ValidationReport::new();
        let clean = def.field.schema.check(&value, id, &mut r);
        if let Some(e) = r.errors().next() {
            return Err(e.message.clone());
        }
        let mut trial = values.clone();
        trial.values.insert(id.to_owned(), clean);
        self.check_rules(&trial)?;
        *values = trial;
        Ok(())
    }

    /// Reads a settings document. Problems are collected in the report; every setting with a problem keeps
    /// its default, so the returned values are always usable. `report.is_ok()` is the strict verdict.
    pub fn parse(&self, doc: &Canon) -> (SettingsValues, ValidationReport) {
        let mut report = ValidationReport::new();
        let mut values = self.defaults();
        let Canon::Map(top) = doc else {
            report.error("type_mismatch", "", "settings must be an object");
            return (values, report);
        };
        let sections = self.sections();
        for (section, body) in top {
            if section == "version" || !sections.contains(section.as_str()) {
                continue; // metadata, or a section a newer build added
            }
            let Canon::Map(entries) = body else {
                report.error("type_mismatch", section, "a settings section is an object");
                continue;
            };
            for (name, value) in entries {
                let id = format!("{section}.{name}");
                match self.defs.get(&id) {
                    None => report.error(
                        "unknown_setting",
                        id.clone(),
                        "no such setting in this section",
                    ),
                    Some(def) => {
                        let mut r = ValidationReport::new();
                        let clean = def.field.schema.check(value, &id, &mut r);
                        if r.is_ok() {
                            values.values.insert(id, clean);
                        } else {
                            report.merge(r);
                        }
                    }
                }
            }
        }
        if let Err(why) = self.check_rules(&values) {
            report.error("setting_rule", "", why);
            // A rule failure means the combination is unusable: fall back to defaults for everything.
            values = self.defaults();
        }
        (values, report)
    }

    /// The document form of `values`.
    pub fn to_canon(&self, values: &SettingsValues) -> Canon {
        let mut sections: BTreeMap<String, BTreeMap<String, Canon>> = BTreeMap::new();
        for (id, v) in &values.values {
            if let Some((section, name)) = id.split_once('.') {
                sections
                    .entry(section.to_owned())
                    .or_default()
                    .insert(name.to_owned(), v.clone());
            }
        }
        let mut top: BTreeMap<String, Canon> = sections
            .into_iter()
            .map(|(k, v)| (k, Canon::Map(v)))
            .collect();
        top.insert("version".to_owned(), Canon::Int(SETTINGS_VERSION));
        Canon::Map(top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldSchema;
    use pg_canon::json::parse;

    fn registry() -> SettingsRegistry {
        let mut r = SettingsRegistry::new();
        r.register(
            "ui.scale_percent",
            Field::int(50, 300, 100),
            Scope::Device,
            false,
        )
        .unwrap();
        r.register(
            "ui.window_mode",
            Field::enumeration(&["windowed", "borderless", "fullscreen"], "windowed"),
            Scope::Device,
            true,
        )
        .unwrap();
        r.register("ai.enabled", Field::boolean(false), Scope::Device, false)
            .unwrap();
        r.register(
            "ai.note",
            Field::maybe(FieldSchema::Text { max_len: 20 }),
            Scope::Device,
            false,
        )
        .unwrap();
        r.add_rule("fullscreen_needs_big_scale", |v| {
            if v.text("ui.window_mode") == Some("fullscreen")
                && v.int("ui.scale_percent") == Some(50)
            {
                Err("fullscreen at 50% scale is unreadable".to_owned())
            } else {
                Ok(())
            }
        });
        r
    }

    fn doc(json: &str) -> Canon {
        parse(json).unwrap()
    }

    #[test]
    fn registration_validates_ids_defaults_and_duplicates() {
        let mut r = SettingsRegistry::new();
        for bad in ["noDot", "a.b.c", "A.b", "a.", ".b", "1a.b", "a.b-c"] {
            assert!(
                r.register(bad, Field::boolean(true), Scope::Device, false)
                    .is_err(),
                "{bad}"
            );
        }
        r.register("a.b", Field::boolean(true), Scope::Device, false)
            .unwrap();
        assert!(r
            .register("a.b", Field::boolean(true), Scope::Device, false)
            .is_err());
        assert!(
            r.register(
                "a.c",
                Field::required(FieldSchema::Bool),
                Scope::Device,
                false
            )
            .is_err(),
            "no default"
        );
        assert!(
            r.register("a.d", Field::int(1, 5, 9), Scope::Device, false)
                .is_err(),
            "default out of range"
        );
        assert_eq!(r.iter().count(), 1);
        assert_eq!(r.get("a.b").unwrap().label_key, "settings.a.b");
    }

    #[test]
    fn defaults_and_round_trip() {
        let r = registry();
        let d = r.defaults();
        assert_eq!(d.int("ui.scale_percent"), Some(100));
        assert_eq!(d.text("ui.window_mode"), Some("windowed"));
        assert_eq!(d.bool("ai.enabled"), Some(false));
        let text = r.to_canon(&d).to_canonical_string();
        assert_eq!(
            text,
            r#"{"ai":{"enabled":false,"note":null},"ui":{"scale_percent":100,"window_mode":"windowed"},"version":1}"#
        );
        let (back, report) = r.parse(&doc(&text));
        assert!(report.is_ok(), "{report}");
        assert_eq!(back, d);
    }

    #[test]
    fn bad_values_keep_their_default_and_are_reported_with_the_setting_id() {
        let r = registry();
        let (v, report) = r.parse(&doc(
            r#"{"ui":{"scale_percent":9000,"window_mode":"sideways"},"ai":{"enabled":true}}"#,
        ));
        assert_eq!(report.error_count(), 2);
        assert!(report
            .errors()
            .any(|i| i.path == "ui.scale_percent" && i.code == "out_of_range"));
        assert!(report.errors().any(|i| i.path == "ui.window_mode"));
        assert_eq!(v.int("ui.scale_percent"), Some(100), "kept the default");
        assert_eq!(v.bool("ai.enabled"), Some(true), "good values still load");
    }

    #[test]
    fn unknown_names_in_known_sections_are_errors_but_unknown_sections_are_ignored() {
        let r = registry();
        let (_, report) = r.parse(&doc(r#"{"ai":{"enabled":true,"api_key":"sk-12345678"}}"#));
        assert!(report.has_code("unknown_setting"));
        assert!(
            !report.to_string().contains("sk-12345678"),
            "the stray value is not echoed"
        );
        let (v, report) = r.parse(&doc(
            r#"{"version":9,"audio":{"volume":3},"ai":{"enabled":true}}"#,
        ));
        assert!(report.is_ok(), "{report}");
        assert_eq!(v.bool("ai.enabled"), Some(true));
        assert!(!r.parse(&doc("[]")).1.is_ok());
        assert!(!r.parse(&doc(r#"{"ai":5}"#)).1.is_ok());
    }

    #[test]
    fn set_validates_type_range_and_cross_rules_and_leaves_values_alone_on_failure() {
        let r = registry();
        let mut v = r.defaults();
        r.set(&mut v, "ui.scale_percent", Canon::Int(150)).unwrap();
        assert_eq!(v.int("ui.scale_percent"), Some(150));
        let before = v.clone();
        assert!(r.set(&mut v, "ui.scale_percent", Canon::Int(5)).is_err());
        assert!(r
            .set(&mut v, "ui.scale_percent", Canon::str("big"))
            .is_err());
        assert!(r
            .set(&mut v, "ui.window_mode", Canon::str("sideways"))
            .is_err());
        let e = r.set(&mut v, "ui.nope", Canon::Int(1)).unwrap_err();
        assert!(e.contains("no setting 'ui.nope'"), "{e}");
        assert_eq!(v, before);
        // The cross-setting rule.
        r.set(&mut v, "ui.window_mode", Canon::str("fullscreen"))
            .unwrap();
        r.set(&mut v, "ui.scale_percent", Canon::Int(100)).unwrap();
        let e = r
            .set(&mut v, "ui.scale_percent", Canon::Int(50))
            .unwrap_err();
        assert!(e.contains("unreadable"), "{e}");
        assert_eq!(v.int("ui.scale_percent"), Some(100));
    }

    #[test]
    fn a_failing_rule_in_a_file_falls_back_to_defaults_and_says_why() {
        let r = registry();
        let (v, report) = r.parse(&doc(
            r#"{"ui":{"window_mode":"fullscreen","scale_percent":50}}"#,
        ));
        assert!(report.has_code("setting_rule"));
        assert_eq!(v, r.defaults());
    }
}
