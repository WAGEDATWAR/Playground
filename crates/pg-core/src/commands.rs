//! Commands: requests that enter the simulation as `SimInput`s and are applied at tick boundaries.
//!
//! Milestone 0.4 has only developer commands. They let the dev tools and tests build and poke a world
//! entirely through logged inputs, so every run is replayable. They are marked developer-only and are
//! replaced by real gameplay commands (and the editor's `EditCommand`s) in later milestones.

use crate::canon::{Canon, CanonError, ToCanon};
use crate::commitment::{CommitState, Commitment};
use crate::containment;
use crate::id::{EntityId, Kind};
use crate::map::{MapKind, Tile};
use crate::object::{Location, Parent};
use crate::pawn::Route;
use crate::pipeline::Event;
use crate::reason::ReasonCode;
use crate::world::WorldState;
use pg_content::ContentSet;

/// Gameplay commands (UI → core). Grows with the milestones; non-exhaustive on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// Generates the town (milestone 1.2, Blueprint §12.1): the map, districts, roads, buildings and
    /// `residents` people with their households and relationships, from the world's seed. Needs content
    /// with game data and a world that has no map yet.
    GenerateTown {
        w: i32,
        h: i32,
        /// Percent of the map that is water.
        water: u32,
        residents: u32,
        /// `cozy`, `standard` or `mature`.
        tone: String,
    },
    /// Records the lines a conversation actually had (generated text, composed with the fallback for turns
    /// already spoken). Presentation entering as a logged input so it is part of the replayable history
    /// (milestone 1.6; Blueprint section 9.4). `a` is the lower id of the pair.
    RecordDialogue {
        a: EntityId,
        b: EntityId,
        started: u64,
        lines: Vec<String>,
    },
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
    /// **Dev:** `proposer` asks `invitee` to meet at `at` during slots `start..start+len` today. The
    /// proposal lapses `expires_in` ticks from now.
    DevPropose {
        proposer: EntityId,
        invitee: EntityId,
        start: u32,
        len: u32,
        at: Tile,
        expires_in: u32,
        reschedulable: bool,
    },
    /// **Dev:** cancels a live commitment and frees both pawns' reserved slots.
    DevCancelCommitment { commitment: EntityId },
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

fn u32_of(c: &Canon, key: &str) -> Result<u32, CanonError> {
    c.field(key)?
        .as_i64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| CanonError(format!("'{key}' must be a non-negative integer")))
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
            Command::GenerateTown {
                w,
                h,
                water,
                residents,
                tone,
            } => Canon::map([
                ("type", Canon::str("generate_town")),
                ("w", w.to_canon()),
                ("h", h.to_canon()),
                ("water", water.to_canon()),
                ("residents", residents.to_canon()),
                ("tone", tone.to_canon()),
            ]),
            Command::RecordDialogue {
                a,
                b,
                started,
                lines,
            } => Canon::map([
                ("type", Canon::str("record_dialogue")),
                ("a", a.to_canon()),
                ("b", b.to_canon()),
                ("started", started.to_canon()),
                (
                    "lines",
                    Canon::List(lines.iter().map(|l| Canon::str(l.clone())).collect()),
                ),
            ]),
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
            Command::DevPropose {
                proposer,
                invitee,
                start,
                len,
                at,
                expires_in,
                reschedulable,
            } => Canon::map([
                ("type", Canon::str("dev_propose")),
                ("proposer", proposer.to_canon()),
                ("invitee", invitee.to_canon()),
                ("start", start.to_canon()),
                ("len", len.to_canon()),
                ("at", at.to_canon()),
                ("expires_in", expires_in.to_canon()),
                ("reschedulable", reschedulable.to_canon()),
            ]),
            Command::DevCancelCommitment { commitment } => Canon::map([
                ("type", Canon::str("dev_cancel_commitment")),
                ("commitment", commitment.to_canon()),
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
            "generate_town" => Ok(Command::GenerateTown {
                w: int_of(c, "w")?,
                h: int_of(c, "h")?,
                water: u32_of(c, "water")?,
                residents: u32_of(c, "residents")?,
                tone: text_of(c, "tone")?,
            }),
            "record_dialogue" => Ok(Command::RecordDialogue {
                a: id_of(c, "a")?,
                b: id_of(c, "b")?,
                started: c
                    .field("started")?
                    .as_i64()
                    .and_then(|v| u64::try_from(v).ok())
                    .ok_or_else(|| CanonError::new("started must be a non-negative integer"))?,
                lines: match c.field("lines")? {
                    Canon::List(l) => l
                        .iter()
                        .map(|x| {
                            x.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| CanonError::new("a line must be text"))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err(CanonError::new("lines must be a list")),
                },
            }),
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
            "dev_propose" => Ok(Command::DevPropose {
                proposer: id_of(c, "proposer")?,
                invitee: id_of(c, "invitee")?,
                start: u32_of(c, "start")?,
                len: u32_of(c, "len")?,
                at: tile_of(c, "at")?,
                expires_in: u32_of(c, "expires_in")?,
                reschedulable: c
                    .field("reschedulable")?
                    .as_bool()
                    .ok_or_else(|| CanonError::new("'reschedulable' must be true or false"))?,
            }),
            "dev_cancel_commitment" => Ok(Command::DevCancelCommitment {
                commitment: id_of(c, "commitment")?,
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
        Command::GenerateTown {
            w,
            h,
            water,
            residents,
            tone,
        } => {
            let Some(content) = content else {
                events.push(rejected(tick, "generate_town", "no content is loaded"));
                return;
            };
            let Some(tone) = crate::world::TonePreset::parse(tone) else {
                events.push(rejected(
                    tick,
                    "generate_town",
                    "the tone is cozy, standard or mature",
                ));
                return;
            };
            let params = crate::worldgen::GenParams {
                width: *w,
                height: *h,
                water_percent: *water,
                residents: usize::try_from(*residents).unwrap_or(usize::MAX),
                tone,
            };
            match crate::worldgen::generate_town(world, content.game(), &params) {
                Ok(out) => events.push(event(
                    tick,
                    "town.generated",
                    Canon::map([
                        ("map", out.map.to_canon()),
                        ("districts", (out.districts as u64).to_canon()),
                        ("buildings", (out.buildings as u64).to_canon()),
                        ("residents", (out.residents as u64).to_canon()),
                        ("attempt", out.attempt.to_canon()),
                        ("hash", Canon::str(out.starting_hash)),
                    ]),
                )),
                Err(e) => events.push(rejected(tick, "generate_town", &e.to_string())),
            }
        }
        Command::RecordDialogue {
            a,
            b,
            started,
            lines,
        } => match crate::conversation::record_dialogue(world, *a, *b, *started, lines) {
            Ok(when) => events.push(event(
                tick,
                "dialogue.recorded",
                Canon::map([
                    ("a", a.to_canon()),
                    ("b", b.to_canon()),
                    ("started", started.to_canon()),
                    ("when", Canon::str(when)),
                ]),
            )),
            Err(why) => events.push(rejected(tick, "record_dialogue", why)),
        },
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
        Command::DevPropose {
            proposer,
            invitee,
            start,
            len,
            at,
            expires_in,
            reschedulable,
        } => propose(
            world,
            tick,
            events,
            (
                *proposer,
                *invitee,
                *start,
                *len,
                *at,
                *expires_in,
                *reschedulable,
            ),
        ),
        Command::DevCancelCommitment { commitment } => cancel(world, tick, events, *commitment),
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

type ProposalArgs = (EntityId, EntityId, u32, u32, Tile, u32, bool);

fn propose(world: &mut WorldState, tick: u64, events: &mut Vec<Event>, args: ProposalArgs) {
    let (proposer, invitee, start, len, at, expires_in, reschedulable) = args;
    let refuse =
        |events: &mut Vec<Event>, why: &str| events.push(rejected(tick, "dev_propose", why));
    if proposer == invitee {
        return refuse(events, "a pawn cannot meet itself");
    }
    let Some(map) = world.pawns.get(proposer).map(|p| p.position.map) else {
        return refuse(events, &format!("no pawn {proposer}"));
    };
    if !world.pawns.contains(invitee) {
        return refuse(events, &format!("no pawn {invitee}"));
    }
    let slots = world.settings.slot_minutes.slots_per_day();
    if len == 0 || start.checked_add(len).is_none_or(|end| end > slots) {
        return refuse(
            events,
            &format!("slots {start}+{len} do not fit in a day of {slots}"),
        );
    }
    if !world.is_passable(map, at) {
        return refuse(events, &format!("tile {at} cannot be stood on"));
    }
    let Ok(action) = pg_content::ActionId::new("meet_at") else {
        return refuse(events, "the meet_at action id is invalid");
    };
    let id = match world.id_counters.allocate(Kind::Commitment) {
        Ok(id) => id,
        Err(e) => return refuse(events, &e.to_string()),
    };
    let commitment = Commitment {
        id,
        proposer,
        invitee,
        day: world.clock.day(),
        start,
        len,
        action,
        params: Canon::map([("at", at.to_canon())]),
        state: CommitState::Proposed,
        created_tick: tick,
        expires_tick: tick.saturating_add(u64::from(expires_in)),
        reschedulable,
        reason: None,
    };
    let _ = world.commitments.insert(id, commitment);
    events.push(event(
        tick,
        "commitment.proposed",
        Canon::map([
            ("commitment", id.to_canon()),
            ("proposer", proposer.to_canon()),
            ("invitee", invitee.to_canon()),
            ("start", start.to_canon()),
            ("len", len.to_canon()),
        ]),
    ));
}

fn cancel(world: &mut WorldState, tick: u64, events: &mut Vec<Event>, id: EntityId) {
    let Some(c) = world.commitments.get_mut(id) else {
        return events.push(rejected(
            tick,
            "dev_cancel_commitment",
            &format!("no commitment {id}"),
        ));
    };
    let reason = ReasonCode::builtin(
        "commitment_cancelled",
        [("why", Canon::str("cancelled by request"))],
    );
    if let Err(e) = c.transition(CommitState::Cancelled, reason) {
        return events.push(rejected(tick, "dev_cancel_commitment", &e.to_string()));
    }
    let parties = c.parties();
    for who in parties {
        if let Some(schedule) = world.pawns.get_mut(who).and_then(|p| p.schedule.as_mut()) {
            let held: Vec<u32> = schedule
                .reservations()
                .filter(|r| r.commitment == Some(id))
                .map(|r| r.id)
                .collect();
            for r in held {
                schedule.remove(r);
            }
        }
    }
    events.push(event(
        tick,
        "commitment.cancelled",
        Canon::map([("commitment", id.to_canon())]),
    ));
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
            Command::DevPropose {
                proposer: id(Kind::Pawn, 1),
                invitee: id(Kind::Pawn, 2),
                start: 4,
                len: 2,
                at: Tile::new(3, 3),
                expires_in: 600,
                reschedulable: true,
            },
            Command::DevCancelCommitment {
                commitment: id(Kind::Commitment, 1),
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
