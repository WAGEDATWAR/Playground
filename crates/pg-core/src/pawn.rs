//! Pawns, spatial fields only (Blueprint §4.6). Needs, mood, memory and the rest arrive in Stage 1;
//! milestone 0.4 needs position, facing and the route a pawn is following.

use crate::canon::{Canon, ToCanon};
use crate::capacity::Capacities;
use crate::id::EntityId;
use crate::map::{Dir4, Tile};
use crate::read::{ReadError, Reader};
use crate::reason::ReasonCode;
use crate::schedule::DaySchedule;
use crate::social::{Memory, Occupation};
use pg_content::ActionId;
use std::collections::BTreeMap;

/// Where a pawn is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Position {
    pub map: EntityId,
    pub tile: Tile,
}

impl ToCanon for Position {
    fn to_canon(&self) -> Canon {
        Canon::map([("map", self.map.to_canon()), ("tile", self.tile.to_canon())])
    }
}

/// The path a pawn is walking. Authoritative state: it is saved, hashed and replayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Where the pawn is going.
    pub goal: Tile,
    /// The tiles to visit, excluding the tile the pawn stood on when the path was solved. Empty until the
    /// first solve.
    pub path: Vec<Tile>,
    /// Index into `path` of the next tile to enter.
    pub next: usize,
    /// Ticks accumulated toward the next step (a step happens at `move_ticks_per_tile`).
    pub since_step: u32,
    /// Ticks spent waiting on an occupied next tile.
    pub waited: u32,
    /// Times the path was re-solved after the first solve (reset on arrival).
    pub repaths: u32,
    /// A (re)solve is wanted: set on creation, on a map edit that blocks the path, and after a sidestep.
    pub needs_repath: bool,
}

impl Route {
    /// A route to `goal` that has not been solved yet.
    pub fn to(goal: Tile) -> Route {
        Route {
            goal,
            path: Vec::new(),
            next: 0,
            since_step: 0,
            waited: 0,
            repaths: 0,
            needs_repath: true,
        }
    }

    /// Tiles still to enter.
    pub fn remaining(&self) -> &[Tile] {
        self.path.get(self.next..).unwrap_or(&[])
    }
}

impl ToCanon for Route {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("goal", self.goal.to_canon()),
            (
                "path",
                Canon::List(self.path.iter().map(ToCanon::to_canon).collect()),
            ),
            ("next", (self.next as u64).to_canon()),
            ("since_step", self.since_step.to_canon()),
            ("waited", self.waited.to_canon()),
            ("repaths", self.repaths.to_canon()),
            ("needs_repath", self.needs_repath.to_canon()),
        ])
    }
}

/// What a pawn is supposed to be doing right now (Blueprint §8.5, "Activation").
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Unreserved time: the pawn is open to self-directed behaviour.
    Free,
    /// The reservation, in the pawn's current schedule, that covers the current slot.
    Reservation(u32),
}

impl ToCanon for Intent {
    fn to_canon(&self) -> Canon {
        match self {
            Intent::Free => Canon::str("free"),
            Intent::Reservation(id) => Canon::map([("reservation", id.to_canon())]),
        }
    }
}

/// One concrete step of a task.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Walk to a tile (the movement system does the walking). The step is done once the pawn is within
    /// `within` tiles (Manhattan) of the goal; 0 means exactly on it.
    MoveTo { goal: Tile, within: u32 },
    /// Stay put until this absolute tick.
    PerformUntil(u64),
}

impl ToCanon for Step {
    fn to_canon(&self) -> Canon {
        match self {
            Step::MoveTo { goal, within } => Canon::map([(
                "move_to",
                Canon::map([("goal", goal.to_canon()), ("within", within.to_canon())]),
            )]),
            Step::PerformUntil(tick) => Canon::map([("perform_until", tick.to_canon())]),
        }
    }
}

/// A reservation being carried out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// The reservation that started it.
    pub reservation: u32,
    pub action: ActionId,
    pub steps: Vec<Step>,
    /// Index of the step being worked on; equal to `steps.len()` when the task is done.
    pub current: usize,
    /// The current `MoveTo` step has handed its route to the movement system.
    pub moving: bool,
    pub interruptible: bool,
}

impl Task {
    pub fn is_done(&self) -> bool {
        self.current >= self.steps.len()
    }

    pub fn step(&self) -> Option<&Step> {
        self.steps.get(self.current)
    }
}

impl ToCanon for Task {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("reservation", self.reservation.to_canon()),
            ("action", self.action.to_canon()),
            (
                "steps",
                Canon::List(self.steps.iter().map(ToCanon::to_canon).collect()),
            ),
            ("current", (self.current as u64).to_canon()),
            ("moving", self.moving.to_canon()),
            ("interruptible", self.interruptible.to_canon()),
        ])
    }
}

/// A request to re-plan the rest of the day, made by a system that noticed a trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replan {
    pub from: u32,
    pub why: String,
}

impl ToCanon for Replan {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("from", self.from.to_canon()),
            ("why", Canon::str(self.why.clone())),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pawn {
    pub id: EntityId,
    pub name: String,
    pub position: Position,
    pub facing: Dir4,
    pub route: Option<Route>,
    /// Today's plan; `None` until the planner has run for this pawn.
    pub schedule: Option<DaySchedule>,
    pub intent: Intent,
    pub task: Option<Task>,
    pub replan: Option<Replan>,
    /// Why the last route failed, left by the movement system for the activity system to read.
    pub move_failure: Option<String>,
    /// The last task failure, kept for the inspector until the next one.
    pub last_failure: Option<ReasonCode>,
    /// What the pawn does for a living (an archetype; Blueprint §4.6).
    pub occupation: Option<Occupation>,
    /// Need id -> level, 0 to 1000. A need the pawn has no entry for is added at its starting level by
    /// the needs system.
    pub needs: BTreeMap<String, i32>,
    /// A mood id from the game data (`neutral` until the mood system sets it).
    pub mood: String,
    pub household: Option<EntityId>,
    /// Bounded by the memory parameters; oldest first.
    pub memories: Vec<Memory>,
    /// What the pawn can currently do (Blueprint §8.1); derived each minute by the needs system.
    pub capacities: Capacities,
    /// Where the pawn lives and eats and sleeps. Set by population or, failing that, to the tile the pawn
    /// first stood on.
    pub home_tile: Option<Tile>,
    /// Where the pawn works, if it has a job with a place (set by population).
    pub workplace: Option<Tile>,
    /// The tick since which the pawn has had nothing to do (for the `bored` mood).
    pub idle_since: Option<u64>,
}

/// The mood a pawn has before the mood system has looked at it.
pub const DEFAULT_MOOD: &str = "neutral";

impl Pawn {
    pub fn new(id: EntityId, name: &str, position: Position) -> Pawn {
        Pawn {
            id,
            name: name.to_owned(),
            position,
            facing: Dir4::S,
            route: None,
            schedule: None,
            intent: Intent::Free,
            task: None,
            replan: None,
            move_failure: None,
            last_failure: None,
            occupation: None,
            needs: BTreeMap::new(),
            mood: DEFAULT_MOOD.to_owned(),
            household: None,
            memories: Vec::new(),
            capacities: Capacities::FULL,
            home_tile: None,
            workplace: None,
            idle_since: None,
        }
    }
}

impl ToCanon for Pawn {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("name", self.name.to_canon()),
            ("position", self.position.to_canon()),
            ("facing", Canon::str(self.facing.name())),
            (
                "route",
                self.route.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "schedule",
                self.schedule
                    .as_ref()
                    .map_or(Canon::Null, ToCanon::to_canon),
            ),
            ("intent", self.intent.to_canon()),
            (
                "task",
                self.task.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "replan",
                self.replan.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "move_failure",
                self.move_failure
                    .as_ref()
                    .map_or(Canon::Null, |s| Canon::str(s.clone())),
            ),
            (
                "last_failure",
                self.last_failure
                    .as_ref()
                    .map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "occupation",
                self.occupation
                    .as_ref()
                    .map_or(Canon::Null, ToCanon::to_canon),
            ),
            (
                "needs",
                Canon::Map(
                    self.needs
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_canon()))
                        .collect(),
                ),
            ),
            ("mood", Canon::str(self.mood.clone())),
            (
                "household",
                self.household.map_or(Canon::Null, |h| h.to_canon()),
            ),
            (
                "memories",
                Canon::List(self.memories.iter().map(ToCanon::to_canon).collect()),
            ),
            ("capacities", self.capacities.to_canon()),
            (
                "home_tile",
                self.home_tile.map_or(Canon::Null, |t| t.to_canon()),
            ),
            (
                "workplace",
                self.workplace.map_or(Canon::Null, |t| t.to_canon()),
            ),
            (
                "idle_since",
                self.idle_since.map_or(Canon::Null, |t| t.to_canon()),
            ),
        ])
    }
}

impl Position {
    pub fn from_reader(r: Reader<'_>) -> Result<Position, ReadError> {
        r.only(&["map", "tile"])?;
        Ok(Position {
            map: r.child("map")?.reader().parse()?,
            tile: Tile::from_reader(r.child("tile")?.reader())?,
        })
    }
}

impl Route {
    pub fn from_reader(r: Reader<'_>) -> Result<Route, ReadError> {
        r.only(&[
            "goal",
            "path",
            "next",
            "since_step",
            "waited",
            "repaths",
            "needs_repath",
        ])?;
        let path = r
            .child("path")?
            .reader()
            .list()?
            .iter()
            .map(|t| Tile::from_reader(t.reader()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Route {
            goal: Tile::from_reader(r.child("goal")?.reader())?,
            path,
            next: r.child("next")?.reader().usize()?,
            since_step: r.child("since_step")?.reader().u32()?,
            waited: r.child("waited")?.reader().u32()?,
            repaths: r.child("repaths")?.reader().u32()?,
            needs_repath: r.child("needs_repath")?.reader().bool()?,
        })
    }
}

impl Intent {
    pub fn from_reader(r: Reader<'_>) -> Result<Intent, ReadError> {
        match r.value() {
            Canon::Str(s) if s == "free" => Ok(Intent::Free),
            Canon::Map(_) => {
                r.only(&["reservation"])?;
                Ok(Intent::Reservation(r.child("reservation")?.reader().u32()?))
            }
            _ => Err(r.err("expected \"free\" or {\"reservation\": id}")),
        }
    }
}

impl Step {
    pub fn from_reader(r: Reader<'_>) -> Result<Step, ReadError> {
        if let Some(m) = r.maybe("move_to")? {
            let m = m.reader();
            m.only(&["goal", "within"])?;
            return Ok(Step::MoveTo {
                goal: Tile::from_reader(m.child("goal")?.reader())?,
                within: m.child("within")?.reader().u32()?,
            });
        }
        if let Some(t) = r.maybe("perform_until")? {
            return Ok(Step::PerformUntil(t.reader().u64()?));
        }
        Err(r.err("expected a step: move_to or perform_until"))
    }
}

impl Task {
    pub fn from_reader(r: Reader<'_>) -> Result<Task, ReadError> {
        r.only(&[
            "reservation",
            "action",
            "steps",
            "current",
            "moving",
            "interruptible",
        ])?;
        let steps = r
            .child("steps")?
            .reader()
            .list()?
            .iter()
            .map(|s| Step::from_reader(s.reader()))
            .collect::<Result<Vec<_>, _>>()?;
        let current = r.child("current")?.reader().usize()?;
        if current > steps.len() {
            return Err(r.err("current step is past the end"));
        }
        Ok(Task {
            reservation: r.child("reservation")?.reader().u32()?,
            action: r.child("action")?.reader().parse()?,
            steps,
            current,
            moving: r.child("moving")?.reader().bool()?,
            interruptible: r.child("interruptible")?.reader().bool()?,
        })
    }
}

impl Replan {
    pub fn from_reader(r: Reader<'_>) -> Result<Replan, ReadError> {
        r.only(&["from", "why"])?;
        Ok(Replan {
            from: r.child("from")?.reader().u32()?,
            why: r.child("why")?.reader().str()?.to_owned(),
        })
    }
}

impl Pawn {
    pub fn from_reader(r: Reader<'_>) -> Result<Pawn, ReadError> {
        r.only(&[
            "id",
            "name",
            "position",
            "facing",
            "route",
            "schedule",
            "intent",
            "task",
            "replan",
            "move_failure",
            "last_failure",
            "occupation",
            "needs",
            "mood",
            "household",
            "memories",
            "capacities",
            "home_tile",
            "workplace",
            "idle_since",
        ])?;
        let facing_child = r.child("facing")?;
        let facing = Dir4::from_name(facing_child.reader().str()?)
            .ok_or_else(|| facing_child.reader().err("expected N, E, S or W"))?;
        Ok(Pawn {
            id: r.child("id")?.reader().parse()?,
            name: r.child("name")?.reader().str()?.to_owned(),
            position: Position::from_reader(r.child("position")?.reader())?,
            facing,
            route: match r.maybe("route")? {
                Some(c) => Some(Route::from_reader(c.reader())?),
                None => None,
            },
            schedule: match r.maybe("schedule")? {
                Some(c) => Some(DaySchedule::from_reader(c.reader())?),
                None => None,
            },
            intent: Intent::from_reader(r.child("intent")?.reader())?,
            task: match r.maybe("task")? {
                Some(c) => Some(Task::from_reader(c.reader())?),
                None => None,
            },
            replan: match r.maybe("replan")? {
                Some(c) => Some(Replan::from_reader(c.reader())?),
                None => None,
            },
            move_failure: match r.maybe("move_failure")? {
                Some(c) => Some(c.reader().str()?.to_owned()),
                None => None,
            },
            last_failure: match r.maybe("last_failure")? {
                Some(c) => Some(ReasonCode::from_reader(c.reader())?),
                None => None,
            },
            occupation: match r.maybe("occupation")? {
                Some(c) => Some(Occupation::from_reader(c.reader())?),
                None => None,
            },
            needs: {
                let mut needs = BTreeMap::new();
                for (k, c) in r.child("needs")?.reader().entries()? {
                    let v = c.reader().i32()?;
                    if !(0..=1000).contains(&v) {
                        return Err(c.reader().err("a need level is 0 to 1000"));
                    }
                    needs.insert(k, v);
                }
                needs
            },
            mood: r.child("mood")?.reader().str()?.to_owned(),
            household: match r.maybe("household")? {
                Some(c) => Some(c.reader().parse()?),
                None => None,
            },
            memories: r
                .child("memories")?
                .reader()
                .list()?
                .iter()
                .map(|c| Memory::from_reader(c.reader()))
                .collect::<Result<Vec<_>, _>>()?,
            capacities: Capacities::from_reader(r.child("capacities")?.reader())?,
            home_tile: match r.maybe("home_tile")? {
                Some(c) => Some(Tile::from_reader(c.reader())?),
                None => None,
            },
            workplace: match r.maybe("workplace")? {
                Some(c) => Some(Tile::from_reader(c.reader())?),
                None => None,
            },
            idle_since: match r.maybe("idle_since")? {
                Some(c) => Some(c.reader().u64()?),
                None => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    #[test]
    fn a_new_route_wants_a_solve_and_has_nothing_remaining() {
        let r = Route::to(Tile::new(3, 4));
        assert!(r.needs_repath && r.path.is_empty() && r.remaining().is_empty());
    }

    #[test]
    fn remaining_follows_the_cursor() {
        let mut r = Route::to(Tile::new(2, 0));
        r.path = vec![Tile::new(1, 0), Tile::new(2, 0)];
        assert_eq!(r.remaining().len(), 2);
        r.next = 1;
        assert_eq!(r.remaining(), &[Tile::new(2, 0)]);
        r.next = 2;
        assert!(r.remaining().is_empty());
        r.next = 99;
        assert!(
            r.remaining().is_empty(),
            "an out-of-range cursor is not a panic"
        );
    }

    #[test]
    fn canonical_form_includes_the_route() {
        let p = Pawn::new(
            EntityId::new(Kind::Pawn, 1),
            "Ann",
            Position {
                map: EntityId::new(Kind::Map, 1),
                tile: Tile::new(1, 2),
            },
        );
        assert_eq!(
            p.to_canon().to_canonical_string(),
            r#"{"capacities":{"breathing":1000,"consciousness":1000,"eating":1000,"manipulation":1000,"moving":1000,"talking":1000},"facing":"S","home_tile":null,"household":null,"id":"pawn_1","idle_since":null,"intent":"free","last_failure":null,"memories":[],"mood":"neutral","move_failure":null,"name":"Ann","needs":{},"occupation":null,"position":{"map":"map_1","tile":[1,2]},"replan":null,"route":null,"schedule":null,"task":null,"workplace":null}"#
        );
    }
}
