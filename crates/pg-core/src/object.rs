//! Object instances and containment state (Blueprint §4.4).
//!
//! An object is made from a template (`pg-content`), lives either on a map tile or inside another
//! object's container, and owns the container slots its template defines. The logic that keeps these
//! consistent is in [`crate::containment`].

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::map::Tile;
use crate::read::{ReadError, Reader};
use pg_content::TemplateId;
use std::collections::BTreeMap;

/// The container an object sits in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parent {
    pub owner: EntityId,
    pub container: String,
}

/// Where an object is. Exactly one place at a time, which is how "every entity has at most one parent"
/// is enforced structurally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    OnMap { map: EntityId, tile: Tile },
    InContainer(Parent),
}

/// One container's contents, in insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerState {
    pub slots: Vec<EntityId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInstance {
    pub id: EntityId,
    pub template: TemplateId,
    pub location: Location,
    /// One entry per container the template defines, keyed by container id.
    pub containers: BTreeMap<String, ContainerState>,
}

impl ToCanon for Location {
    fn to_canon(&self) -> Canon {
        match self {
            Location::OnMap { map, tile } => Canon::map([
                ("kind", Canon::str("map")),
                ("map", map.to_canon()),
                ("tile", tile.to_canon()),
            ]),
            Location::InContainer(p) => Canon::map([
                ("kind", Canon::str("container")),
                ("owner", p.owner.to_canon()),
                ("container", Canon::str(p.container.clone())),
            ]),
        }
    }
}

impl ToCanon for ObjectInstance {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("template", self.template.to_canon()),
            ("location", self.location.to_canon()),
            (
                "containers",
                Canon::Map(
                    self.containers
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                Canon::List(v.slots.iter().map(ToCanon::to_canon).collect()),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

impl ObjectInstance {
    pub fn from_reader(r: Reader<'_>) -> Result<ObjectInstance, ReadError> {
        r.only(&["id", "template", "location", "containers"])?;
        let loc = r.child("location")?;
        let l = loc.reader();
        let kind = l.child("kind")?;
        let location = match kind.reader().str()? {
            "map" => {
                l.only(&["kind", "map", "tile"])?;
                Location::OnMap {
                    map: l.child("map")?.reader().parse()?,
                    tile: Tile::from_reader(l.child("tile")?.reader())?,
                }
            }
            "container" => {
                l.only(&["kind", "owner", "container"])?;
                Location::InContainer(Parent {
                    owner: l.child("owner")?.reader().parse()?,
                    container: l.child("container")?.reader().str()?.to_owned(),
                })
            }
            other => {
                return Err(kind
                    .reader()
                    .err(format!("unknown location kind '{other}'")))
            }
        };
        let mut containers = BTreeMap::new();
        for (name, child) in r.child("containers")?.reader().entries()? {
            let slots = child
                .reader()
                .list()?
                .iter()
                .map(|i| i.reader().parse())
                .collect::<Result<Vec<EntityId>, _>>()?;
            containers.insert(name, ContainerState { slots });
        }
        Ok(ObjectInstance {
            id: r.child("id")?.reader().parse()?,
            template: r.child("template")?.reader().parse()?,
            location,
            containers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    #[test]
    fn canonical_form_is_stable() {
        let mut containers = BTreeMap::new();
        containers.insert(
            "contents".to_owned(),
            ContainerState {
                slots: vec![EntityId::new(Kind::Object, 2)],
            },
        );
        let o = ObjectInstance {
            id: EntityId::new(Kind::Object, 1),
            template: "furniture.drawer".parse().unwrap(),
            location: Location::OnMap {
                map: EntityId::new(Kind::Map, 1),
                tile: Tile::new(4, 5),
            },
            containers,
        };
        assert_eq!(
            o.to_canon().to_canonical_string(),
            r#"{"containers":{"contents":["obj_2"]},"id":"obj_1","location":{"kind":"map","map":"map_1","tile":[4,5]},"template":"furniture.drawer"}"#
        );
    }
}
