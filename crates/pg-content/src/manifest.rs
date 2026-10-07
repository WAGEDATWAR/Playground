//! Pack manifests (Blueprint §23.3): `pack.json` describes a content pack and what it needs.
//!
//! Parsing is strict. Versions are `major[.minor[.patch]]`; ranges are space-separated comparators
//! such as `>=0.2 <0.4`. A pack that declares scripts must name an `entry` script, and capabilities
//! outside the known set are errors.

use crate::ids::PackId;
use crate::report::ValidationReport;
use pg_canon::Canon;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// The engine and modding-API versions this build provides (checked against pack requirements).
pub const ENGINE_VERSION: Version = Version::new(0, 0, 1);
pub const API_VERSION: Version = Version::new(0, 1, 0);

const MANIFEST_KEYS: &[&str] = &[
    "id",
    "name",
    "version",
    "api",
    "engine",
    "depends",
    "capabilities",
    "entry",
    "settings",
];
const MAX_NAME_LEN: usize = 64;
const MAX_DEPENDS: usize = 32;
const MAX_SETTINGS: usize = 32;

/// `major.minor.patch`; a missing component parses as 0.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A version or range string could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionError(pub String);

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VersionError {}

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Version, VersionError> {
        let bad = || {
            VersionError(format!(
                "'{s}' is not a version (expected major[.minor[.patch]], digits only)"
            ))
        };
        let parts: Vec<&str> = s.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(bad());
        }
        let mut nums = [0u32; 3];
        for (slot, part) in nums.iter_mut().zip(&parts) {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) || part.len() > 9 {
                return Err(bad());
            }
            *slot = part.parse().map_err(|_| bad())?;
        }
        Ok(Version::new(nums[0], nums[1], nums[2]))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
}

/// A conjunction of comparators. `*` (or empty) matches everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionReq {
    comparators: Vec<(Op, Version)>,
    text: String,
}

impl VersionReq {
    pub fn any() -> VersionReq {
        VersionReq {
            comparators: Vec::new(),
            text: "*".to_owned(),
        }
    }

    pub fn matches(&self, v: &Version) -> bool {
        self.comparators.iter().all(|(op, bound)| match op {
            Op::Ge => v >= bound,
            Op::Gt => v > bound,
            Op::Le => v <= bound,
            Op::Lt => v < bound,
            Op::Eq => v == bound,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl FromStr for VersionReq {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<VersionReq, VersionError> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed == "*" {
            return Ok(VersionReq::any());
        }
        let mut comparators = Vec::new();
        for token in trimmed.split_whitespace() {
            let (op, rest) = if let Some(r) = token.strip_prefix(">=") {
                (Op::Ge, r)
            } else if let Some(r) = token.strip_prefix("<=") {
                (Op::Le, r)
            } else if let Some(r) = token.strip_prefix('>') {
                (Op::Gt, r)
            } else if let Some(r) = token.strip_prefix('<') {
                (Op::Lt, r)
            } else if let Some(r) = token.strip_prefix('=') {
                (Op::Eq, r)
            } else {
                (Op::Eq, token)
            };
            let v = rest
                .parse::<Version>()
                .map_err(|e| VersionError(format!("in range '{s}': {e}")))?;
            comparators.push((op, v));
        }
        Ok(VersionReq {
            comparators,
            text: trimmed.to_owned(),
        })
    }
}

/// What a pack asks permission to do (Blueprint §23.2).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    Read,
    Data,
    Systems,
    WorldWrite,
    Worldgen,
    Ai,
    Dev,
}

impl Capability {
    pub const ALL: [Capability; 7] = [
        Capability::Read,
        Capability::Data,
        Capability::Systems,
        Capability::WorldWrite,
        Capability::Worldgen,
        Capability::Ai,
        Capability::Dev,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Capability::Read => "read",
            Capability::Data => "data",
            Capability::Systems => "systems",
            Capability::WorldWrite => "world-write",
            Capability::Worldgen => "worldgen",
            Capability::Ai => "ai",
            Capability::Dev => "dev",
        }
    }

    pub fn from_name(name: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.name() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub id: PackId,
    pub version: VersionReq,
}

/// A per-world setting a pack declares (shown at world creation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackSetting {
    Int {
        id: String,
        min: i64,
        max: i64,
        default: i64,
    },
    Bool {
        id: String,
        default: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub id: PackId,
    pub name: String,
    pub version: Version,
    pub api: VersionReq,
    pub engine: VersionReq,
    pub depends: Vec<Dependency>,
    pub capabilities: BTreeSet<Capability>,
    pub entry: Option<String>,
    pub settings: Vec<PackSetting>,
}

/// Whether `path` is a safe pack-relative path: `/`-separated ASCII segments, no `..`, no absolute or
/// drive-letter forms, no backslashes.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && path.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

impl PackManifest {
    /// Parses and validates a manifest. `path` names the file in reports.
    pub fn from_canon(
        value: &Canon,
        path: &str,
        report: &mut ValidationReport,
    ) -> Option<PackManifest> {
        let Canon::Map(map) = value else {
            report.error("type_mismatch", path, "the manifest must be an object");
            return None;
        };
        let before = report.error_count();
        for key in map.keys() {
            if !MANIFEST_KEYS.contains(&key.as_str()) {
                let hint = crate::hints::hint(key, MANIFEST_KEYS.iter().copied());
                report.error(
                    "unknown_field",
                    format!("{path}.{key}"),
                    format!(
                        "unknown manifest field{hint} (allowed: {})",
                        MANIFEST_KEYS.join(", ")
                    ),
                );
            }
        }
        let p = |k: &str| format!("{path}.{k}");

        let id = match map.get("id").and_then(Canon::as_str).map(PackId::new) {
            Some(Ok(id)) => Some(id),
            Some(Err(e)) => {
                report.error("bad_id", p("id"), e.to_string());
                None
            }
            None => {
                report.error("missing_field", p("id"), "a manifest needs a text 'id'");
                None
            }
        };

        let name = match map.get("name").and_then(Canon::as_str) {
            Some(n) if n.trim().is_empty() => {
                report.error("bad_text", p("name"), "name may not be empty");
                String::new()
            }
            Some(n) if n.chars().count() > MAX_NAME_LEN || n.chars().any(char::is_control) => {
                report.error(
                    "bad_text",
                    p("name"),
                    format!(
                        "name must be at most {MAX_NAME_LEN} characters with no control characters"
                    ),
                );
                String::new()
            }
            Some(n) => n.to_owned(),
            None => {
                report.error("missing_field", p("name"), "a manifest needs a text 'name'");
                String::new()
            }
        };

        let version = match map
            .get("version")
            .and_then(Canon::as_str)
            .map(str::parse::<Version>)
        {
            Some(Ok(v)) => v,
            Some(Err(e)) => {
                report.error("bad_version", p("version"), e.to_string());
                Version::new(0, 0, 0)
            }
            None => {
                report.error(
                    "missing_field",
                    p("version"),
                    "a manifest needs a text 'version'",
                );
                Version::new(0, 0, 0)
            }
        };

        let mut req = |key: &str| match map.get(key) {
            None => VersionReq::any(),
            Some(v) => match v.as_str().map(str::parse::<VersionReq>) {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    report.error("bad_version", p(key), e.to_string());
                    VersionReq::any()
                }
                None => {
                    report.error("type_mismatch", p(key), "a version range must be text");
                    VersionReq::any()
                }
            },
        };
        let api = req("api");
        let engine = req("engine");

        let mut depends = Vec::new();
        match map.get("depends") {
            None => {}
            Some(Canon::List(items)) => {
                if items.len() > MAX_DEPENDS {
                    report.error(
                        "too_long",
                        p("depends"),
                        format!("more than {MAX_DEPENDS} dependencies"),
                    );
                }
                let mut seen = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    let ipath = format!("{}[{i}]", p("depends"));
                    let dep_id = item.get("id").and_then(Canon::as_str).map(PackId::new);
                    let dep_ver = match item.get("version") {
                        None => Some(Ok(VersionReq::any())),
                        Some(v) => v.as_str().map(str::parse::<VersionReq>),
                    };
                    match (dep_id, dep_ver) {
                        (Some(Ok(did)), Some(Ok(dv))) => {
                            if !seen.insert(did.clone()) {
                                report.error(
                                    "duplicate_dependency",
                                    ipath,
                                    format!("'{did}' is listed twice"),
                                );
                            } else if Some(&did) == id.as_ref() {
                                report.error(
                                    "self_dependency",
                                    ipath,
                                    "a pack cannot depend on itself",
                                );
                            } else {
                                depends.push(Dependency {
                                    id: did,
                                    version: dv,
                                });
                            }
                        }
                        (Some(Err(e)), _) => report.error("bad_id", ipath, e.to_string()),
                        (_, Some(Err(e))) => report.error("bad_version", ipath, e.to_string()),
                        _ => report.error(
                            "type_mismatch",
                            ipath,
                            "a dependency needs a text 'id' and an optional text 'version'",
                        ),
                    }
                }
            }
            Some(_) => report.error("type_mismatch", p("depends"), "depends must be a list"),
        }

        let mut capabilities = BTreeSet::new();
        match map.get("capabilities") {
            None => {}
            Some(Canon::List(items)) => {
                for (i, item) in items.iter().enumerate() {
                    match item.as_str().map(|s| (s, Capability::from_name(s))) {
                        Some((_, Some(c))) => {
                            capabilities.insert(c);
                        }
                        Some((s, None)) => report.error(
                            "unknown_capability",
                            format!("{}[{i}]", p("capabilities")),
                            format!(
                                "'{s}' is not a capability{} (known: {})",
                                crate::hints::hint(s, Capability::ALL.iter().map(|c| c.name())),
                                Capability::ALL
                                    .iter()
                                    .map(|c| c.name())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ),
                        None => report.error(
                            "type_mismatch",
                            format!("{}[{i}]", p("capabilities")),
                            "capabilities must be text",
                        ),
                    }
                }
            }
            Some(_) => report.error(
                "type_mismatch",
                p("capabilities"),
                "capabilities must be a list",
            ),
        }

        let entry = match map.get("entry") {
            None | Some(Canon::Null) => None,
            Some(v) => match v.as_str() {
                Some(e) if is_safe_relative_path(e) && e.ends_with(".luau") => Some(e.to_owned()),
                Some(e) => {
                    report.error(
                        "bad_path",
                        p("entry"),
                        format!("'{e}' must be a safe relative path ending in .luau"),
                    );
                    None
                }
                None => {
                    report.error("type_mismatch", p("entry"), "entry must be a path");
                    None
                }
            },
        };

        let mut settings = Vec::new();
        match map.get("settings") {
            None => {}
            Some(Canon::List(items)) => {
                if items.len() > MAX_SETTINGS {
                    report.error(
                        "too_long",
                        p("settings"),
                        format!("more than {MAX_SETTINGS} settings"),
                    );
                }
                let mut seen = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    if let Some(s) = parse_setting(item, &format!("{}[{i}]", p("settings")), report)
                    {
                        let sid = match &s {
                            PackSetting::Int { id, .. } | PackSetting::Bool { id, .. } => {
                                id.clone()
                            }
                        };
                        if seen.insert(sid.clone()) {
                            settings.push(s);
                        } else {
                            report.error(
                                "duplicate_setting",
                                format!("{}[{i}]", p("settings")),
                                format!("setting '{sid}' is declared twice"),
                            );
                        }
                    }
                }
            }
            Some(_) => report.error("type_mismatch", p("settings"), "settings must be a list"),
        }

        // Consistency between scripts and capabilities.
        let scripting = capabilities
            .iter()
            .any(|c| *c != Capability::Read && *c != Capability::Data);
        if entry.is_some() && capabilities.is_empty() {
            report.warn(
                "script_without_capabilities",
                path,
                "the pack has a script entry but declares no capabilities",
            );
        }
        if entry.is_none() && scripting {
            report.warn(
                "capabilities_without_script",
                path,
                "capabilities beyond read/data are declared but there is no script entry",
            );
        }

        if report.error_count() > before {
            return None;
        }
        Some(PackManifest {
            id: id?,
            name,
            version,
            api,
            engine,
            depends,
            capabilities,
            entry,
            settings,
        })
    }
}

fn parse_setting(item: &Canon, path: &str, report: &mut ValidationReport) -> Option<PackSetting> {
    let before = report.error_count();
    let Canon::Map(m) = item else {
        report.error("type_mismatch", path, "a setting must be an object");
        return None;
    };
    for key in m.keys() {
        if !["id", "type", "min", "max", "default"].contains(&key.as_str()) {
            let hint = crate::hints::hint(key, ["id", "type", "min", "max", "default"]);
            report.error(
                "unknown_field",
                format!("{path}.{key}"),
                format!("unknown setting field{hint}"),
            );
        }
    }
    let id = match m.get("id").and_then(Canon::as_str) {
        Some(s) if PackId::new(s).is_ok() => s.to_owned(),
        _ => {
            report.error(
                "bad_id",
                format!("{path}.id"),
                "a setting needs an id of lowercase letters, digits and _",
            );
            String::new()
        }
    };
    let result = match m.get("type").and_then(Canon::as_str) {
        Some("int") => {
            let get = |k: &str| m.get(k).and_then(Canon::as_i64);
            match (get("min"), get("max"), get("default")) {
                (Some(min), Some(max), Some(default))
                    if min <= max && (min..=max).contains(&default) =>
                {
                    Some(PackSetting::Int {
                        id,
                        min,
                        max,
                        default,
                    })
                }
                (Some(_), Some(_), Some(_)) => {
                    report.error(
                        "out_of_range",
                        path,
                        "an int setting needs min <= default <= max",
                    );
                    None
                }
                _ => {
                    report.error(
                        "missing_field",
                        path,
                        "an int setting needs integer min, max and default",
                    );
                    None
                }
            }
        }
        Some("bool") => match m.get("default").and_then(Canon::as_bool) {
            Some(default) => Some(PackSetting::Bool { id, default }),
            None => {
                report.error(
                    "missing_field",
                    path,
                    "a bool setting needs a boolean default",
                );
                None
            }
        },
        _ => {
            report.error(
                "bad_enum",
                format!("{path}.type"),
                "setting type must be \"int\" or \"bool\"",
            );
            None
        }
    };
    if report.error_count() > before {
        None
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json::parse;

    fn load(json: &str) -> (Option<PackManifest>, ValidationReport) {
        let mut r = ValidationReport::new();
        let m = PackManifest::from_canon(&parse(json).unwrap(), "pack.json", &mut r);
        (m, r)
    }

    #[test]
    fn versions_parse_and_order() {
        assert_eq!("1".parse::<Version>().unwrap(), Version::new(1, 0, 0));
        assert_eq!("1.2".parse::<Version>().unwrap(), Version::new(1, 2, 0));
        assert_eq!("0.10.3".parse::<Version>().unwrap(), Version::new(0, 10, 3));
        assert!(
            Version::new(0, 10, 0) > Version::new(0, 9, 9),
            "numeric, not lexicographic"
        );
        for bad in [
            "",
            "a",
            "1.",
            ".1",
            "1.2.3.4",
            "1.x",
            "-1",
            "1 .2",
            "99999999999",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?}");
        }
        assert_eq!(Version::new(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn ranges_match_as_expected() {
        let r: VersionReq = ">=0.2 <0.4".parse().unwrap();
        assert!(r.matches(&Version::new(0, 2, 0)) && r.matches(&Version::new(0, 3, 9)));
        assert!(!r.matches(&Version::new(0, 1, 9)) && !r.matches(&Version::new(0, 4, 0)));
        assert!("*"
            .parse::<VersionReq>()
            .unwrap()
            .matches(&Version::new(9, 9, 9)));
        assert!(""
            .parse::<VersionReq>()
            .unwrap()
            .matches(&Version::new(0, 0, 0)));
        let exact: VersionReq = "1.2.3".parse().unwrap();
        assert!(exact.matches(&Version::new(1, 2, 3)) && !exact.matches(&Version::new(1, 2, 4)));
        assert!(">1 <=2"
            .parse::<VersionReq>()
            .unwrap()
            .matches(&Version::new(2, 0, 0)));
        assert!(!">1 <=2"
            .parse::<VersionReq>()
            .unwrap()
            .matches(&Version::new(1, 0, 0)));
        assert!(">=x".parse::<VersionReq>().is_err());
        assert!(">=".parse::<VersionReq>().is_err());
    }

    #[test]
    fn a_full_manifest_loads() {
        let (m, r) = load(
            r#"{"id":"coffee_shop","name":"Coffee Shop","version":"0.3.0","api":">=0.1 <0.4","engine":">=0.0.1",
                "depends":[{"id":"base","version":">=0.1"}],"capabilities":["data","systems"],
                "entry":"scripts/main.luau",
                "settings":[{"id":"strength","type":"int","min":0,"max":100,"default":50},{"id":"fancy","type":"bool","default":true}]}"#,
        );
        assert!(r.is_ok(), "{r}");
        let m = m.unwrap();
        assert_eq!(m.id.as_str(), "coffee_shop");
        assert_eq!(m.version, Version::new(0, 3, 0));
        assert_eq!(m.depends.len(), 1);
        assert!(m.capabilities.contains(&Capability::Systems));
        assert_eq!(m.entry.as_deref(), Some("scripts/main.luau"));
        assert_eq!(m.settings.len(), 2);
    }

    #[test]
    fn a_data_only_manifest_needs_only_three_fields() {
        let (m, r) = load(r#"{"id":"base","name":"Base","version":"0.1.0"}"#);
        assert!(r.is_ok() && r.is_empty(), "{r}");
        let m = m.unwrap();
        assert!(m.entry.is_none() && m.capabilities.is_empty() && m.api == VersionReq::any());
    }

    #[test]
    fn mistakes_are_reported_with_codes() {
        let cases: [(&str, &str); 19] = [
            (r#"[]"#, "type_mismatch"),
            (r#"{"name":"x","version":"1"}"#, "missing_field"),
            (r#"{"id":"x","version":"1"}"#, "missing_field"),
            (r#"{"id":"x","name":"x"}"#, "missing_field"),
            (r#"{"id":"Bad","name":"x","version":"1"}"#, "bad_id"),
            (r#"{"id":"x","name":" ","version":"1"}"#, "bad_text"),
            (r#"{"id":"x","name":"x","version":"one"}"#, "bad_version"),
            (
                r#"{"id":"x","name":"x","version":"1","api":">=q"}"#,
                "bad_version",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","api":5}"#,
                "type_mismatch",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","extra":1}"#,
                "unknown_field",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","capabilities":["fly"]}"#,
                "unknown_capability",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","capabilities":"data"}"#,
                "type_mismatch",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","entry":"../evil.luau"}"#,
                "bad_path",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","entry":"main.lua"}"#,
                "bad_path",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","depends":[{"id":"x"}]}"#,
                "self_dependency",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","depends":[{"id":"y"},{"id":"y"}]}"#,
                "duplicate_dependency",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","depends":[{"version":"1"}]}"#,
                "type_mismatch",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","settings":[{"id":"a","type":"int","min":5,"max":1,"default":2}]}"#,
                "out_of_range",
            ),
            (
                r#"{"id":"x","name":"x","version":"1","settings":[{"id":"a","type":"float","default":1}]}"#,
                "bad_enum",
            ),
        ];
        for (json, code) in cases {
            let (m, r) = load(json);
            assert!(m.is_none(), "{json} should not load");
            assert!(r.has_code(code), "{json}: wanted {code}, got:\n{r}");
        }
    }

    #[test]
    fn script_and_capability_consistency_warns() {
        let (m, r) = load(r#"{"id":"x","name":"x","version":"1","entry":"main.luau"}"#);
        assert!(m.is_some() && r.has_code("script_without_capabilities"));
        let (m, r) = load(r#"{"id":"x","name":"x","version":"1","capabilities":["systems"]}"#);
        assert!(m.is_some() && r.has_code("capabilities_without_script"));
    }

    #[test]
    fn safe_paths() {
        for ok in [
            "pack.json",
            "data/templates/a.json",
            "scripts/main.luau",
            "assets/sprite-1_a.png",
        ] {
            assert!(is_safe_relative_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/abs",
            "a//b",
            "../x",
            "a/../b",
            "a/./b",
            "a\\b",
            "C:/x",
            "a b",
            "é",
            "a/",
            "./a",
            &"a/".repeat(150),
        ] {
            assert!(!is_safe_relative_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn capability_names_round_trip() {
        for c in Capability::ALL {
            assert_eq!(Capability::from_name(c.name()), Some(c));
        }
        assert_eq!(Capability::from_name("world_write"), None);
    }
}
