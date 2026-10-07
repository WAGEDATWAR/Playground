//! Maps (Blueprint §7.1): flat, row-major tile arrays plus movement costs.
//!
//! Tile arrays are indexed `y * w + x`. Every edit bumps `edit_version`, which keys the path cache, and
//! marks the touched 16×16 chunk dirty for the renderer. Neither the version nor the dirty set is part
//! of the authoritative (hashed) state: they are derived bookkeeping.

use crate::canon::{Canon, ToCanon};
use crate::read::{ReadError, Reader};
use crate::id::EntityId;
use std::collections::BTreeSet;
use std::fmt;

pub const CHUNK_SIZE: i32 = 16;
/// Largest side length; `w * h` is also capped so a map fits comfortably in memory.
pub const MAX_DIM: i32 = 2048;
pub const MAX_TILES: usize = 1 << 21;

// Terrain type ids.
pub const GRASS: u8 = 0;
pub const WATER: u8 = 1;
pub const SAND: u8 = 2;
// Surface type ids.
pub const SURFACE_NONE: u8 = 0;
pub const ROAD: u8 = 1;
pub const SIDEWALK: u8 = 2;
pub const FLOOR: u8 = 3;

/// A tile coordinate. Ordering is `(x, y)` and is only used for tie-breaks that are always combined
/// with a row-major index where order matters.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
}

impl Tile {
    pub const fn new(x: i32, y: i32) -> Tile {
        Tile { x, y }
    }

    /// Manhattan distance, saturating.
    pub fn manhattan(self, other: Tile) -> u32 {
        let dx = (i64::from(self.x) - i64::from(other.x)).unsigned_abs();
        let dy = (i64::from(self.y) - i64::from(other.y)).unsigned_abs();
        u32::try_from(dx.saturating_add(dy)).unwrap_or(u32::MAX)
    }
}

impl fmt::Display for Tile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.x, self.y)
    }
}

impl ToCanon for Tile {
    fn to_canon(&self) -> Canon {
        Canon::List(vec![self.x.to_canon(), self.y.to_canon()])
    }
}

impl Tile {
    pub fn from_reader(r: Reader<'_>) -> Result<Tile, ReadError> {
        Tile::from_canon(r.value()).ok_or_else(|| r.err("expected a tile [x, y] of 32-bit integers"))
    }

    pub fn from_canon(c: &Canon) -> Option<Tile> {
        let l = c.as_list()?;
        match l {
            [x, y] => Some(Tile {
                x: i32::try_from(x.as_i64()?).ok()?,
                y: i32::try_from(y.as_i64()?).ok()?,
            }),
            _ => None,
        }
    }
}

/// The four directions in their fixed neighbor-expansion order: N, E, S, W.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dir4 {
    N,
    E,
    S,
    W,
}

impl Dir4 {
    /// Inverse of [`Dir4::name`].
    pub fn from_name(name: &str) -> Option<Dir4> {
        Dir4::ALL.into_iter().find(|d| d.name() == name)
    }

    pub const ALL: [Dir4; 4] = [Dir4::N, Dir4::E, Dir4::S, Dir4::W];

    pub const fn delta(self) -> (i32, i32) {
        match self {
            Dir4::N => (0, -1),
            Dir4::E => (1, 0),
            Dir4::S => (0, 1),
            Dir4::W => (-1, 0),
        }
    }

    pub fn step(self, t: Tile) -> Tile {
        let (dx, dy) = self.delta();
        Tile {
            x: t.x.saturating_add(dx),
            y: t.y.saturating_add(dy),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Dir4::N => "N",
            Dir4::E => "E",
            Dir4::S => "S",
            Dir4::W => "W",
        }
    }

    /// The direction of a single step from `a` to the adjacent tile `b`, if they are adjacent.
    pub fn between(a: Tile, b: Tile) -> Option<Dir4> {
        Dir4::ALL.into_iter().find(|d| d.step(a) == b)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MapKind {
    Overworld,
    Interior,
}

impl MapKind {
    pub const fn name(self) -> &'static str {
        match self {
            MapKind::Overworld => "overworld",
            MapKind::Interior => "interior",
        }
    }
}

/// An entrance linking two maps (used from Stage 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portal {
    pub id: String,
    pub tile: Tile,
    pub to_map: EntityId,
    pub to_tile: Tile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapError {
    BadSize { w: i32, h: i32 },
    OutOfBounds(Tile),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::BadSize { w, h } => {
                write!(f, "map size {w}x{h} is invalid (1..={MAX_DIM} per side, at most {MAX_TILES} tiles)")
            }
            MapError::OutOfBounds(t) => write!(f, "tile {t} is outside the map"),
        }
    }
}

impl std::error::Error for MapError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapData {
    pub id: EntityId,
    pub kind: MapKind,
    w: i32,
    h: i32,
    terrain: Vec<u8>,
    surface: Vec<u8>,
    blocked: Vec<u8>,
    zone_id: Vec<u16>,
    pub portals: Vec<Portal>,
    /// Derived: bumped on every edit; keys the path cache. Not hashed.
    edit_version: u64,
    /// Derived: chunks edited since the renderer last looked. Not hashed.
    dirty: BTreeSet<(i32, i32)>,
}

impl MapData {
    pub fn new(id: EntityId, kind: MapKind, w: i32, h: i32) -> Result<MapData, MapError> {
        let tiles = usize::try_from(i64::from(w) * i64::from(h)).unwrap_or(usize::MAX);
        if w < 1 || h < 1 || w > MAX_DIM || h > MAX_DIM || tiles > MAX_TILES {
            return Err(MapError::BadSize { w, h });
        }
        Ok(MapData {
            id,
            kind,
            w,
            h,
            terrain: vec![GRASS; tiles],
            surface: vec![SURFACE_NONE; tiles],
            blocked: vec![0; tiles],
            zone_id: vec![0; tiles],
            portals: Vec::new(),
            edit_version: 0,
            dirty: BTreeSet::new(),
        })
    }

    pub const fn width(&self) -> i32 {
        self.w
    }

    pub const fn height(&self) -> i32 {
        self.h
    }

    pub fn tile_count(&self) -> usize {
        self.terrain.len()
    }

    pub fn in_bounds(&self, t: Tile) -> bool {
        t.x >= 0 && t.y >= 0 && t.x < self.w && t.y < self.h
    }

    /// Row-major index `y * w + x`.
    pub fn index(&self, t: Tile) -> Option<usize> {
        if self.in_bounds(t) {
            usize::try_from(i64::from(t.y) * i64::from(self.w) + i64::from(t.x)).ok()
        } else {
            None
        }
    }

    pub fn tile_of(&self, index: usize) -> Option<Tile> {
        let w = usize::try_from(self.w).ok()?;
        if index >= self.tile_count() || w == 0 {
            return None;
        }
        Some(Tile {
            x: i32::try_from(index % w).ok()?,
            y: i32::try_from(index / w).ok()?,
        })
    }

    pub fn terrain_at(&self, t: Tile) -> Option<u8> {
        self.index(t).and_then(|i| self.terrain.get(i)).copied()
    }

    pub fn surface_at(&self, t: Tile) -> Option<u8> {
        self.index(t).and_then(|i| self.surface.get(i)).copied()
    }

    pub fn is_blocked(&self, t: Tile) -> bool {
        self.index(t)
            .and_then(|i| self.blocked.get(i))
            .is_none_or(|b| *b != 0)
    }

    pub fn zone_at(&self, t: Tile) -> Option<u16> {
        self.index(t).and_then(|i| self.zone_id.get(i)).copied()
    }

    pub fn edit_version(&self) -> u64 {
        self.edit_version
    }

    fn touch(&mut self, t: Tile) {
        self.edit_version = self.edit_version.wrapping_add(1);
        self.dirty
            .insert((t.x.div_euclid(CHUNK_SIZE), t.y.div_euclid(CHUNK_SIZE)));
    }

    pub fn set_terrain(&mut self, t: Tile, value: u8) -> Result<(), MapError> {
        let i = self.index(t).ok_or(MapError::OutOfBounds(t))?;
        if let Some(slot) = self.terrain.get_mut(i) {
            *slot = value;
        }
        self.touch(t);
        Ok(())
    }

    pub fn set_surface(&mut self, t: Tile, value: u8) -> Result<(), MapError> {
        let i = self.index(t).ok_or(MapError::OutOfBounds(t))?;
        if let Some(slot) = self.surface.get_mut(i) {
            *slot = value;
        }
        self.touch(t);
        Ok(())
    }

    pub fn set_blocked(&mut self, t: Tile, blocked: bool) -> Result<(), MapError> {
        let i = self.index(t).ok_or(MapError::OutOfBounds(t))?;
        if let Some(slot) = self.blocked.get_mut(i) {
            *slot = u8::from(blocked);
        }
        self.touch(t);
        Ok(())
    }

    pub fn set_zone(&mut self, t: Tile, zone: u16) -> Result<(), MapError> {
        let i = self.index(t).ok_or(MapError::OutOfBounds(t))?;
        if let Some(slot) = self.zone_id.get_mut(i) {
            *slot = zone;
        }
        self.touch(t);
        Ok(())
    }

    /// Chunks edited since the last call (ascending), for the renderer's cache.
    pub fn take_dirty_chunks(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }
}

fn hex(bytes: impl Iterator<Item = u8>) -> String {
    let mut s = String::new();
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        s.push(char::from_digit(u32::from(b & 0xF), 16).unwrap_or('0'));
    }
    s
}

impl ToCanon for MapData {
    /// Tile arrays are written as hex strings. `edit_version` and the dirty set are derived and left out.
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("kind", Canon::str(self.kind.name())),
            ("w", self.w.to_canon()),
            ("h", self.h.to_canon()),
            ("terrain", Canon::Str(hex(self.terrain.iter().copied()))),
            ("surface", Canon::Str(hex(self.surface.iter().copied()))),
            ("blocked", Canon::Str(hex(self.blocked.iter().copied()))),
            (
                "zone_id",
                Canon::Str(hex(self.zone_id.iter().flat_map(|z| z.to_le_bytes()))),
            ),
            (
                "portals",
                Canon::List(
                    self.portals
                        .iter()
                        .map(|p| {
                            Canon::map([
                                ("id", Canon::str(p.id.clone())),
                                ("tile", p.tile.to_canon()),
                                ("to_map", p.to_map.to_canon()),
                                ("to_tile", p.to_tile.to_canon()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// Movement costs per terrain and surface type (Blueprint §7.3). A surface other than `SURFACE_NONE`
/// overrides the terrain (a road over water is a bridge). A cost of 0 means impassable; unknown type
/// ids are impassable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveCosts {
    terrain: [u16; 8],
    surface: [u16; 8],
}

impl Default for MoveCosts {
    fn default() -> MoveCosts {
        fn fill(table: &mut [u16; 8], entries: &[(u8, u16)]) {
            for (id, cost) in entries {
                if let Some(slot) = table.get_mut(usize::from(*id)) {
                    *slot = *cost;
                }
            }
        }
        let mut terrain = [0u16; 8];
        fill(&mut terrain, &[(GRASS, 10), (SAND, 14)]); // WATER stays 0: impassable
        let mut surface = [0u16; 8];
        fill(&mut surface, &[(ROAD, 6), (SIDEWALK, 7), (FLOOR, 8)]);
        MoveCosts { terrain, surface }
    }
}

impl MoveCosts {
    /// The cost of entering `t`, or `None` if it is out of bounds, blocked or impassable.
    pub fn step_cost(&self, map: &MapData, t: Tile) -> Option<u32> {
        if map.is_blocked(t) {
            return None;
        }
        let surface = map.surface_at(t)?;
        let cost = if surface != SURFACE_NONE {
            self.surface.get(usize::from(surface)).copied().unwrap_or(0)
        } else {
            self.terrain
                .get(usize::from(map.terrain_at(t)?))
                .copied()
                .unwrap_or(0)
        };
        (cost > 0).then_some(u32::from(cost))
    }

    /// The cheapest possible step: the heuristic's multiplier, so A\* stays admissible.
    pub fn min_step_cost(&self) -> u32 {
        self.terrain
            .iter()
            .chain(self.surface.iter())
            .copied()
            .filter(|c| *c > 0)
            .min()
            .map_or(1, u32::from)
    }
}

fn unhex(r: Reader<'_>, bytes: usize) -> Result<Vec<u8>, ReadError> {
    let text = r.str()?;
    if text.len() != bytes * 2 {
        return Err(r.err(format!(
            "expected {} hex characters, found {}",
            bytes * 2,
            text.len()
        )));
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| match pair {
            [a, b] => {
                let hi = char::from(*a).to_digit(16);
                let lo = char::from(*b).to_digit(16);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => Ok((hi * 16 + lo) as u8),
                    _ => Err(r.err("not a hex string")),
                }
            }
            _ => Err(r.err("odd hex length")),
        })
        .collect()
}

impl MapData {
    /// Decodes the canonical form. Sizes are checked before any tile array is allocated, so a hostile
    /// save cannot ask for a huge map.
    pub fn from_reader(r: Reader<'_>) -> Result<MapData, ReadError> {
        r.only(&[
            "id", "kind", "w", "h", "terrain", "surface", "blocked", "zone_id", "portals",
        ])?;
        let id: EntityId = r.child("id")?.reader().parse()?;
        let kind_child = r.child("kind")?;
        let kind = match kind_child.reader().str()? {
            "overworld" => MapKind::Overworld,
            "interior" => MapKind::Interior,
            other => return Err(kind_child.reader().err(format!("unknown map kind '{other}'"))),
        };
        let (w, h) = (r.child("w")?.reader().i32()?, r.child("h")?.reader().i32()?);
        let mut m = MapData::new(id, kind, w, h).map_err(|e| r.err(e.to_string()))?;
        let n = m.terrain.len();
        m.terrain = unhex(r.child("terrain")?.reader(), n)?;
        m.surface = unhex(r.child("surface")?.reader(), n)?;
        m.blocked = unhex(r.child("blocked")?.reader(), n)?;
        let zones = unhex(r.child("zone_id")?.reader(), n * 2)?;
        m.zone_id = zones
            .chunks(2)
            .map(|c| match c {
                [a, b] => u16::from_le_bytes([*a, *b]),
                _ => 0,
            })
            .collect();
        for item in r.child("portals")?.reader().list()? {
            let p = item.reader();
            p.only(&["id", "tile", "to_map", "to_tile"])?;
            m.portals.push(Portal {
                id: p.child("id")?.reader().str()?.to_owned(),
                tile: Tile::from_reader(p.child("tile")?.reader())?,
                to_map: p.child("to_map")?.reader().parse()?,
                to_tile: Tile::from_reader(p.child("to_tile")?.reader())?,
            });
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    fn map(w: i32, h: i32) -> MapData {
        MapData::new(EntityId::new(Kind::Map, 1), MapKind::Overworld, w, h).unwrap()
    }

    #[test]
    fn sizes_are_validated() {
        let id = EntityId::new(Kind::Map, 1);
        assert!(MapData::new(id, MapKind::Overworld, 1, 1).is_ok());
        assert!(
            MapData::new(id, MapKind::Overworld, MAX_DIM, MAX_DIM / 2).is_ok(),
            "exactly the tile cap is allowed"
        );
        for (w, h) in [
            (0, 5),
            (5, 0),
            (-1, 5),
            (MAX_DIM + 1, 1),
            (MAX_DIM, MAX_DIM),
        ] {
            assert!(
                MapData::new(id, MapKind::Overworld, w, h).is_err(),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn indexing_is_row_major_and_round_trips() {
        let m = map(5, 3);
        assert_eq!(m.index(Tile::new(0, 0)), Some(0));
        assert_eq!(m.index(Tile::new(4, 0)), Some(4));
        assert_eq!(m.index(Tile::new(0, 1)), Some(5));
        assert_eq!(m.index(Tile::new(4, 2)), Some(14));
        assert_eq!(m.index(Tile::new(5, 0)), None);
        assert_eq!(m.index(Tile::new(-1, 0)), None);
        assert_eq!(m.index(Tile::new(0, 3)), None);
        for i in 0..m.tile_count() {
            assert_eq!(m.index(m.tile_of(i).unwrap()), Some(i));
        }
        assert_eq!(m.tile_of(15), None);
    }

    #[test]
    fn edits_bump_the_version_and_mark_chunks_dirty() {
        let mut m = map(40, 40);
        assert_eq!(m.edit_version(), 0);
        m.set_blocked(Tile::new(3, 3), true).unwrap();
        m.set_terrain(Tile::new(20, 3), WATER).unwrap();
        m.set_surface(Tile::new(39, 39), ROAD).unwrap();
        assert_eq!(m.edit_version(), 3);
        assert_eq!(m.take_dirty_chunks(), vec![(0, 0), (1, 0), (2, 2)]);
        assert!(m.take_dirty_chunks().is_empty(), "taking clears the set");
        assert_eq!(
            m.set_blocked(Tile::new(40, 0), true),
            Err(MapError::OutOfBounds(Tile::new(40, 0)))
        );
        assert_eq!(m.edit_version(), 3, "a failed edit changes nothing");
    }

    #[test]
    fn out_of_bounds_reads_are_blocked_not_panics() {
        let m = map(4, 4);
        assert!(m.is_blocked(Tile::new(-1, 0)) && m.is_blocked(Tile::new(4, 4)));
        assert_eq!(m.terrain_at(Tile::new(9, 9)), None);
    }

    #[test]
    fn costs_follow_terrain_surface_and_blocking() {
        let mut m = map(4, 4);
        let c = MoveCosts::default();
        let t = Tile::new(1, 1);
        assert_eq!(c.step_cost(&m, t), Some(10), "grass");
        m.set_terrain(t, SAND).unwrap();
        assert_eq!(c.step_cost(&m, t), Some(14));
        m.set_surface(t, ROAD).unwrap();
        assert_eq!(
            c.step_cost(&m, t),
            Some(6),
            "a road is cheaper than the terrain under it"
        );
        m.set_terrain(t, WATER).unwrap();
        assert_eq!(c.step_cost(&m, t), Some(6), "a road over water is a bridge");
        m.set_surface(t, SURFACE_NONE).unwrap();
        assert_eq!(c.step_cost(&m, t), None, "water is impassable");
        m.set_terrain(t, GRASS).unwrap();
        m.set_blocked(t, true).unwrap();
        assert_eq!(c.step_cost(&m, t), None, "blocked");
        m.set_blocked(t, false).unwrap();
        m.set_terrain(t, 7).unwrap();
        assert_eq!(
            c.step_cost(&m, t),
            None,
            "unknown terrain ids are impassable"
        );
        assert_eq!(c.step_cost(&m, Tile::new(9, 9)), None);
        assert_eq!(c.min_step_cost(), 6);
    }

    #[test]
    fn canonical_form_ignores_derived_state() {
        let mut a = map(4, 4);
        let mut b = map(4, 4);
        a.set_blocked(Tile::new(1, 1), true).unwrap();
        a.set_blocked(Tile::new(1, 1), false).unwrap(); // edited and restored
        let _ = b.take_dirty_chunks();
        assert_ne!(a.edit_version(), b.edit_version());
        assert_eq!(
            a.to_canon(),
            b.to_canon(),
            "edit history is not authoritative state"
        );
        b.set_blocked(Tile::new(2, 2), true).unwrap();
        assert_ne!(a.to_canon(), b.to_canon());
    }

    #[test]
    fn directions_have_a_fixed_order_and_deltas() {
        assert_eq!(Dir4::ALL, [Dir4::N, Dir4::E, Dir4::S, Dir4::W]);
        let t = Tile::new(5, 5);
        assert_eq!(Dir4::N.step(t), Tile::new(5, 4));
        assert_eq!(Dir4::E.step(t), Tile::new(6, 5));
        assert_eq!(Dir4::S.step(t), Tile::new(5, 6));
        assert_eq!(Dir4::W.step(t), Tile::new(4, 5));
        assert_eq!(Dir4::between(t, Tile::new(6, 5)), Some(Dir4::E));
        assert_eq!(Dir4::between(t, Tile::new(7, 5)), None);
        assert_eq!(Dir4::between(t, Tile::new(6, 6)), None);
        assert_eq!(Tile::new(0, 0).manhattan(Tile::new(3, -4)), 7);
    }
}
