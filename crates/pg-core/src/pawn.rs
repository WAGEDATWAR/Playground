//! Pawns, spatial fields only (Blueprint §4.6). Needs, mood, memory and the rest arrive in Stage 1;
//! milestone 0.4 needs position, facing and the route a pawn is following.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::map::{Dir4, Tile};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pawn {
    pub id: EntityId,
    pub name: String,
    pub position: Position,
    pub facing: Dir4,
    pub route: Option<Route>,
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
        ])
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
        let p = Pawn {
            id: EntityId::new(Kind::Pawn, 1),
            name: "Ann".into(),
            position: Position {
                map: EntityId::new(Kind::Map, 1),
                tile: Tile::new(1, 2),
            },
            facing: Dir4::S,
            route: None,
        };
        assert_eq!(
            p.to_canon().to_canonical_string(),
            r#"{"facing":"S","id":"pawn_1","name":"Ann","position":{"map":"map_1","tile":[1,2]},"route":null}"#
        );
    }
}
