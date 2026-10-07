//! Extension data: pack-defined components and pack health (Blueprint §23.10).
//!
//! Scripts never write the world. A pack declares components (typed integer fields with ranges); their
//! values live here, in [`ExtStore`], are saved and hashed with the rest of the world, and change only
//! through [`apply_set_field`], which validates exactly like built-in code: the component must be
//! declared, the entity must exist and be of a kind the component applies to, the field must exist and
//! the value must be in range (a refused write changes nothing).
//!
//! A component applies to **every** entity of its kinds: an entity with no stored value has the field
//! defaults. Only values that were written are stored, so a world with no packs has an empty store and
//! the store contributes nothing to the canonical form or the state hash; existing saves and replays keep
//! their hashes. Values of components whose pack is missing or quarantined stay in place untouched
//! (orphan data) and are still hashed.
//!
//! The store also remembers each pack's script errors and quarantine, because quarantine changes what the
//! simulation does and so must survive snapshots, keyframes and saves (Blueprint §23.6).

use crate::canon::{Canon, ToCanon};
use crate::id::{EntityId, Kind};
use crate::read::{ReadError, Reader};
use crate::world::WorldState;
use std::collections::BTreeMap;
use std::fmt;

/// An integer field with a range and a default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub min: i64,
    pub max: i64,
    pub default: i64,
}

/// A declared component. `name` is the full stored name, `<pack>.<component>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentDef {
    pub name: String,
    pub pack: String,
    pub applies_to: Vec<Kind>,
    /// Sorted by field name.
    pub fields: Vec<FieldDef>,
    pub version: u32,
}

impl ComponentDef {
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn applies(&self, kind: Kind) -> bool {
        self.applies_to.contains(&kind)
    }
}

/// The declared components of every active pack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtSchemas {
    defs: BTreeMap<String, ComponentDef>,
}

impl ExtSchemas {
    pub fn new() -> ExtSchemas {
        ExtSchemas::default()
    }

    /// Adds a component; refuses a name that is already taken.
    pub fn register(&mut self, def: ComponentDef) -> Result<(), String> {
        if self.defs.contains_key(&def.name) {
            return Err(format!("component '{}' is already declared", def.name));
        }
        self.defs.insert(def.name.clone(), def);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ComponentDef> {
        self.defs.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ComponentDef> {
        self.defs.values()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// One pack's recent script failures and whether it has been switched off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackHealth {
    /// Ticks of recent errors, oldest first.
    pub errors: Vec<u64>,
    pub quarantined_at: Option<u64>,
}

/// Why a script-requested write was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtError {
    UnknownComponent(String),
    UnknownEntity(EntityId),
    WrongKind {
        entity: EntityId,
        component: String,
    },
    UnknownField {
        component: String,
        field: String,
    },
    OutOfRange {
        field: String,
        value: i64,
        min: i64,
        max: i64,
    },
    /// A pack may write only its own components.
    NotOwner {
        pack: String,
        component: String,
    },
}

impl fmt::Display for ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtError::UnknownComponent(c) => write!(f, "no component '{c}' is declared"),
            ExtError::UnknownEntity(e) => write!(f, "no entity {e}"),
            ExtError::WrongKind { entity, component } => {
                write!(f, "component '{component}' does not apply to {entity}")
            }
            ExtError::UnknownField { component, field } => {
                write!(f, "component '{component}' has no field '{field}'")
            }
            ExtError::OutOfRange {
                field,
                value,
                min,
                max,
            } => {
                write!(f, "{value} is outside {min}..={max} for field '{field}'")
            }
            ExtError::NotOwner { pack, component } => {
                write!(f, "pack '{pack}' may not write component '{component}'")
            }
        }
    }
}

impl std::error::Error for ExtError {}

/// The stored extension data of a world.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtStore {
    /// component -> entity -> field -> value; only written values.
    values: BTreeMap<String, BTreeMap<EntityId, BTreeMap<String, i64>>>,
    health: BTreeMap<String, PackHealth>,
}

impl ExtStore {
    pub fn new() -> ExtStore {
        ExtStore::default()
    }

    /// With nothing stored the store is left out of the canonical form and the hash.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.health.is_empty()
    }

    /// The stored value of a field, if one was written.
    pub fn stored(&self, component: &str, entity: EntityId, field: &str) -> Option<i64> {
        self.values
            .get(component)
            .and_then(|m| m.get(&entity))
            .and_then(|f| f.get(field))
            .copied()
    }

    /// Every field of `def` for `entity`: stored values over the defaults.
    pub fn fields_of(&self, def: &ComponentDef, entity: EntityId) -> BTreeMap<String, i64> {
        def.fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    self.stored(&def.name, entity, &f.name).unwrap_or(f.default),
                )
            })
            .collect()
    }

    /// Component names that have stored values (declared or orphaned), in order.
    pub fn component_names(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }

    pub fn health(&self, pack: &str) -> Option<&PackHealth> {
        self.health.get(pack)
    }

    pub fn is_quarantined(&self, pack: &str) -> bool {
        self.health
            .get(pack)
            .is_some_and(|h| h.quarantined_at.is_some())
    }

    /// Records a script failure of `pack` at `tick`. Errors older than `window` ticks are forgotten; when
    /// `max_errors` remain the pack is quarantined. Returns `true` if this call quarantined it. A failure
    /// while loading is passed with `immediate`.
    pub fn record_error(
        &mut self,
        pack: &str,
        tick: u64,
        max_errors: usize,
        window: u64,
        immediate: bool,
    ) -> bool {
        let h = self.health.entry(pack.to_owned()).or_default();
        if h.quarantined_at.is_some() {
            return false;
        }
        h.errors.retain(|t| tick.saturating_sub(*t) < window);
        h.errors.push(tick);
        if immediate || h.errors.len() >= max_errors {
            h.quarantined_at = Some(tick);
            return true;
        }
        false
    }

    /// Lifts a quarantine (the player re-enabled the pack).
    pub fn clear_health(&mut self, pack: &str) {
        self.health.remove(pack);
    }

    /// Row hashes for the state hash: one row per stored entity value set and one per pack health entry.
    pub(crate) fn row_hashes(&self) -> Vec<(String, crate::hash::StateHash)> {
        let mut rows = Vec::new();
        for (component, by_entity) in &self.values {
            for (entity, fields) in by_entity {
                let canon = fields_canon(fields);
                rows.push((
                    format!("{component}/{entity}"),
                    crate::hash::hash_canon(&canon),
                ));
            }
        }
        for (pack, h) in &self.health {
            rows.push((
                format!("health/{pack}"),
                crate::hash::hash_canon(&health_canon(h)),
            ));
        }
        rows
    }

    pub(crate) fn from_reader(r: Reader<'_>) -> Result<ExtStore, ReadError> {
        r.only(&["values", "health"])?;
        let mut store = ExtStore::new();
        if let Some(values) = r.maybe("values")? {
            for (component, by_entity) in values.reader().entries()? {
                let mut entities = BTreeMap::new();
                for (entity, fields) in by_entity.reader().entries()? {
                    let id: EntityId = by_entity_key(&by_entity.reader(), &entity)?;
                    let mut map = BTreeMap::new();
                    for (name, v) in fields.reader().entries()? {
                        map.insert(name, v.reader().i64()?);
                    }
                    entities.insert(id, map);
                }
                store.values.insert(component, entities);
            }
        }
        if let Some(health) = r.maybe("health")? {
            for (pack, h) in health.reader().entries()? {
                let hr = h.reader();
                hr.only(&["errors", "quarantined_at"])?;
                let mut errors = Vec::new();
                for e in hr.child("errors")?.reader().list()? {
                    errors.push(e.reader().u64()?);
                }
                let quarantined_at = match hr.maybe("quarantined_at")? {
                    Some(q) => Some(q.reader().u64()?),
                    None => None,
                };
                store.health.insert(
                    pack,
                    PackHealth {
                        errors,
                        quarantined_at,
                    },
                );
            }
        }
        Ok(store)
    }
}

fn by_entity_key(r: &Reader<'_>, key: &str) -> Result<EntityId, ReadError> {
    key.parse()
        .map_err(|_| r.err(format!("'{key}' is not an entity id")))
}

fn fields_canon(fields: &BTreeMap<String, i64>) -> Canon {
    Canon::Map(
        fields
            .iter()
            .map(|(k, v)| (k.clone(), v.to_canon()))
            .collect(),
    )
}

fn health_canon(h: &PackHealth) -> Canon {
    Canon::map([
        (
            "errors",
            Canon::List(h.errors.iter().map(ToCanon::to_canon).collect()),
        ),
        (
            "quarantined_at",
            h.quarantined_at
                .as_ref()
                .map_or(Canon::Null, ToCanon::to_canon),
        ),
    ])
}

impl ToCanon for ExtStore {
    fn to_canon(&self) -> Canon {
        let mut out = BTreeMap::new();
        if !self.values.is_empty() {
            let values = self
                .values
                .iter()
                .map(|(c, by_entity)| {
                    (
                        c.clone(),
                        Canon::Map(
                            by_entity
                                .iter()
                                .map(|(e, f)| (e.to_string(), fields_canon(f)))
                                .collect(),
                        ),
                    )
                })
                .collect();
            out.insert("values".to_owned(), Canon::Map(values));
        }
        if !self.health.is_empty() {
            let health = self
                .health
                .iter()
                .map(|(p, h)| (p.clone(), health_canon(h)))
                .collect();
            out.insert("health".to_owned(), Canon::Map(health));
        }
        Canon::Map(out)
    }
}

fn kind_of_existing(world: &WorldState, entity: EntityId) -> Option<Kind> {
    let exists = match entity.kind() {
        Kind::Pawn => world.pawns.get(entity).is_some(),
        Kind::Object => world.objects.get(entity).is_some(),
        Kind::Map => world.maps.get(entity).is_some(),
        _ => false,
    };
    exists.then_some(entity.kind())
}

/// Validates and applies a script's request to set one field. `pack` is the requesting pack: it may write
/// only components it declared. A refusal changes nothing.
pub fn apply_set_field(
    world: &mut WorldState,
    schemas: &ExtSchemas,
    pack: &str,
    entity: EntityId,
    component: &str,
    field: &str,
    value: i64,
) -> Result<(), ExtError> {
    let def = schemas
        .get(component)
        .ok_or_else(|| ExtError::UnknownComponent(component.to_owned()))?;
    if def.pack != pack {
        return Err(ExtError::NotOwner {
            pack: pack.to_owned(),
            component: component.to_owned(),
        });
    }
    let kind = kind_of_existing(world, entity).ok_or(ExtError::UnknownEntity(entity))?;
    if !def.applies(kind) {
        return Err(ExtError::WrongKind {
            entity,
            component: component.to_owned(),
        });
    }
    let f = def.field(field).ok_or_else(|| ExtError::UnknownField {
        component: component.to_owned(),
        field: field.to_owned(),
    })?;
    if value < f.min || value > f.max {
        return Err(ExtError::OutOfRange {
            field: field.to_owned(),
            value,
            min: f.min,
            max: f.max,
        });
    }
    world
        .ext
        .values
        .entry(component.to_owned())
        .or_default()
        .entry(entity)
        .or_default()
        .insert(field.to_owned(), value);
    Ok(())
}

#[cfg(test)]
mod tests;
