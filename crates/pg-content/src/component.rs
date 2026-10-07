//! The component registry (Blueprint §4.3).
//!
//! A component has a name and a parameter schema. Built-ins are registered here; pack components
//! (milestone 0.9 onward) register through the script API using the same declarative schemas, so the
//! validator, hasher and inspector handle both identically.

use crate::ids::{ComponentName, PackId};
use crate::schema::{Field, FieldSchema, ParamSchema};
use std::collections::BTreeMap;
use std::fmt;

/// Who registered a component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    Pack(PackId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentDef {
    pub name: ComponentName,
    pub schema: ParamSchema,
    pub origin: Origin,
    pub doc: String,
}

/// Registering a name that already exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateComponent(pub ComponentName);

impl fmt::Display for DuplicateComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "component '{}' is already registered", self.0)
    }
}

impl std::error::Error for DuplicateComponent {}

#[derive(Clone, Debug, Default)]
pub struct ComponentRegistry {
    defs: BTreeMap<ComponentName, ComponentDef>,
}

impl ComponentRegistry {
    /// An empty registry (tests).
    pub fn empty() -> ComponentRegistry {
        ComponentRegistry::default()
    }

    /// A registry holding the built-in components.
    pub fn builtin() -> ComponentRegistry {
        let mut r = ComponentRegistry::empty();
        for def in builtin_defs() {
            // Built-in names are unique by construction; a clash is a bug caught by the tests.
            let _ = r.register(def);
        }
        r
    }

    pub fn register(&mut self, def: ComponentDef) -> Result<(), DuplicateComponent> {
        if self.defs.contains_key(&def.name) {
            return Err(DuplicateComponent(def.name));
        }
        self.defs.insert(def.name.clone(), def);
        Ok(())
    }

    pub fn get(&self, name: &ComponentName) -> Option<&ComponentDef> {
        self.defs.get(name)
    }

    /// Definitions in name order.
    pub fn iter(&self) -> impl Iterator<Item = &ComponentDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// What a container accepts: tags to include, tags to exclude, and specific templates.
pub fn accept_rule_schema() -> ParamSchema {
    ParamSchema::new()
        .field("tags", Field::list(FieldSchema::Tag, 16))
        .field("exclude_tags", Field::list(FieldSchema::Tag, 16))
        .field("templates", Field::list(FieldSchema::TemplateRef, 32))
}

/// One container (`ContainerDef` in the Blueprint). Also the item schema of a template's `containers`
/// sugar and of the `container` component.
pub fn container_def_schema() -> ParamSchema {
    ParamSchema::new()
        .field("id", Field::required_text(32))
        .field("accepts", Field::object(accept_rule_schema()))
        .field("capacity", Field::required_int(1, 1000))
        .field("ordered", Field::boolean(false))
}

/// A built-in definition. Names are literals; the registry tests assert every one parses, and an
/// invalid name would simply be skipped rather than panic.
fn def(name: &str, doc: &str, schema: ParamSchema) -> Option<ComponentDef> {
    Some(ComponentDef {
        name: ComponentName::new(name).ok()?,
        schema,
        origin: Origin::Builtin,
        doc: doc.to_owned(),
    })
}

fn builtin_defs() -> Vec<ComponentDef> {
    let need = &["hunger", "energy", "social"];
    let all = vec![
        def(
            "physical",
            "Size and movement blocking.",
            ParamSchema::new()
                .field("width", Field::int(1, 64, 1))
                .field("height", Field::int(1, 64, 1))
                .field("blocks_movement", Field::boolean(false))
                .field("weight", Field::int(0, 1_000_000, 0)),
        ),
        def(
            "interaction",
            "Named interaction points a pawn can use (sit, sleep, eat, ...).",
            ParamSchema::new().field(
                "points",
                Field::list(
                    FieldSchema::Object(
                        ParamSchema::new()
                            .field("name", Field::required_text(32))
                            .field("dx", Field::int(-8, 8, 0))
                            .field("dy", Field::int(-8, 8, 0))
                            .field("slots", Field::int(1, 8, 1)),
                    ),
                    16,
                ),
            ),
        ),
        def(
            "container",
            "Slots that accept compatible objects.",
            ParamSchema::new().field(
                "containers",
                Field::list(FieldSchema::Object(container_def_schema()), 8),
            ),
        ),
        def(
            "durability",
            "Wear over time.",
            ParamSchema::new()
                .field("max", Field::required_int(1, 1_000_000))
                .field("decay_per_day", Field::int(0, 10_000, 0)),
        ),
        def(
            "value",
            "Worth in minor currency units.",
            ParamSchema::new().field("base", Field::int(0, 1_000_000_000, 0)),
        ),
        def(
            "damage",
            "Hit points.",
            ParamSchema::new().field("max_health", Field::required_int(1, 100_000)),
        ),
        def(
            "needs_restore",
            "Per-minute need changes while used.",
            ParamSchema::new().field(
                "effects",
                Field::list(
                    FieldSchema::Object(
                        ParamSchema::new()
                            .field("need", Field::required_enum(need))
                            .field("per_minute", Field::required_int(-1000, 1000)),
                    ),
                    8,
                ),
            ),
        ),
        def(
            "appearance",
            "How the object is drawn.",
            ParamSchema::new()
                .field("sprite", Field::text(64, "placeholder"))
                .field("layer", Field::int(0, 15, 0)),
        ),
    ];
    all.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eight_builtins_register() {
        let r = ComponentRegistry::builtin();
        let names: Vec<_> = r.iter().map(|d| d.name.to_string()).collect();
        assert_eq!(
            names,
            [
                "appearance",
                "container",
                "damage",
                "durability",
                "interaction",
                "needs_restore",
                "physical",
                "value"
            ]
        );
        assert!(r
            .iter()
            .all(|d| d.origin == Origin::Builtin && !d.doc.is_empty()));
    }

    #[test]
    fn builtin_names_are_valid() {
        for d in builtin_defs() {
            assert!(ComponentName::new(d.name.as_str()).is_ok());
        }
    }

    #[test]
    fn duplicates_are_rejected() {
        let mut r = ComponentRegistry::builtin();
        let dup = r.get(&"physical".parse().unwrap()).unwrap().clone();
        assert_eq!(
            r.register(dup),
            Err(DuplicateComponent("physical".parse().unwrap()))
        );
        assert_eq!(r.len(), 8);
    }

    #[test]
    fn pack_components_register_beside_builtins() {
        let mut r = ComponentRegistry::builtin();
        r.register(ComponentDef {
            name: "coffee_shop.caffeine".parse().unwrap(),
            schema: ParamSchema::new().field("level", Field::int(0, 1000, 0)),
            origin: Origin::Pack("coffee_shop".parse().unwrap()),
            doc: String::new(),
        })
        .unwrap();
        assert_eq!(r.len(), 9);
    }
}
