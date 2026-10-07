//! JSON Schema export for pack files (suggestion S-013, Blueprint §23.12).
//!
//! Editors validate and complete JSON as it is typed when they are given a JSON Schema. The schemas here
//! are generated from the same tables the validator uses (the manifest keys, the template keys and every
//! registered component's parameter schema), so they cannot drift from what the loader accepts.

use crate::component::ComponentRegistry;
use crate::schema::{Field, FieldSchema, ParamSchema};
use pg_canon::Canon;
use std::collections::BTreeMap;

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

fn s(text: &str) -> Canon {
    Canon::str(text)
}

fn int(i: i64) -> Canon {
    Canon::Int(i128::from(i))
}

fn obj<const N: usize>(pairs: [(&str, Canon); N]) -> Canon {
    Canon::map(pairs)
}

fn id_pattern() -> Canon {
    obj([
        ("type", s("string")),
        ("pattern", s("^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$")),
    ])
}

/// The JSON Schema for one field shape.
pub fn field_schema(f: &FieldSchema) -> Canon {
    match f {
        FieldSchema::Any => Canon::map::<&str>([]),
        FieldSchema::Int { min, max } => obj([
            ("type", s("integer")),
            ("minimum", int(*min)),
            ("maximum", int(*max)),
        ]),
        FieldSchema::Bool => obj([("type", s("boolean"))]),
        FieldSchema::Text { max_len } => obj([
            ("type", s("string")),
            (
                "maxLength",
                int(i64::try_from(*max_len).unwrap_or(i64::MAX)),
            ),
        ]),
        FieldSchema::Enum(values) => {
            obj([("enum", Canon::List(values.iter().map(|v| s(v)).collect()))])
        }
        FieldSchema::Tag => id_pattern(),
        FieldSchema::TemplateRef => id_pattern(),
        FieldSchema::Tile => obj([
            ("type", s("array")),
            ("items", obj([("type", s("integer"))])),
            ("minItems", int(2)),
            ("maxItems", int(2)),
        ]),
        FieldSchema::EntityId { kind } => obj([
            ("type", s("string")),
            ("pattern", s(&format!("^{kind}_[0-9a-z]+$"))),
        ]),
        FieldSchema::Optional(inner) => obj([(
            "anyOf",
            Canon::List(vec![field_schema(inner), obj([("type", s("null"))])]),
        )]),
        FieldSchema::List { item, max_len } => obj([
            ("type", s("array")),
            ("items", field_schema(item)),
            ("maxItems", int(i64::try_from(*max_len).unwrap_or(i64::MAX))),
        ]),
        FieldSchema::Object(params) => params_schema(params),
    }
}

/// The JSON Schema for an object of named parameters.
pub fn params_schema(p: &ParamSchema) -> Canon {
    let mut props = BTreeMap::new();
    let mut required = Vec::new();
    for (name, Field { schema, default }) in &p.fields {
        let mut fs = field_schema(schema);
        if let (Canon::Map(m), Some(d)) = (&mut fs, default) {
            m.insert("default".to_owned(), d.clone());
        }
        props.insert(name.clone(), fs);
        if default.is_none() {
            required.push(s(name));
        }
    }
    let mut out = BTreeMap::new();
    out.insert("type".to_owned(), s("object"));
    out.insert("properties".to_owned(), Canon::Map(props));
    out.insert("additionalProperties".to_owned(), Canon::Bool(false));
    if !required.is_empty() {
        out.insert("required".to_owned(), Canon::List(required));
    }
    Canon::Map(out)
}

/// The schema for `pack.json`.
pub fn manifest_schema() -> Canon {
    let capabilities: Vec<Canon> = crate::manifest::Capability::ALL
        .iter()
        .map(|c| s(c.name()))
        .collect();
    obj([
        ("$schema", s(DRAFT)),
        ("title", s("Playground pack manifest")),
        ("type", s("object")),
        ("additionalProperties", Canon::Bool(false)),
        (
            "required",
            Canon::List(vec![s("id"), s("name"), s("version")]),
        ),
        (
            "properties",
            obj([
                ("id", id_pattern()),
                ("name", obj([("type", s("string")), ("maxLength", int(64))])),
                (
                    "version",
                    obj([
                        ("type", s("string")),
                        ("description", s("major[.minor[.patch]]")),
                    ]),
                ),
                (
                    "api",
                    obj([
                        ("type", s("string")),
                        ("description", s("A range such as '>=0.1 <0.2'.")),
                    ]),
                ),
                ("engine", obj([("type", s("string"))])),
                (
                    "depends",
                    obj([
                        ("type", s("array")),
                        (
                            "items",
                            obj([
                                ("type", s("object")),
                                ("required", Canon::List(vec![s("id")])),
                                ("additionalProperties", Canon::Bool(false)),
                                (
                                    "properties",
                                    obj([
                                        ("id", id_pattern()),
                                        ("version", obj([("type", s("string"))])),
                                    ]),
                                ),
                            ]),
                        ),
                    ]),
                ),
                (
                    "capabilities",
                    obj([
                        ("type", s("array")),
                        ("items", obj([("enum", Canon::List(capabilities))])),
                        ("uniqueItems", Canon::Bool(true)),
                    ]),
                ),
                (
                    "entry",
                    obj([("type", s("string")), ("pattern", s("^scripts/.*\\.luau$"))]),
                ),
                ("settings", obj([("type", s("array"))])),
            ]),
        ),
    ])
}

/// The schema for a template file (one template object; a file may also hold a list of them).
pub fn template_schema(registry: &ComponentRegistry) -> Canon {
    let mut components = BTreeMap::new();
    for def in registry.iter() {
        let mut p = params_schema(&def.schema);
        if let Canon::Map(m) = &mut p {
            m.insert("description".to_owned(), s(&def.doc));
        }
        components.insert(
            def.name.to_string(),
            obj([("anyOf", Canon::List(vec![p, obj([("type", s("null"))])]))]),
        );
    }
    let template = obj([
        ("type", s("object")),
        ("additionalProperties", Canon::Bool(false)),
        ("required", Canon::List(vec![s("id")])),
        (
            "properties",
            obj([
                ("id", id_pattern()),
                (
                    "schema",
                    obj([
                        ("type", s("integer")),
                        ("const", int(i64::from(crate::template::TEMPLATE_SCHEMA))),
                    ]),
                ),
                ("extends", id_pattern()),
                (
                    "tags",
                    obj([
                        ("type", s("array")),
                        ("items", id_pattern()),
                        ("maxItems", int(32)),
                    ]),
                ),
                (
                    "components",
                    obj([
                        ("type", s("object")),
                        ("properties", Canon::Map(components)),
                        ("additionalProperties", Canon::Bool(false)),
                    ]),
                ),
                ("containers", obj([("type", s("object"))])),
            ]),
        ),
    ]);
    obj([
        ("$schema", s(DRAFT)),
        ("title", s("Playground template file")),
        (
            "oneOf",
            Canon::List(vec![
                template.clone(),
                obj([("type", s("array")), ("items", template)]),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_has_a_schema_and_objects_are_closed() {
        let reg = ComponentRegistry::builtin();
        let t = template_schema(&reg);
        let text = t.to_canonical_string();
        for def in reg.iter() {
            assert!(text.contains(&format!("\"{}\"", def.name)), "{}", def.name);
        }
        assert!(text.contains("additionalProperties"));
        let m = manifest_schema();
        let props = m.get("properties").unwrap();
        for key in [
            "id",
            "name",
            "version",
            "api",
            "engine",
            "depends",
            "capabilities",
            "entry",
            "settings",
        ] {
            assert!(props.get(key).is_some(), "{key}");
        }
        // Every shape converts.
        for f in [
            FieldSchema::Any,
            FieldSchema::Bool,
            FieldSchema::Tile,
            FieldSchema::Tag,
            FieldSchema::EntityId {
                kind: "pawn".into(),
            },
            FieldSchema::Optional(Box::new(FieldSchema::Bool)),
        ] {
            let _ = field_schema(&f);
        }
    }

    #[test]
    fn required_and_default_follow_the_validator() {
        let p = ParamSchema::new()
            .field("a", Field::required_int(0, 5))
            .field("b", Field::int(0, 9, 3));
        let c = params_schema(&p);
        assert_eq!(c.get("required"), Some(&Canon::List(vec![Canon::str("a")])));
        assert_eq!(
            c.get("properties")
                .unwrap()
                .get("b")
                .unwrap()
                .get("default"),
            Some(&Canon::Int(3))
        );
    }
}
