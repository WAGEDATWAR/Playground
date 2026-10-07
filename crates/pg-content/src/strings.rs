//! String tables (suggestion S-026, Blueprint §14.3): every piece of player-visible text is a key into a
//! per-locale table, so translation and pack-supplied wording need no code changes.
//!
//! * Packs ship `data/strings/<locale>.json`: one flat object of `key -> text`. Keys are lower-case dotted
//!   slugs; the base pack may define any key, other packs only keys under their own id (`coffee.menu.order`).
//! * Text may contain `{name}` placeholders filled from parameters at lookup time.
//! * Lookup falls back from the requested locale to `en` and finally to the bracketed key (`[menu.play]`),
//!   so a missing string is visible and never a crash.
//! * The virtual locale `pseudo` is derived from `en` on the fly: letters are accented and the text is
//!   padded by about 40 percent, which exposes clipped layouts and hard-coded text before translators do.
//! * [`Strings::lint`] reports keys missing from a locale, keys with no `en` original, and placeholders
//!   that differ from the English text.

use crate::report::ValidationReport;
use std::collections::{BTreeMap, BTreeSet};

pub const STRINGS_DIR: &str = "data/strings/";
/// The locale every other locale falls back to.
pub const FALLBACK_LOCALE: &str = "en";
/// The derived test locale.
pub const PSEUDO_LOCALE: &str = "pseudo";

const MAX_TEXT_CHARS: usize = 2_000;
const MAX_KEY_LEN: usize = 100;

fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= MAX_KEY_LEN
        && !k.starts_with('.')
        && !k.ends_with('.')
        && !k.contains("..")
        && k.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

fn valid_locale(l: &str) -> bool {
    !l.is_empty()
        && l != PSEUDO_LOCALE
        && l.len() <= 16
        && l.split('-')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The `{name}` placeholders in `text`.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    out.insert(name.to_owned());
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// Fills `{name}` placeholders. A placeholder with no parameter stays visible as written.
pub fn interpolate(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (k, v) in params {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Accents the letters and pads by about 40 percent, leaving `{placeholders}` alone.
pub fn pseudo(text: &str) -> String {
    let mut out = String::from("[");
    let mut in_placeholder = false;
    for c in text.chars() {
        match c {
            '{' => {
                in_placeholder = true;
                out.push(c);
            }
            '}' => {
                in_placeholder = false;
                out.push(c);
            }
            _ if in_placeholder => out.push(c),
            _ => {
                out.push(match c {
                    'a' => 'á',
                    'e' => 'é',
                    'i' => 'í',
                    'o' => 'ó',
                    'u' => 'ú',
                    'c' => 'ç',
                    'n' => 'ñ',
                    'y' => 'ý',
                    'A' => 'Á',
                    'E' => 'É',
                    'I' => 'Í',
                    'O' => 'Ó',
                    'U' => 'Ú',
                    other => other,
                });
            }
        }
    }
    out.extend(std::iter::repeat_n('~', text.chars().count() * 2 / 5));
    out.push(']');
    out
}

/// One locale's table.
pub type Table = BTreeMap<String, String>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strings {
    locales: BTreeMap<String, Table>,
}

/// What [`Strings::lint`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StringLint {
    /// `(locale, key)`: present in `en`, absent from the locale.
    pub missing: Vec<(String, String)>,
    /// `(locale, key)`: present in the locale, absent from `en`.
    pub orphaned: Vec<(String, String)>,
    /// `(locale, key)`: the placeholders differ from the English text.
    pub placeholder_mismatch: Vec<(String, String)>,
    /// Keys in `en` that the caller says nothing uses.
    pub unused: Vec<String>,
}

impl StringLint {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
            && self.orphaned.is_empty()
            && self.placeholder_mismatch.is_empty()
            && self.unused.is_empty()
    }
}

impl Strings {
    pub fn new() -> Strings {
        Strings::default()
    }

    /// Adds one pack's table for `locale`. Returns the keys that were already defined (the caller reports
    /// them; the first definition wins).
    pub fn add(&mut self, locale: &str, table: &Table) -> Vec<String> {
        let t = self.locales.entry(locale.to_owned()).or_default();
        let mut dup = Vec::new();
        for (k, v) in table {
            if t.contains_key(k) {
                dup.push(k.clone());
            } else {
                t.insert(k.clone(), v.clone());
            }
        }
        dup
    }

    pub fn locales(&self) -> Vec<&str> {
        self.locales.keys().map(String::as_str).collect()
    }

    pub fn len(&self, locale: &str) -> usize {
        self.locales.get(locale).map_or(0, BTreeMap::len)
    }

    pub fn is_empty(&self) -> bool {
        self.locales.values().all(BTreeMap::is_empty)
    }

    /// The raw template for `key` in exactly `locale`.
    pub fn get(&self, locale: &str, key: &str) -> Option<&str> {
        self.locales.get(locale)?.get(key).map(String::as_str)
    }

    /// The template for `key`: the requested locale, then English; the pseudo-locale is English transformed.
    pub fn template(&self, locale: &str, key: &str) -> Option<String> {
        if locale == PSEUDO_LOCALE {
            return self.get(FALLBACK_LOCALE, key).map(pseudo);
        }
        self.get(locale, key)
            .or_else(|| self.get(FALLBACK_LOCALE, key))
            .map(str::to_owned)
    }

    /// The text for `key` with placeholders filled; a missing key shows as `[key]`.
    pub fn text(&self, locale: &str, key: &str, params: &[(&str, &str)]) -> String {
        match self.template(locale, key) {
            Some(t) => interpolate(&t, params),
            None => format!("[{key}]"),
        }
    }

    /// Checks every locale against English. `used` (when given) is the set of keys the program refers to;
    /// English keys outside it are reported as unused.
    pub fn lint(&self, used: Option<&BTreeSet<String>>) -> StringLint {
        let mut out = StringLint::default();
        let Some(en) = self.locales.get(FALLBACK_LOCALE) else {
            return out;
        };
        for (locale, table) in &self.locales {
            if locale == FALLBACK_LOCALE {
                continue;
            }
            for (k, v) in en {
                match table.get(k) {
                    None => out.missing.push((locale.clone(), k.clone())),
                    Some(t) if placeholders(t) != placeholders(v) => {
                        out.placeholder_mismatch.push((locale.clone(), k.clone()));
                    }
                    Some(_) => {}
                }
            }
            for k in table.keys() {
                if !en.contains_key(k) {
                    out.orphaned.push((locale.clone(), k.clone()));
                }
            }
        }
        if let Some(used) = used {
            out.unused = en.keys().filter(|k| !used.contains(*k)).cloned().collect();
        }
        out
    }
}

/// Parses one `data/strings/<locale>.json` file. Problems are added to `report`; returns the locale and
/// table when the file is usable.
pub fn parse_strings_file(
    path: &str,
    value: &pg_canon::Canon,
    pack_id: &str,
    is_base: bool,
    report: &mut ValidationReport,
) -> Option<(String, Table)> {
    let stem = path
        .strip_prefix(STRINGS_DIR)
        .and_then(|f| f.strip_suffix(".json"))
        .unwrap_or("");
    if stem.contains('/') || !valid_locale(stem) {
        report.error(
            "bad_locale",
            path,
            "the file name must be a locale such as en.json or pt-BR.json (not 'pseudo')",
        );
        return None;
    }
    let pg_canon::Canon::Map(map) = value else {
        report.error(
            "type_mismatch",
            path,
            "a strings file is one object of key: text",
        );
        return None;
    };
    let mut table = Table::new();
    let mut ok = true;
    for (k, v) in map {
        let here = format!("{path}.{k}");
        if !valid_key(k) {
            report.error(
                "bad_string_key",
                here.clone(),
                "keys are lower-case dotted slugs (a-z, 0-9, _ and .)",
            );
            ok = false;
        } else if !is_base && !k.starts_with(&format!("{pack_id}.")) {
            report.error(
                "string_namespace",
                here.clone(),
                format!("pack strings must start with '{pack_id}.'"),
            );
            ok = false;
        }
        match v.as_str() {
            Some(t)
                if t.chars().count() <= MAX_TEXT_CHARS
                    && !t.chars().any(|c| c.is_control() && c != '\n') =>
            {
                table.insert(k.clone(), t.to_owned());
            }
            Some(_) => {
                report.error("bad_string_text", here, format!("text must be at most {MAX_TEXT_CHARS} characters with no control characters"));
                ok = false;
            }
            None => {
                report.error("type_mismatch", here, "string values must be text");
                ok = false;
            }
        }
    }
    ok.then(|| (stem.to_owned(), table))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json::parse;

    fn table(pairs: &[(&str, &str)]) -> Table {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn sample() -> Strings {
        let mut s = Strings::new();
        s.add(
            "en",
            &table(&[("menu.play", "Play"), ("menu.day", "Day {day} of {total}")]),
        );
        s.add("fr", &table(&[("menu.play", "Jouer")]));
        s
    }

    #[test]
    fn lookup_falls_back_to_english_then_to_the_visible_key() {
        let s = sample();
        assert_eq!(s.text("en", "menu.play", &[]), "Play");
        assert_eq!(s.text("fr", "menu.play", &[]), "Jouer");
        assert_eq!(
            s.text("fr", "menu.day", &[("day", "3"), ("total", "9")]),
            "Day 3 of 9"
        );
        assert_eq!(s.text("de", "menu.play", &[]), "Play");
        assert_eq!(s.text("en", "no.such.key", &[]), "[no.such.key]");
        assert_eq!(
            s.text("en", "menu.day", &[("day", "3")]),
            "Day 3 of {total}",
            "a missing parameter stays visible"
        );
    }

    #[test]
    fn the_pseudo_locale_accents_pads_and_keeps_placeholders_intact() {
        let s = sample();
        let p = s.text("pseudo", "menu.day", &[("day", "3"), ("total", "9")]);
        assert!(p.starts_with('[') && p.ends_with(']'), "{p}");
        assert!(
            p.contains('3') && p.contains('9'),
            "placeholders still fill: {p}"
        );
        assert!(p.contains('í') || p.contains('á'), "{p}");
        let plain = "Day {day} of {total}";
        assert!(pseudo(plain).chars().count() > plain.chars().count() * 13 / 10);
        assert_eq!(placeholders(&pseudo(plain)), placeholders(plain));
        assert_eq!(s.text("pseudo", "nope", &[]), "[nope]");
    }

    #[test]
    fn placeholders_are_found_and_odd_braces_are_ignored() {
        assert_eq!(
            placeholders("a {x} b {y_1} {} { } {x"),
            ["x", "y_1"].iter().map(|s| (*s).to_owned()).collect()
        );
        assert!(placeholders("no braces").is_empty());
    }

    #[test]
    fn lint_reports_missing_orphaned_mismatched_and_unused_keys() {
        let mut s = sample();
        s.add(
            "de",
            &table(&[
                ("menu.play", "Spielen"),
                ("menu.day", "Tag {tag}"),
                ("menu.extra", "x"),
            ]),
        );
        let used: BTreeSet<String> = ["menu.play".to_owned()].into_iter().collect();
        let l = s.lint(Some(&used));
        assert_eq!(l.missing, [("fr".to_owned(), "menu.day".to_owned())]);
        assert_eq!(l.orphaned, [("de".to_owned(), "menu.extra".to_owned())]);
        assert_eq!(
            l.placeholder_mismatch,
            [("de".to_owned(), "menu.day".to_owned())]
        );
        assert_eq!(l.unused, ["menu.day"]);
        assert!(!l.is_clean());
        assert!(Strings::new().lint(None).is_clean());
    }

    #[test]
    fn the_first_definition_wins_and_duplicates_are_reported() {
        let mut s = Strings::new();
        assert!(s.add("en", &table(&[("a", "one")])).is_empty());
        assert_eq!(s.add("en", &table(&[("a", "two"), ("b", "x")])), ["a"]);
        assert_eq!(s.get("en", "a"), Some("one"));
        assert_eq!(s.get("en", "b"), Some("x"));
        assert_eq!((s.len("en"), s.locales()), (2, vec!["en"]));
    }

    fn parse_file(
        path: &str,
        json: &str,
        pack: &str,
        base: bool,
    ) -> (Option<(String, Table)>, ValidationReport) {
        let mut r = ValidationReport::new();
        let out = parse_strings_file(path, &parse(json).unwrap(), pack, base, &mut r);
        (out, r)
    }

    #[test]
    fn files_are_validated_with_namespaces_and_limits() {
        let (ok, r) = parse_file(
            "data/strings/en.json",
            r#"{"menu.play":"Play"}"#,
            "base",
            true,
        );
        assert!(r.is_ok() && ok.unwrap().0 == "en");
        // A pack must stay under its own id.
        let (bad, r) = parse_file(
            "data/strings/en.json",
            r#"{"menu.play":"x"}"#,
            "coffee",
            false,
        );
        assert!(bad.is_none() && r.has_code("string_namespace"));
        let (good, _) = parse_file(
            "data/strings/pt-BR.json",
            r#"{"coffee.order":"Pedir"}"#,
            "coffee",
            false,
        );
        assert_eq!(good.unwrap().0, "pt-BR");
        for (path, json, code) in [
            ("data/strings/pseudo.json", "{}", "bad_locale"),
            ("data/strings/en/x.json", "{}", "bad_locale"),
            ("data/strings/E N.json", "{}", "bad_locale"),
            ("data/strings/en.json", "[]", "type_mismatch"),
            (
                "data/strings/en.json",
                r#"{"Bad Key":"x"}"#,
                "bad_string_key",
            ),
            ("data/strings/en.json", r#"{"a..b":"x"}"#, "bad_string_key"),
            ("data/strings/en.json", r#"{"a":5}"#, "type_mismatch"),
            (
                "data/strings/en.json",
                r#"{"a":"bell\u0007"}"#,
                "bad_string_text",
            ),
        ] {
            let (out, r) = parse_file(path, json, "base", true);
            assert!(out.is_none() && r.has_code(code), "{path} {json}: {r}");
        }
        let long = format!(r#"{{"a":"{}"}}"#, "x".repeat(2_001));
        assert!(parse_file("data/strings/en.json", &long, "base", true)
            .0
            .is_none());
    }
}
