//! Occupancy (Blueprint §7.2): which pawn stands on which tile, per map.
//!
//! This is **derived** state: it is rebuilt from the pawn table on load and maintained by the movement
//! system, and is never hashed or saved. One pawn per tile for walking.

use crate::id::EntityId;
use crate::map::{MapData, Tile};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OccupancyError {
    UnknownMap(EntityId),
    OutOfBounds(Tile),
    Occupied { tile: Tile, by: EntityId },
}

impl fmt::Display for OccupancyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OccupancyError::UnknownMap(m) => write!(f, "no map {m}"),
            OccupancyError::OutOfBounds(t) => write!(f, "tile {t} is outside the map"),
            OccupancyError::Occupied { tile, by } => write!(f, "tile {tile} is occupied by {by}"),
        }
    }
}

impl std::error::Error for OccupancyError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Occupancy {
    grids: BTreeMap<EntityId, Grid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Grid {
    w: i32,
    h: i32,
    cells: Vec<Option<EntityId>>,
}

impl Grid {
    fn index(&self, t: Tile) -> Option<usize> {
        if t.x < 0 || t.y < 0 || t.x >= self.w || t.y >= self.h {
            return None;
        }
        usize::try_from(i64::from(t.y) * i64::from(self.w) + i64::from(t.x)).ok()
    }
}

impl Occupancy {
    pub fn new() -> Occupancy {
        Occupancy::default()
    }

    /// Adds an empty grid for a map (call when a map is created).
    pub fn add_map(&mut self, map: &MapData) {
        self.grids.insert(
            map.id,
            Grid {
                w: map.width(),
                h: map.height(),
                cells: vec![None; map.tile_count()],
            },
        );
    }

    pub fn remove_map(&mut self, map: EntityId) {
        self.grids.remove(&map);
    }

    /// Who stands on `tile`, if anyone. Out-of-bounds and unknown maps read as empty.
    pub fn occupant(&self, map: EntityId, tile: Tile) -> Option<EntityId> {
        let g = self.grids.get(&map)?;
        g.index(tile)
            .and_then(|i| g.cells.get(i))
            .copied()
            .flatten()
    }

    /// Puts `pawn` on `tile`. Fails (changing nothing) if the tile is outside the map or taken.
    pub fn place(
        &mut self,
        map: EntityId,
        tile: Tile,
        pawn: EntityId,
    ) -> Result<(), OccupancyError> {
        let g = self
            .grids
            .get_mut(&map)
            .ok_or(OccupancyError::UnknownMap(map))?;
        let i = g.index(tile).ok_or(OccupancyError::OutOfBounds(tile))?;
        match g.cells.get_mut(i) {
            Some(slot) => match *slot {
                Some(by) if by != pawn => Err(OccupancyError::Occupied { tile, by }),
                _ => {
                    *slot = Some(pawn);
                    Ok(())
                }
            },
            None => Err(OccupancyError::OutOfBounds(tile)),
        }
    }

    /// Clears `tile` if `pawn` is the one standing there.
    pub fn vacate(&mut self, map: EntityId, tile: Tile, pawn: EntityId) {
        if let Some(g) = self.grids.get_mut(&map) {
            if let Some(slot) = g.index(tile).and_then(|i| g.cells.get_mut(i)) {
                if *slot == Some(pawn) {
                    *slot = None;
                }
            }
        }
    }

    /// Every occupied tile of `map` with its occupant, in row-major order.
    pub fn occupied(&self, map: EntityId) -> Vec<(Tile, EntityId)> {
        let Some(g) = self.grids.get(&map) else {
            return Vec::new();
        };
        let w = usize::try_from(g.w).unwrap_or(1).max(1);
        g.cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let pawn = (*c)?;
                Some((
                    Tile::new(i32::try_from(i % w).ok()?, i32::try_from(i / w).ok()?),
                    pawn,
                ))
            })
            .collect()
    }

    /// All occupied cells across all maps (for consistency checks).
    pub fn total_occupied(&self) -> usize {
        self.grids
            .values()
            .map(|g| g.cells.iter().filter(|c| c.is_some()).count())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;
    use crate::map::MapKind;

    fn setup() -> (Occupancy, EntityId) {
        let id = EntityId::new(Kind::Map, 1);
        let map = MapData::new(id, MapKind::Overworld, 4, 3).unwrap();
        let mut o = Occupancy::new();
        o.add_map(&map);
        (o, id)
    }

    fn pawn(n: u32) -> EntityId {
        EntityId::new(Kind::Pawn, n)
    }

    #[test]
    fn place_vacate_and_query() {
        let (mut o, m) = setup();
        assert_eq!(o.occupant(m, Tile::new(1, 1)), None);
        o.place(m, Tile::new(1, 1), pawn(1)).unwrap();
        assert_eq!(o.occupant(m, Tile::new(1, 1)), Some(pawn(1)));
        o.vacate(m, Tile::new(1, 1), pawn(1));
        assert_eq!(o.occupant(m, Tile::new(1, 1)), None);
    }

    #[test]
    fn one_pawn_per_tile() {
        let (mut o, m) = setup();
        o.place(m, Tile::new(2, 2), pawn(1)).unwrap();
        assert_eq!(
            o.place(m, Tile::new(2, 2), pawn(2)),
            Err(OccupancyError::Occupied {
                tile: Tile::new(2, 2),
                by: pawn(1)
            })
        );
        assert_eq!(
            o.occupant(m, Tile::new(2, 2)),
            Some(pawn(1)),
            "the failed place changed nothing"
        );
        assert!(
            o.place(m, Tile::new(2, 2), pawn(1)).is_ok(),
            "re-placing the same pawn is idempotent"
        );
    }

    #[test]
    fn vacate_only_clears_for_the_right_pawn() {
        let (mut o, m) = setup();
        o.place(m, Tile::new(0, 0), pawn(1)).unwrap();
        o.vacate(m, Tile::new(0, 0), pawn(2));
        assert_eq!(o.occupant(m, Tile::new(0, 0)), Some(pawn(1)));
    }

    #[test]
    fn bad_maps_and_tiles_are_errors_not_panics() {
        let (mut o, m) = setup();
        assert_eq!(
            o.place(m, Tile::new(4, 0), pawn(1)),
            Err(OccupancyError::OutOfBounds(Tile::new(4, 0)))
        );
        assert!(matches!(
            o.place(EntityId::new(Kind::Map, 9), Tile::new(0, 0), pawn(1)),
            Err(OccupancyError::UnknownMap(_))
        ));
        assert_eq!(
            o.occupant(EntityId::new(Kind::Map, 9), Tile::new(0, 0)),
            None
        );
        assert_eq!(o.occupant(m, Tile::new(-1, 0)), None);
        o.vacate(m, Tile::new(99, 99), pawn(1));
    }

    #[test]
    fn occupied_lists_row_major() {
        let (mut o, m) = setup();
        o.place(m, Tile::new(3, 2), pawn(2)).unwrap();
        o.place(m, Tile::new(1, 0), pawn(1)).unwrap();
        o.place(m, Tile::new(0, 2), pawn(3)).unwrap();
        assert_eq!(
            o.occupied(m),
            vec![
                (Tile::new(1, 0), pawn(1)),
                (Tile::new(0, 2), pawn(3)),
                (Tile::new(3, 2), pawn(2))
            ]
        );
        assert_eq!(o.total_occupied(), 3);
    }
}
