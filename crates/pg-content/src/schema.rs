//! Declarative parameter schemas for components (Blueprint §4.3, §23.10).
//!
//! A schema describes bounded integers, booleans, text, enums, ids, lists and nested objects. The
//! same description drives validation, default filling, the inspector and (later) pack-defined
//! components, so none of those needs per-component code. Every value is checked: unknown fields,
//! missing required fields, out-of-range numbers and over-long lists are all errors.

use crate::ids::{Tag, TemplateId};
use crate::report::ValidationReport;
use pg_canon::Canon;
use std::collections::BTreeMap;

/// The shape of one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldSchema {
    /// An integer in `min..=max`.
    Int {
        min: i64,
        max: i64,
    },
    Bool,
    /// Text of at most `max_len` characters, with no control characters.
    Text {
        max_len: usize,
    },
    /// One of the listed strings.
    Enum(Vec<String>),
    /// A valid [`Tag`].
    Tag,
    /// A valid [`TemplateId`].
    TemplateRef,
    /// A tile coordinate `[x, y]` of 32-bit integers.
    Tile,
    /// An entity id of the given kind prefix, for example `pawn_1a` (`kind` is `"pawn"`).
    EntityId {
        kind: String,
    },
    /// `null` (or absent) or a value of the inner shape.
    Optional(Box<FieldSchema>),
    List {
        item: Box<FieldSchema>,
        max_len: usize,
    },
    Object(ParamSchema),
}

/// A field: its shape and, if optional, the value used when it is absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub schema: FieldSchema,
    pub default: Option<Canon>,
}

impl Field {
    pub fn required(schema: FieldSchema) -> Field {
        Field {
            schema,
            default: None,
        }
    }

    pub fn optional(schema: FieldSchema, default: Canon) -> Field {
        Field {
            schema,
            default: Some(default),
        }
    }

    pub fn int(min: i64, max: i64, default: i64) -> Field {
        Field::optional(
            FieldSchema::Int { min, max },
            Canon::Int(i128::from(default)),
        )
    }

    pub fn required_int(min: i64, max: i64) -> Field {
        Field::required(FieldSchema::Int { min, max })
    }

    pub fn boolean(default: bool) -> Field {
        Field::optional(FieldSchema::Bool, Canon::Bool(default))
    }

    pub fn text(max_len: usize, default: &str) -> Field {
        Field::optional(FieldSchema::Text { max_len }, Canon::str(default))
    }

    pub fn required_text(max_len: usize) -> Field {
        Field::required(FieldSchema::Text { max_len })
    }

    pub fn enumeration(values: &[&str], default: &str) -> Field {
        Field::optional(
            FieldSchema::Enum(values.iter().map(|s| (*s).to_owned()).collect()),
            Canon::str(default),
        )
    }

    pub fn required_enum(values: &[&str]) -> Field {
        Field::required(FieldSchema::Enum(
            values.iter().map(|s| (*s).to_owned()).collect(),
        ))
    }

    /// An optional field that is `null` when absent.
    pub fn maybe(schema: FieldSchema) -> Field {
        Field::optional(FieldSchema::Optional(Box::new(schema)), Canon::Null)
    }

    /// An optional list that defaults to empty.
    pub fn list(item: FieldSchema, max_len: usize) -> Field {
        Field::optional(
            FieldSchema::List {
                item: Box::new(item),
                max_len,
            },
            Canon::List(Vec::new()),
        )
    }

    /// An optional object whose own defaults fill it in when absent.
    pub fn object(schema: ParamSchema) -> Field {
        Field::optional(FieldSchema::Object(schema), Canon::Map(BTreeMap::new()))
    }
}

/// An object schema: a set of named fields. Unknown fields are always rejected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamSchema {
    pub fields: BTreeMap<String, Field>,
}

impl ParamSchema {
    pub fn new() -> ParamSchema {
        ParamSchema::default()
    }

    pub fn field(mut self, name: &str, field: Field) -> ParamSchema {
        self.fields.insert(name.to_owned(), field);
        self
    }

    /// Validates `value` against this schema, recording problems in `report` under `path`.
    /// Returns the normalized value (defaults filled in). When errors were recorded the returned
    /// value is best-effort and must not be used.
    pub fn check(&self, value: &Canon, path: &str, report: &mut ValidationReport) -> Canon {
        let Canon::Map(map) = value else {
            report.error(
                "type_mismatch",
                path,
                format!("expected an object, found {}", kind_name(value)),
            );
            return Canon::Map(BTreeMap::new());
        };
        let mut out = BTreeMap::new();
        for key in map.keys() {
            if !self.fields.contains_key(key) {
                let known = self.fields.keys().cloned().collect::<Vec<_>>().join(", ");
                let hint = crate::hints::hint(key, self.fields.keys().map(String::as_str));
                report.error(
                    "unknown_field",
                    join(path, key),
                    format!("unknown field{hint} (known fields: {known})"),
                );
            }
        }
        for (name, field) in &self.fields {
            let field_path = join(path, name);
            match map.get(name) {
                Some(v) => {
                    let normalized = field.schema.check(v, &field_path, report);
                    out.insert(name.clone(), normalized);
                }
                None => match &field.default {
                    Some(default) => {
                        // Defaults may themselves be partial objects: normalize them too.
                        let mut scratch = ValidationReport::new();
                        let normalized = field.schema.check(default, &field_path, &mut scratch);
                        out.insert(
                            name.clone(),
                            if scratch.is_ok() {
                                normalized
                            } else {
                                default.clone()
                            },
                        );
                    }
                    None => report.error("missing_field", field_path, "required field is missing"),
                },
            }
        }
        Canon::Map(out)
    }
}

impl FieldSchema {
    /// Validates one value against this shape (used by the settings registry for single values).
    pub fn check(&self, value: &Canon, path: &str, report: &mut ValidationReport) -> Canon {
        match self {
            FieldSchema::Int { min, max } => match value.as_i128() {
                Some(v) if v >= i128::from(*min) && v <= i128::from(*max) => value.clone(),
                Some(v) => {
                    report.error(
                        "out_of_range",
                        path,
                        format!("{v} is outside {min}..={max}"),
                    );
                    value.clone()
                }
                None => mismatch(report, path, "an integer", value),
            },
            FieldSchema::Bool => match value {
                Canon::Bool(_) => value.clone(),
                _ => mismatch(report, path, "true or false", value),
            },
            FieldSchema::Text { max_len } => match value.as_str() {
                Some(s) if s.chars().count() > *max_len => {
                    report.error(
                        "too_long",
                        path,
                        format!("text is longer than {max_len} characters"),
                    );
                    value.clone()
                }
                Some(s) if s.chars().any(char::is_control) => {
                    report.error("bad_text", path, "text may not contain control characters");
                    value.clone()
                }
                Some(_) => value.clone(),
                None => mismatch(report, path, "text", value),
            },
            FieldSchema::Enum(allowed) => match value.as_str() {
                Some(s) if allowed.iter().any(|a| a == s) => value.clone(),
                Some(s) => {
                    let hint = crate::hints::hint(s, allowed.iter().map(String::as_str));
                    report.error(
                        "bad_enum",
                        path,
                        format!("'{s}' is not one of: {}{hint}", allowed.join(", ")),
                    );
                    value.clone()
                }
                None => mismatch(report, path, "text", value),
            },
            FieldSchema::Tag => match value.as_str() {
                Some(s) => {
                    if let Err(e) = Tag::new(s) {
                        report.error("bad_id", path, e.to_string());
                    }
                    value.clone()
                }
                None => mismatch(report, path, "a tag", value),
            },
            FieldSchema::TemplateRef => match value.as_str() {
                Some(s) => {
                    if let Err(e) = TemplateId::new(s) {
                        report.error("bad_id", path, e.to_string());
                    }
                    value.clone()
                }
                None => mismatch(report, path, "a template id", value),
            },
            FieldSchema::Tile => match value.as_list() {
                Some([x, y])
                    if [x, y]
                        .iter()
                        .all(|v| v.as_i64().is_some_and(|n| i32::try_from(n).is_ok())) =>
                {
                    value.clone()
                }
                Some(_) => {
                    report.error(
                        "bad_tile",
                        path,
                        "a tile is [x, y] with two 32-bit integers",
                    );
                    value.clone()
                }
                None => mismatch(report, path, "a tile [x, y]", value),
            },
            FieldSchema::EntityId { kind } => match value.as_str() {
                Some(s) if is_entity_id(s, kind) => value.clone(),
                Some(s) => {
                    report.error(
                        "bad_id",
                        path,
                        format!("'{s}' is not a {kind} id (like {kind}_1a)"),
                    );
                    value.clone()
                }
                None => mismatch(report, path, "an entity id", value),
            },
            FieldSchema::Optional(inner) => match value {
                Canon::Null => Canon::Null,
                other => inner.check(other, path, report),
            },
            FieldSchema::List { item, max_len } => match value.as_list() {
                Some(items) => {
                    if items.len() > *max_len {
                        report.error(
                            "too_long",
                            path,
                            format!("list has {} items; the limit is {max_len}", items.len()),
                        );
                    }
                    Canon::List(
                        items
                            .iter()
                            .enumerate()
                            .map(|(i, v)| item.check(v, &format!("{path}[{i}]"), report))
                            .collect(),
                    )
                }
                None => mismatch(report, path, "a list", value),
            },
            FieldSchema::Object(schema) => schema.check(value, path, report),
        }
    }
}

/// Whether `s` is `<kind>_<lowercase base36>` (the canonical form of an entity id).
fn is_entity_id(s: &str, kind: &str) -> bool {
    s.strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix('_'))
        .is_some_and(|n| {
            !n.is_empty()
                && n.bytes()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
        })
}

fn kind_name(v: &Canon) -> &'static str {
    match v {
        Canon::Null => "null",
        Canon::Bool(_) => "a boolean",
        Canon::Int(_) => "an integer",
        Canon::Str(_) => "text",
        Canon::List(_) => "a list",
        Canon::Map(_) => "an object",
    }
}

fn mismatch(report: &mut ValidationReport, path: &str, expected: &str, found: &Canon) -> Canon {
    report.error(
        "type_mismatch",
        path,
        format!("expected {expected}, found {}", kind_name(found)),
    );
    found.clone()
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json::parse;

    fn schema() -> ParamSchema {
        ParamSchema::new()
            .field("w", Field::int(1, 64, 1))
            .field("name", Field::required_text(8))
            .field("solid", Field::boolean(false))
            .field("kind", Field::enumeration(&["a", "b"], "a"))
            .field("tags", Field::list(FieldSchema::Tag, 3))
            .field(
                "inner",
                Field::object(
                    ParamSchema::new()
                        .field("n", Field::int(0, 10, 5))
                        .field("t", Field::text(4, "x")),
                ),
            )
    }

    fn check(json: &str) -> (Canon, ValidationReport) {
        let mut r = ValidationReport::new();
        let out = schema().check(&parse(json).unwrap(), "root", &mut r);
        (out, r)
    }

    #[test]
    fn defaults_are_filled_including_nested_ones() {
        let (out, r) = check(r#"{"name":"bob"}"#);
        assert!(r.is_ok(), "{r}");
        assert_eq!(
            out.to_canonical_string(),
            r#"{"inner":{"n":5,"t":"x"},"kind":"a","name":"bob","solid":false,"tags":[],"w":1}"#
        );
    }

    #[test]
    fn explicit_values_win_and_nested_partials_normalize() {
        let (out, r) = check(
            r#"{"name":"bob","w":9,"solid":true,"kind":"b","tags":["x","y"],"inner":{"n":2}}"#,
        );
        assert!(r.is_ok(), "{r}");
        assert_eq!(
            out.to_canonical_string(),
            r#"{"inner":{"n":2,"t":"x"},"kind":"b","name":"bob","solid":true,"tags":["x","y"],"w":9}"#
        );
    }

    #[test]
    fn every_kind_of_mistake_is_reported_with_a_path_and_code() {
        let cases: [(&str, &str, &str); 11] = [
            (r#"{}"#, "missing_field", "root.name"),
            (r#"{"name":"bob","extra":1}"#, "unknown_field", "root.extra"),
            (r#"{"name":"bob","w":0}"#, "out_of_range", "root.w"),
            (r#"{"name":"bob","w":65}"#, "out_of_range", "root.w"),
            (r#"{"name":"bob","w":"x"}"#, "type_mismatch", "root.w"),
            (r#"{"name":"much too long"}"#, "too_long", "root.name"),
            (r#"{"name":"bob","kind":"z"}"#, "bad_enum", "root.kind"),
            (
                r#"{"name":"bob","tags":["a","b","c","d"]}"#,
                "too_long",
                "root.tags",
            ),
            (
                r#"{"name":"bob","tags":["Bad Tag"]}"#,
                "bad_id",
                "root.tags[0]",
            ),
            (
                r#"{"name":"bob","inner":{"n":11}}"#,
                "out_of_range",
                "root.inner.n",
            ),
            (r#"{"name":"a\nb"}"#, "bad_text", "root.name"),
        ];
        for (json, code, path) in cases {
            let (_, r) = check(json);
            assert!(
                r.errors().any(|i| i.code == code && i.path == path),
                "{json}: wanted {code} at {path}, got:\n{r}"
            );
        }
    }

    #[test]
    fn tile_entity_id_and_optional_fields() {
        let schema = ParamSchema::new()
            .field("at", Field::required(FieldSchema::Tile))
            .field(
                "who",
                Field::maybe(FieldSchema::EntityId {
                    kind: "pawn".into(),
                }),
            )
            .field("spot", Field::maybe(FieldSchema::Tile));
        let run = |json: &str| {
            let mut r = ValidationReport::new();
            let out = schema.check(&parse(json).unwrap(), "p", &mut r);
            (out, r)
        };
        let (out, r) = run(r#"{"at":[3,-4]}"#);
        assert!(r.is_ok(), "{r}");
        assert_eq!(
            out.to_canonical_string(),
            r#"{"at":[3,-4],"spot":null,"who":null}"#
        );
        let (_, r) = run(r#"{"at":[1,2],"who":"pawn_1a","spot":[0,0]}"#);
        assert!(r.is_ok(), "{r}");
        let (_, r) = run(r#"{"at":[1,2],"who":null}"#);
        assert!(r.is_ok(), "an explicit null is the same as absent: {r}");
        for (bad, code, path) in [
            (r#"{"at":[1]}"#, "bad_tile", "p.at"),
            (r#"{"at":[1,2,3]}"#, "bad_tile", "p.at"),
            (r#"{"at":[1,99999999999]}"#, "bad_tile", "p.at"),
            (r#"{"at":"1,2"}"#, "type_mismatch", "p.at"),
            (r#"{"at":[1,2],"who":"obj_1"}"#, "bad_id", "p.who"),
            (r#"{"at":[1,2],"who":"pawn_"}"#, "bad_id", "p.who"),
            (r#"{"at":[1,2],"who":"pawn_1A"}"#, "bad_id", "p.who"),
            (r#"{"at":[1,2],"who":5}"#, "type_mismatch", "p.who"),
            (r#"{"at":[1,2],"spot":[1]}"#, "bad_tile", "p.spot"),
            (r#"{}"#, "missing_field", "p.at"),
        ] {
            let (_, r) = run(bad);
            assert!(
                r.errors().any(|i| i.code == code && i.path == path),
                "{bad}: wanted {code} at {path}, got:\n{r}"
            );
        }
    }

    #[test]
    fn non_objects_are_rejected_at_the_top() {
        let mut r = ValidationReport::new();
        schema().check(&Canon::Int(3), "root", &mut r);
        assert!(r.has_code("type_mismatch"));
    }

    #[test]
    fn all_problems_are_collected_not_just_the_first() {
        let (_, r) = check(r#"{"w":99,"extra":1,"kind":"z"}"#);
        let codes: Vec<_> = r.errors().map(|i| i.code).collect();
        assert!(codes.contains(&"out_of_range") && codes.contains(&"unknown_field"));
        assert!(codes.contains(&"bad_enum") && codes.contains(&"missing_field"));
    }
}
