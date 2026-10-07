//! Object templates (Blueprint §4.3): the immutable, data-only description of a kind of object.
//!
//! Templates inherit through `extends`, forming a single chain whose root is `base.object`.
//! `containers` is sugar for the `container` component. Parsing is strict: unknown keys, wrong types
//! and unsupported schema versions are all reported with the path that caused them.

use crate::component::container_def_schema;
use crate::ids::{ComponentName, Tag, TemplateId};
use crate::report::ValidationReport;
use crate::schema::{Field, FieldSchema, ParamSchema};
use pg_canon::{Canon, ToCanon};
use std::collections::{BTreeMap, BTreeSet};

/// The template file-format version this build reads.
pub const TEMPLATE_SCHEMA: u32 = 1;

const TEMPLATE_KEYS: &[&str] = &[
    "id",
    "schema",
    "extends",
    "tags",
    "components",
    "containers",
];
const MAX_TAGS: usize = 32;
const MAX_COMPONENTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectTemplate {
    pub id: TemplateId,
    pub schema: u32,
    pub extends: Option<TemplateId>,
    pub tags: BTreeSet<Tag>,
    /// `Some(params)` sets or merges a component; `None` (JSON `null`) deletes an inherited one.
    pub components: BTreeMap<ComponentName, Option<Canon>>,
}

impl ObjectTemplate {
    /// Parses one template from a canonical value. Problems are recorded in `report` (paths start at
    /// `path`); returns `None` if the template is unusable.
    pub fn from_canon(
        value: &Canon,
        path: &str,
        report: &mut ValidationReport,
    ) -> Option<ObjectTemplate> {
        let Canon::Map(map) = value else {
            report.error("type_mismatch", path, "a template must be an object");
            return None;
        };
        let before = report.error_count();

        for key in map.keys() {
            if !TEMPLATE_KEYS.contains(&key.as_str()) {
                report.error(
                    "unknown_field",
                    format!("{path}.{key}"),
                    format!(
                        "unknown template field (allowed: {})",
                        TEMPLATE_KEYS.join(", ")
                    ),
                );
            }
        }

        let id = match map.get("id").and_then(Canon::as_str) {
            Some(text) => match TemplateId::new(text) {
                Ok(id) => Some(id),
                Err(e) => {
                    report.error("bad_id", format!("{path}.id"), e.to_string());
                    None
                }
            },
            None => {
                report.error(
                    "missing_field",
                    format!("{path}.id"),
                    "a template needs a text 'id'",
                );
                None
            }
        };
        let label = id
            .as_ref()
            .map_or_else(|| path.to_owned(), ToString::to_string);

        let schema = match map
            .get("schema")
            .map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
        {
            Some(Some(n)) if n == TEMPLATE_SCHEMA => TEMPLATE_SCHEMA,
            Some(Some(n)) => {
                report.error(
                    "unsupported_schema",
                    format!("{label}.schema"),
                    format!(
                        "template schema {n} is not supported (this build reads {TEMPLATE_SCHEMA})"
                    ),
                );
                0
            }
            Some(None) => {
                report.error(
                    "type_mismatch",
                    format!("{label}.schema"),
                    "schema must be a non-negative integer",
                );
                0
            }
            None => {
                report.error(
                    "missing_field",
                    format!("{label}.schema"),
                    "a template needs a 'schema' version",
                );
                0
            }
        };

        let extends = match map.get("extends") {
            None | Some(Canon::Null) => None,
            Some(v) => match v.as_str().map(TemplateId::new) {
                Some(Ok(id)) => Some(id),
                Some(Err(e)) => {
                    report.error("bad_id", format!("{label}.extends"), e.to_string());
                    None
                }
                None => {
                    report.error(
                        "type_mismatch",
                        format!("{label}.extends"),
                        "extends must be a template id",
                    );
                    None
                }
            },
        };

        let mut tags = BTreeSet::new();
        match map.get("tags") {
            None => {}
            Some(Canon::List(items)) => {
                if items.len() > MAX_TAGS {
                    report.error(
                        "too_long",
                        format!("{label}.tags"),
                        format!("more than {MAX_TAGS} tags"),
                    );
                }
                for (i, t) in items.iter().enumerate() {
                    match t.as_str().map(Tag::new) {
                        Some(Ok(tag)) => {
                            if !tags.insert(tag.clone()) {
                                report.warn(
                                    "duplicate_tag",
                                    format!("{label}.tags[{i}]"),
                                    format!("tag '{tag}' is listed twice"),
                                );
                            }
                        }
                        Some(Err(e)) => {
                            report.error("bad_id", format!("{label}.tags[{i}]"), e.to_string())
                        }
                        None => report.error(
                            "type_mismatch",
                            format!("{label}.tags[{i}]"),
                            "tags must be text",
                        ),
                    }
                }
            }
            Some(_) => report.error(
                "type_mismatch",
                format!("{label}.tags"),
                "tags must be a list",
            ),
        }

        let mut components: BTreeMap<ComponentName, Option<Canon>> = BTreeMap::new();
        match map.get("components") {
            None => {}
            Some(Canon::Map(comps)) => {
                if comps.len() > MAX_COMPONENTS {
                    report.error(
                        "too_long",
                        format!("{label}.components"),
                        format!("more than {MAX_COMPONENTS} components"),
                    );
                }
                for (name, params) in comps {
                    match ComponentName::new(name) {
                        Ok(cn) => {
                            let params = if matches!(params, Canon::Null) {
                                None
                            } else {
                                Some(params.clone())
                            };
                            components.insert(cn, params);
                        }
                        Err(e) => report.error(
                            "bad_id",
                            format!("{label}.components.{name}"),
                            e.to_string(),
                        ),
                    }
                }
            }
            Some(_) => report.error(
                "type_mismatch",
                format!("{label}.components"),
                "components must be an object",
            ),
        }

        // `containers` is sugar for the `container` component.
        if let Some(sugar) = map.get("containers") {
            let list_schema = ParamSchema::new().field(
                "containers",
                Field::required(FieldSchema::List {
                    item: Box::new(FieldSchema::Object(container_def_schema())),
                    max_len: 8,
                }),
            );
            let probe = Canon::map([("containers", sugar.clone())]);
            let mut scratch = ValidationReport::new();
            list_schema.check(&probe, &label, &mut scratch);
            for issue in scratch.issues() {
                report.error(issue.code, issue.path.clone(), issue.message.clone());
            }
            match ComponentName::new("container") {
                Ok(cn) => match components.entry(cn) {
                    std::collections::btree_map::Entry::Occupied(_) => report.error(
                        "container_conflict",
                        format!("{label}.containers"),
                        "use either 'containers' or components.container, not both",
                    ),
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(Some(probe));
                    }
                },
                Err(e) => report.error("bad_id", format!("{label}.containers"), e.to_string()),
            }
        }

        if report.error_count() > before {
            return None;
        }
        Some(ObjectTemplate {
            id: id?,
            schema,
            extends,
            tags,
            components,
        })
    }
}

impl ToCanon for ObjectTemplate {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("schema", self.schema.to_canon()),
            (
                "extends",
                self.extends.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "tags",
                Canon::List(self.tags.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "components",
                Canon::Map(
                    self.components
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone().unwrap_or(Canon::Null)))
                        .collect(),
                ),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json::parse;

    fn load(json: &str) -> (Option<ObjectTemplate>, ValidationReport) {
        let mut r = ValidationReport::new();
        let t = ObjectTemplate::from_canon(&parse(json).unwrap(), "file.json", &mut r);
        (t, r)
    }

    #[test]
    fn a_minimal_template_loads() {
        let (t, r) = load(r#"{"id":"base.object","schema":1}"#);
        assert!(r.is_ok(), "{r}");
        let t = t.unwrap();
        assert_eq!(t.id.as_str(), "base.object");
        assert!(t.extends.is_none() && t.tags.is_empty() && t.components.is_empty());
    }

    #[test]
    fn a_full_template_loads_and_null_means_delete() {
        let (t, r) = load(
            r#"{"id":"furniture.drawer","schema":1,"extends":"base.furniture","tags":["storage","furniture"],
                "components":{"value":{"base":40},"durability":null}}"#,
        );
        assert!(r.is_ok(), "{r}");
        let t = t.unwrap();
        assert_eq!(t.extends.as_ref().unwrap().as_str(), "base.furniture");
        assert_eq!(t.tags.len(), 2);
        assert_eq!(
            t.components.get(&"durability".parse().unwrap()),
            Some(&None)
        );
        assert!(t
            .components
            .get(&"value".parse().unwrap())
            .unwrap()
            .is_some());
    }

    #[test]
    fn containers_sugar_becomes_the_container_component() {
        let (t, r) = load(
            r#"{"id":"furniture.box","schema":1,"extends":"base.furniture",
                "containers":[{"id":"contents","capacity":4,"accepts":{"tags":["item"]}}]}"#,
        );
        assert!(r.is_ok(), "{r}");
        let t = t.unwrap();
        let c = t
            .components
            .get(&"container".parse().unwrap())
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(c.to_canonical_string().contains(r#""capacity":4"#));
    }

    #[test]
    fn sugar_and_component_together_conflict() {
        let (t, r) = load(
            r#"{"id":"a.b","schema":1,"components":{"container":{"containers":[]}},
                "containers":[{"id":"x","capacity":1}]}"#,
        );
        assert!(t.is_none());
        assert!(r.has_code("container_conflict"));
    }

    #[test]
    fn bad_sugar_is_validated_like_the_component() {
        let (t, r) = load(r#"{"id":"a.b","schema":1,"containers":[{"id":"x","capacity":0}]}"#);
        assert!(t.is_none());
        assert!(r.errors().any(|i| i.code == "out_of_range"), "{r}");
    }

    #[test]
    fn mistakes_are_reported_with_codes() {
        let cases: [(&str, &str); 12] = [
            (r#"[]"#, "type_mismatch"),
            (r#"{"schema":1}"#, "missing_field"),
            (r#"{"id":"base.object"}"#, "missing_field"),
            (r#"{"id":"Base.Object","schema":1}"#, "bad_id"),
            (r#"{"id":"a.b","schema":2}"#, "unsupported_schema"),
            (r#"{"id":"a.b","schema":"1"}"#, "type_mismatch"),
            (r#"{"id":"a.b","schema":1,"extends":"x"}"#, "bad_id"),
            (r#"{"id":"a.b","schema":1,"tags":"x"}"#, "type_mismatch"),
            (r#"{"id":"a.b","schema":1,"tags":["Bad Tag"]}"#, "bad_id"),
            (
                r#"{"id":"a.b","schema":1,"components":[]}"#,
                "type_mismatch",
            ),
            (
                r#"{"id":"a.b","schema":1,"components":{"Bad Name":{}}}"#,
                "bad_id",
            ),
            (r#"{"id":"a.b","schema":1,"colour":"red"}"#, "unknown_field"),
        ];
        for (json, code) in cases {
            let (t, r) = load(json);
            assert!(t.is_none(), "{json} should not load");
            assert!(r.has_code(code), "{json}: wanted {code}, got:\n{r}");
        }
    }

    #[test]
    fn duplicate_tags_warn_but_load() {
        let (t, r) = load(r#"{"id":"a.b","schema":1,"tags":["x","x"]}"#);
        assert!(t.is_some() && r.has_code("duplicate_tag") && r.is_ok());
    }

    #[test]
    fn canonical_form_is_stable() {
        let (t, _) = load(
            r#"{"id":"a.b","schema":1,"extends":"base.object","tags":["y","x"],"components":{"value":{"base":1},"damage":null}}"#,
        );
        assert_eq!(
            t.unwrap().to_canon().to_canonical_string(),
            r#"{"components":{"damage":null,"value":{"base":1}},"extends":"base.object","id":"a.b","schema":1,"tags":["x","y"]}"#
        );
    }
}
