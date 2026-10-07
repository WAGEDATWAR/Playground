//! Commands: requests that enter the simulation as `SimInput`s and are applied at tick boundaries.
//!
//! Milestone 0.4 has only developer commands. They let the dev tools and tests build and poke a world
//! entirely through logged inputs, so every run is replayable. They are marked developer-only and are
//! replaced by real gameplay commands (and the editor's `EditCommand`s) in later milestones.

use crate::canon::{Canon, CanonError, ToCanon};
use crate::containment;
use crate::id::EntityId;
use crate::map::{MapKind, Tile};
use crate::object::{Location, Parent};
use crate::pawn::Route;
use crate::pipeline::Event;
use crate::world::WorldState;
use pg_content::ContentSet;

/// Gameplay commands (UI → core). Grows with the milestones; non-exhaustive on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// **Dev:** adds `amount` to the dev probe value.
    DevNudge { amount: i32 },
    /// **Dev:** creates an overworld map of `w`×`h` tiles. `style` 0 is open grass; 1 is a generated town
    /// (roads, sidewalks, buildings, a pond).
    DevCreateMap { w: i32, h: i32, style: u8 },
    /// **Dev:** creates a pawn on `at`, or on a seeded random free tile when `at` is `None`.
    DevSpawnPawn {
        map: EntityId,
        at: Option<Tile>,
        name: String,
    },
    /// **Dev:** creates an object from a template on a tile (needs content loaded).
    DevSpawnObject {
        map: EntityId,
        at: Tile,
        template: String,
    },
    /// **Dev:** moves an object into a container (needs content loaded).
    DevPutInContainer {
        child: EntityId,
        owner: EntityId,
        container: String,
    },
    /// **Dev:** sends a pawn to a tile.
    DevMove { pawn: EntityId, to: Tile },
    /// **Dev:** blocks or unblocks a tile (a map edit; pawns re-path around it).
    DevSetBlocked {
        map: EntityId,
        at: Tile,
        blocked: bool,
    },
}

fn id_of(c: &Canon, key: &str) -> Result<EntityId, CanonError> {
    c.field(key)?
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| CanonError(format!("'{key}' must be an entity id")))
}

fn tile_of(c: &Canon, key: &str) -> Result<Tile, CanonError> {
    Tile::from_canon(c.field(key)?).ok_or_else(|| CanonError(format!("'{key}' must be [x, y]")))
}

fn int_of(c: &Canon, key: &str) -> Result<i32, CanonError> {
    c.field(key)?
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| CanonError(format!("'{key}' must be an integer that fits i32")))
}

fn text_of(c: &Canon, key: &str) -> Result<String, CanonError> {
    c.field(key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| CanonError(format!("'{key}' must be text")))
}

impl ToCanon for Command {
    fn to_canon(&self) -> Canon {
        match self {
            Command::DevNudge { amount } => Canon::map([
                ("type", Canon::str("dev_nudge")),
                ("amount", amount.to_canon()),
            ]),
            Command::DevCreateMap { w, h, style } => Canon::map([
                ("type", Canon::str("dev_create_map")),
                ("w", w.to_canon()),
                ("h", h.to_canon()),
                ("style", style.to_canon()),
            ]),
            Command::DevSpawnPawn { map, at, name } => Canon::map([
                ("type", Canon::str("dev_spawn_pawn")),
                ("map", map.to_canon()),
                ("at", at.as_ref().map_or(Canon::Null, ToCanon::to_canon)),
                ("name", name.to_canon()),
            ]),
            Command::DevSpawnObject { map, at, template } => Canon::map([
                ("type", Canon::str("dev_spawn_object")),
                ("map", map.to_canon()),
                ("at", at.to_canon()),
                ("template", template.to_canon()),
            ]),
            Command::DevPutInContainer {
                child,
                owner,
                container,
            } => Canon::map([
                ("type", Canon::str("dev_put_in_container")),
                ("child", child.to_canon()),
                ("owner", owner.to_canon()),
                ("container", container.to_canon()),
            ]),
            Command::DevMove { pawn, to } => Canon::map([
                ("type", Canon::str("dev_move")),
                ("pawn", pawn.to_canon()),
                ("to", to.to_canon()),
            ]),
            Command::DevSetBlocked { map, at, blocked } => Canon::map([
                ("type", Canon::str("dev_set_blocked")),
                ("map", map.to_canon()),
                ("at", at.to_canon()),
                ("blocked", blocked.to_canon()),
            ]),
        }
    }
}

impl Command {
    pub fn from_canon(c: &Canon) -> Result<Command, CanonError> {
        let ty = c
            .field("type")?
            .as_str()
            .ok_or_else(|| CanonError::new("command type must be text"))?;
        match ty {
            "dev_nudge" => Ok(Command::DevNudge {
                amount: int_of(c, "amount")?,
            }),
            "dev_create_map" => Ok(Command::DevCreateMap {
                w: int_of(c, "w")?,
                h: int_of(c, "h")?,
                style: u8::try_from(int_of(c, "style")?)
                    .map_err(|_| CanonError::new("'style' must be 0..=255"))?,
            }),
            "dev_spawn_pawn" => Ok(Command::DevSpawnPawn {
                map: id_of(c, "map")?,
                at: match c.field("at")? {
                    Canon::Null => None,
                    _ => Some(tile_of(c, "at")?),
                },
                name: text_of(c, "name")?,
            }),
            "dev_spawn_object" => Ok(Command::DevSpawnObject {
                map: id_of(c, "map")?,
                at: tile_of(c, "at")?,
                template: text_of(c, "template")?,
            }),
            "dev_put_in_container" => Ok(Command::DevPutInContainer {
                child: id_of(c, "child")?,
                owner: id_of(c, "owner")?,
                container: text_of(c, "container")?,
            }),
            "dev_move" => Ok(Command::DevMove {
                pawn: id_of(c, "pawn")?,
                to: tile_of(c, "to")?,
            }),
            "dev_set_blocked" => Ok(Command::DevSetBlocked {
                map: id_of(c, "map")?,
                at: tile_of(c, "at")?,
                blocked: c
                    .field("blocked")?
                    .as_bool()
                    .ok_or_else(|| CanonError::new("'blocked' must be true or false"))?,
            }),
            other => Err(CanonError(format!("unknown command type '{other}'"))),
        }
    }
}

fn event(tick: u64, kind: &str, detail: Canon) -> Event {
    Event {
        tick,
        kind: kind.to_owned(),
        detail,
    }
}

fn rejected(tick: u64, command: &str, reason: &str) -> Event {
    event(
        tick,
        "input_rejected",
        Canon::map([
            ("command", Canon::str(command)),
            ("reason", Canon::str(reason)),
        ]),
    )
}

/// Applies a command to the world. Refusals become `input_rejected` events and change nothing.
pub(crate) fn apply(
    world: &mut WorldState,
    content: Option<&ContentSet>,
    tick: u64,
    cmd: &Command,
    events: &mut Vec<Event>,
) {
    match cmd {
        Command::DevNudge { amount } => {
            world.probe.value = world.probe.value.saturating_add(i64::from(*amount));
            events.push(event(
                tick,
                "dev.nudged",
                Canon::map([("amount", amount.to_canon())]),
            ));
        }
        Command::DevCreateMap { w, h, style } => match world.create_map(MapKind::Overworld, *w, *h)
        {
            Ok(id) => {
                crate::dev::generate_dev_map(world, id, *style);
                events.push(event(
                    tick,
                    "map.created",
                    Canon::map([
                        ("map", id.to_canon()),
                        ("w", w.to_canon()),
                        ("h", h.to_canon()),
                        ("style", style.to_canon()),
                    ]),
                ));
            }
            Err(e) => events.push(rejected(tick, "dev_create_map", &e.to_string())),
        },
        Command::DevSpawnPawn { map, at, name } => {
            let tile = at.or_else(|| crate::dev::random_free_tile(world, *map));
            let Some(tile) = tile else {
                events.push(rejected(tick, "dev_spawn_pawn", "no free tile"));
                return;
            };
            match world.spawn_pawn(name, *map, tile) {
                Ok(id) => events.push(event(
                    tick,
                    "pawn.spawned",
                    Canon::map([("pawn", id.to_canon()), ("tile", tile.to_canon())]),
                )),
                Err(e) => events.push(rejected(tick, "dev_spawn_pawn", &e.to_string())),
            }
        }
        Command::DevSpawnObject { map, at, template } => {
            let Some(content) = content else {
                events.push(rejected(tick, "dev_spawn_object", "no content is loaded"));
                return;
            };
            let Ok(template_id) = template.parse() else {
                events.push(rejected(
                    tick,
                    "dev_spawn_object",
                    &format!("'{template}' is not a template id"),
                ));
                return;
            };
            match containment::spawn_object(
                world,
                content,
                &template_id,
                Location::OnMap {
                    map: *map,
                    tile: *at,
                },
            ) {
                Ok(id) => events.push(event(
                    tick,
                    "object.spawned",
                    Canon::map([("object", id.to_canon()), ("tile", at.to_canon())]),
                )),
                Err(e) => events.push(rejected(tick, "dev_spawn_object", &e.to_string())),
            }
        }
        Command::DevPutInContainer {
            child,
            owner,
            container,
        } => {
            let Some(content) = content else {
                events.push(rejected(
                    tick,
                    "dev_put_in_container",
                    "no content is loaded",
                ));
                return;
            };
            match containment::move_into(world, content, *child, *owner, container) {
                Ok(()) => {
                    let parent = Parent {
                        owner: *owner,
                        container: container.clone(),
                    };
                    events.push(event(
                        tick,
                        "object.contained",
                        Canon::map([
                            ("object", child.to_canon()),
                            ("owner", parent.owner.to_canon()),
                            ("container", Canon::str(parent.container)),
                        ]),
                    ));
                }
                Err(e) => events.push(rejected(tick, "dev_put_in_container", &e.to_string())),
            }
        }
        Command::DevMove { pawn, to } => {
            let Some(p) = world.pawns.get(*pawn) else {
                events.push(rejected(tick, "dev_move", &format!("no pawn {pawn}")));
                return;
            };
            if !world.is_passable(p.position.map, *to) {
                events.push(rejected(
                    tick,
                    "dev_move",
                    &format!("tile {to} cannot be stood on"),
                ));
                return;
            }
            if let Some(p) = world.pawns.get_mut(*pawn) {
                p.route = Some(Route::to(*to));
            }
            events.push(event(
                tick,
                "move.requested",
                Canon::map([("pawn", pawn.to_canon()), ("to", to.to_canon())]),
            ));
        }
        Command::DevSetBlocked { map, at, blocked } => match world.maps.get_mut(*map) {
            None => events.push(rejected(tick, "dev_set_blocked", &format!("no map {map}"))),
            Some(m) => match m.set_blocked(*at, *blocked) {
                Ok(()) => events.push(event(
                    tick,
                    "world_edited",
                    Canon::map([
                        ("map", map.to_canon()),
                        ("tile", at.to_canon()),
                        ("blocked", blocked.to_canon()),
                    ]),
                )),
                Err(e) => events.push(rejected(tick, "dev_set_blocked", &e.to_string())),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    fn id(kind: Kind, n: u32) -> EntityId {
        EntityId::new(kind, n)
    }

    #[test]
    fn every_command_round_trips_through_canonical_form() {
        let cmds = vec![
            Command::DevNudge { amount: -5 },
            Command::DevCreateMap {
                w: 64,
                h: 48,
                style: 1,
            },
            Command::DevSpawnPawn {
                map: id(Kind::Map, 1),
                at: None,
                name: "Ann".into(),
            },
            Command::DevSpawnPawn {
                map: id(Kind::Map, 1),
                at: Some(Tile::new(3, 4)),
                name: "Bob".into(),
            },
            Command::DevSpawnObject {
                map: id(Kind::Map, 1),
                at: Tile::new(1, 2),
                template: "furniture.bed".into(),
            },
            Command::DevPutInContainer {
                child: id(Kind::Object, 2),
                owner: id(Kind::Object, 1),
                container: "contents".into(),
            },
            Command::DevMove {
                pawn: id(Kind::Pawn, 1),
                to: Tile::new(9, 9),
            },
            Command::DevSetBlocked {
                map: id(Kind::Map, 1),
                at: Tile::new(5, 5),
                blocked: true,
            },
        ];
        for c in cmds {
            assert_eq!(Command::from_canon(&c.to_canon()).unwrap(), c, "{c:?}");
        }
    }

    #[test]
    fn decoding_rejects_malformed_commands() {
        for bad in [
            r#"{"type":"nope"}"#,
            r#"{"type":"dev_nudge"}"#,
            r#"{"type":"dev_nudge","amount":99999999999}"#,
            r#"{"type":"dev_create_map","w":1,"h":1,"style":999}"#,
            r#"{"type":"dev_move","pawn":"pawn_1"}"#,
            r#"{"type":"dev_move","pawn":"dog_1","to":[1,1]}"#,
            r#"{"type":"dev_move","pawn":"pawn_1","to":[1]}"#,
            r#"{"type":"dev_set_blocked","map":"map_1","at":[1,1],"blocked":"yes"}"#,
            r#"{"type":"dev_spawn_pawn","map":"map_1","name":"x"}"#,
        ] {
            let c = pg_canon::json::parse(bad).unwrap();
            assert!(Command::from_canon(&c).is_err(), "{bad}");
        }
    }
}
