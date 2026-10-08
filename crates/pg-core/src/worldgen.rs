//! The town generator (Stage 1, milestone 1.2; Blueprint §12.1 to §12.4).
//!
//! `(seed, parameters, game data)` always gives the same town. The pipeline, each step drawing from its own
//! named stream with the attempt number mixed in:
//!
//! 1. **Terrain:** integer value noise, a water threshold chosen so the requested share of the map is water,
//!    a sandy shore, and only the largest landmass kept.
//! 2. **Districts:** centres with a minimum spacing; the one nearest the middle is the commercial core, then
//!    a civic district and a park, the rest residential. Every land tile belongs to its nearest district.
//! 3. **Main roads:** the districts joined by a spanning tree and a few loops, routed with A* over jittered
//!    costs so they bend, with sidewalks beside them.
//! 4. **Local blocks:** a street grid inside each district.
//! 5. **Plots and buildings:** lots cut from the blocks, a building footprint and entrance in each lot, a role
//!    from the district's weights. Footprints are blocked tiles (exteriors only).
//! 6. **Gathering places:** a plaza at the middle of the commercial and park districts.
//! 7. **People:** the population plan (households, occupations, relationships), homes for the households
//!    and a workplace for residents whose occupation has duties, each resident on a tile of their own in the
//!    yard of the building.
//! 8. **Validation:** everything is reachable from the main plaza and nothing overlaps. If any step fails the
//!    next attempt mixes `attempt + 1` into every stream, up to [`MAX_ATTEMPTS`].

use crate::id::EntityId;
use crate::map::{MapKind, Tile, FLOOR, GRASS, ROAD, SAND, SIDEWALK, SURFACE_NONE, WATER};
use crate::population::{apply_population, plan_population, PlacePlan, PlanError, PopulationPlan};
use crate::rng::{Key, Rng, Seed, Stream};
use crate::town::{Building, BuildingRole, District, DistrictKind, Town};
use crate::world::{TonePreset, WorldError, WorldState};
use pg_content::gamedata::{GameData, WorldgenParams};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, VecDeque};
use std::fmt;

/// Attempts before generation gives up.
pub const MAX_ATTEMPTS: u32 = 8;

/// Map sizes the generator accepts (tiles).
pub const MIN_SIDE: i32 = 24;
pub const MAX_SIDE: i32 = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenParams {
    pub width: i32,
    pub height: i32,
    /// Share of the map that is water, percent (limited by the game data).
    pub water_percent: u32,
    pub residents: usize,
    pub tone: TonePreset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenError {
    /// The game data has no town generator parameters, or no occupations or names.
    MissingData(&'static str),
    BadParams(String),
    Plan(PlanError),
    World(WorldError),
    /// Every attempt failed validation; the reason of the last one.
    Failed {
        attempts: u32,
        reason: String,
    },
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenError::MissingData(what) => write!(f, "the game data has no {what}"),
            GenError::BadParams(m) => f.write_str(m),
            GenError::Plan(e) => e.fmt(f),
            GenError::World(e) => e.fmt(f),
            GenError::Failed { attempts, reason } => {
                write!(f, "no valid town in {attempts} attempt(s): {reason}")
            }
        }
    }
}

impl std::error::Error for GenError {}

impl From<WorldError> for GenError {
    fn from(e: WorldError) -> Self {
        GenError::World(e)
    }
}

impl From<PlanError> for GenError {
    fn from(e: PlanError) -> Self {
        GenError::Plan(e)
    }
}

/// What a successful generation produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenOutcome {
    pub map: EntityId,
    pub attempt: u32,
    pub districts: usize,
    pub buildings: usize,
    pub residents: usize,
    /// Hex state hash of the world as generated (also stored in the town).
    pub starting_hash: String,
}

// ---- the grid the generator works on -----------------------------------------------------------------------

#[derive(Clone)]
struct Grid {
    w: i32,
    h: i32,
    terrain: Vec<u8>,
    surface: Vec<u8>,
    blocked: Vec<bool>,
    /// District index + 1, or 0 for none.
    zone: Vec<u16>,
}

impl Grid {
    fn new(w: i32, h: i32) -> Grid {
        let n = usize::try_from(w).unwrap_or(0) * usize::try_from(h).unwrap_or(0);
        Grid {
            w,
            h,
            terrain: vec![GRASS; n],
            surface: vec![SURFACE_NONE; n],
            blocked: vec![false; n],
            zone: vec![0; n],
        }
    }

    fn in_bounds(&self, t: Tile) -> bool {
        t.x >= 0 && t.y >= 0 && t.x < self.w && t.y < self.h
    }

    fn idx(&self, t: Tile) -> Option<usize> {
        self.in_bounds(t)
            .then(|| usize::try_from(t.y * self.w + t.x).ok())
            .flatten()
    }

    fn terrain_at(&self, t: Tile) -> u8 {
        self.idx(t)
            .and_then(|i| self.terrain.get(i).copied())
            .unwrap_or(WATER)
    }

    fn surface_at(&self, t: Tile) -> u8 {
        self.idx(t)
            .and_then(|i| self.surface.get(i).copied())
            .unwrap_or(SURFACE_NONE)
    }

    fn blocked_at(&self, t: Tile) -> bool {
        self.idx(t)
            .and_then(|i| self.blocked.get(i).copied())
            .unwrap_or(true)
    }

    fn zone_at(&self, t: Tile) -> u16 {
        self.idx(t)
            .and_then(|i| self.zone.get(i).copied())
            .unwrap_or(0)
    }

    fn land(&self, t: Tile) -> bool {
        self.in_bounds(t) && self.terrain_at(t) != WATER
    }

    /// Whether a pawn could stand on the tile.
    fn passable(&self, t: Tile) -> bool {
        self.land(t) && !self.blocked_at(t)
    }

    fn set_terrain(&mut self, t: Tile, v: u8) {
        if let Some(s) = self.idx(t).and_then(|i| self.terrain.get_mut(i)) {
            *s = v;
        }
    }

    fn set_surface(&mut self, t: Tile, v: u8) {
        if let Some(s) = self.idx(t).and_then(|i| self.surface.get_mut(i)) {
            *s = v;
        }
    }

    fn set_blocked(&mut self, t: Tile, v: bool) {
        if let Some(s) = self.idx(t).and_then(|i| self.blocked.get_mut(i)) {
            *s = v;
        }
    }

    fn set_zone(&mut self, t: Tile, v: u16) {
        if let Some(s) = self.idx(t).and_then(|i| self.zone.get_mut(i)) {
            *s = v;
        }
    }

    fn tiles(&self) -> impl Iterator<Item = Tile> + '_ {
        let w = self.w;
        (0..self.h).flat_map(move |y| (0..w).map(move |x| Tile::new(x, y)))
    }
}

const NEIGHBOURS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

fn around(t: Tile) -> impl Iterator<Item = Tile> {
    NEIGHBOURS
        .into_iter()
        .map(move |(dx, dy)| Tile::new(t.x + dx, t.y + dy))
}

fn stream(seed: Seed, which: Stream<'_>, attempt: u32, purpose: &str) -> Rng {
    Rng::new(
        seed,
        which,
        &[Key::Int(i64::from(attempt)), Key::Str(purpose)],
    )
}

// ---- 1. terrain --------------------------------------------------------------------------------------------

/// Smooth value noise at `(x, y)` with lattice cells of `cell` tiles, 0..=65535.
fn value_noise(rng: &Rng, x: i32, y: i32, cell: i32) -> u32 {
    let (gx, gy) = (x.div_euclid(cell), y.div_euclid(cell));
    let (fx, fy) = (x.rem_euclid(cell), y.rem_euclid(cell));
    let lat = |ix: i32, iy: i32| -> i64 {
        let counter = (iy as u32).wrapping_mul(73_856_093) ^ (ix as u32).wrapping_mul(19_349_663);
        i64::from(rng.draw(counter) >> 16)
    };
    let smooth = |f: i32| -> i64 {
        let t = i64::from(f) * 1024 / i64::from(cell);
        t * t * (3 * 1024 - 2 * t) / (1024 * 1024)
    };
    let (sx, sy) = (smooth(fx), smooth(fy));
    let lerp = |a: i64, b: i64, s: i64| a + (b - a) * s / 1024;
    let top = lerp(lat(gx, gy), lat(gx + 1, gy), sx);
    let bottom = lerp(lat(gx, gy + 1), lat(gx + 1, gy + 1), sx);
    u32::try_from(lerp(top, bottom, sy)).unwrap_or(0)
}

fn terrain(g: &mut Grid, seed: Seed, attempt: u32, water_percent: u32) -> Result<(), String> {
    let big = stream(seed, Stream::WorldgenTerrain, attempt, "relief");
    let small = stream(seed, Stream::WorldgenTerrain, attempt, "detail");
    let heights: Vec<u32> = g
        .tiles()
        .map(|t| (3 * value_noise(&big, t.x, t.y, 14) + value_noise(&small, t.x, t.y, 6)) / 4)
        .collect();
    let mut sorted = heights.clone();
    sorted.sort_unstable();
    let cut = sorted
        .len()
        .saturating_mul(usize::try_from(water_percent).unwrap_or(0))
        / 100;
    let threshold = sorted.get(cut).copied().unwrap_or(0);
    for (t, h) in g.clone().tiles().zip(&heights) {
        if cut > 0 && *h < threshold {
            g.set_terrain(t, WATER);
        }
    }
    keep_largest_landmass(g);
    for t in g.clone().tiles() {
        if g.land(t) && around(t).any(|n| g.in_bounds(n) && g.terrain_at(n) == WATER) {
            g.set_terrain(t, SAND);
        }
    }
    let land = g.tiles().filter(|t| g.land(*t)).count();
    if land * 100 < g.tiles().count() * 30 {
        return Err("too little land".into());
    }
    Ok(())
}

/// Turns every landmass but the biggest (ties: the one found first) into water.
fn keep_largest_landmass(g: &mut Grid) {
    let n = g.tiles().count();
    let mut label: Vec<u32> = vec![0; n];
    let (mut best, mut best_size, mut next) = (0u32, 0usize, 1u32);
    for start in g.tiles() {
        let Some(si) = g.idx(start) else { continue };
        if !g.land(start) || label.get(si).copied().unwrap_or(1) != 0 {
            continue;
        }
        let mut size = 0;
        let mut queue = VecDeque::from([start]);
        if let Some(l) = label.get_mut(si) {
            *l = next;
        }
        while let Some(t) = queue.pop_front() {
            size += 1;
            for nb in around(t) {
                let Some(ni) = g.idx(nb) else { continue };
                if g.land(nb) && label.get(ni).copied().unwrap_or(1) == 0 {
                    if let Some(l) = label.get_mut(ni) {
                        *l = next;
                    }
                    queue.push_back(nb);
                }
            }
        }
        if size > best_size {
            best = next;
            best_size = size;
        }
        next += 1;
    }
    for t in g.clone().tiles() {
        let l = g.idx(t).and_then(|i| label.get(i).copied()).unwrap_or(0);
        if g.land(t) && l != best {
            g.set_terrain(t, WATER);
        }
    }
}

// ---- 2. districts ------------------------------------------------------------------------------------------

fn isqrt(n: u32) -> u32 {
    let mut x = 0u32;
    while (x + 1).saturating_mul(x + 1) <= n {
        x += 1;
    }
    x
}

fn districts(
    g: &mut Grid,
    seed: Seed,
    attempt: u32,
    p: &WorldgenParams,
) -> Result<Vec<District>, String> {
    let area = u32::try_from(g.tiles().filter(|t| g.land(*t)).count()).unwrap_or(0);
    let want = (area / p.tiles_per_district.max(1)).clamp(p.min_districts, p.max_districts);
    let margin = 5;
    let candidates: Vec<Tile> = g
        .tiles()
        .filter(|t| {
            g.terrain_at(*t) == GRASS
                && t.x >= margin
                && t.y >= margin
                && t.x < g.w - margin
                && t.y < g.h - margin
        })
        .collect();
    if candidates.is_empty() {
        return Err("no room for a district".into());
    }
    let rng = stream(seed, Stream::WorldgenRoads, attempt, "districts");
    let mut spacing = (isqrt(area / want.max(1)) * 3 / 4).max(10);
    let mut chosen: Vec<Tile> = Vec::new();
    let mut counter = 0u32;
    let mut misses = 0;
    while chosen.len() < usize::try_from(want).unwrap_or(0) && spacing >= 6 {
        let pick = rng
            .range(counter, u32::try_from(candidates.len()).unwrap_or(1))
            .and_then(|i| candidates.get(i as usize).copied());
        counter += 1;
        if let Some(t) = pick {
            if chosen.iter().all(|c| c.manhattan(t) >= spacing) {
                chosen.push(t);
                misses = 0;
                continue;
            }
        }
        misses += 1;
        if misses >= 60 {
            spacing -= 1;
            misses = 0;
        }
    }
    if chosen.is_empty() {
        return Err("could not place any district".into());
    }
    // The nearest to the middle is the commercial core, then civic, park, the rest residential.
    let mid = Tile::new(g.w / 2, g.h / 2);
    let mut order: Vec<usize> = (0..chosen.len()).collect();
    order.sort_by_key(|i| {
        let t = chosen.get(*i).copied().unwrap_or(mid);
        (t.manhattan(mid), t.y, t.x)
    });
    // The core is commercial; with four or more districts one is civic, with five or more one is a park;
    // the rest, and always at least one, are residential.
    let mut kinds = vec![DistrictKind::Residential; chosen.len()];
    let special = [
        (DistrictKind::Commercial, 1usize),
        (DistrictKind::Civic, 4),
        (DistrictKind::Park, 5),
    ];
    for (rank, idx) in order.iter().enumerate() {
        if let (Some((kind, needs)), Some(slot)) = (special.get(rank), kinds.get_mut(*idx)) {
            if chosen.len() >= *needs {
                *slot = *kind;
            }
        }
    }
    let list: Vec<District> = chosen
        .iter()
        .zip(&kinds)
        .map(|(c, k)| District {
            kind: *k,
            center: *c,
        })
        .collect();
    for t in g.clone().tiles() {
        if g.land(t) {
            let nearest = list
                .iter()
                .enumerate()
                .min_by_key(|(i, d)| (d.center.manhattan(t), *i))
                .map_or(0, |(i, _)| i + 1);
            g.set_zone(t, u16::try_from(nearest).unwrap_or(0));
        }
    }
    Ok(list)
}

// ---- 3. roads ----------------------------------------------------------------------------------------------

/// A\* over land tiles with a per-tile cost; ties broken by tile index so the result is deterministic.
fn route(
    g: &Grid,
    from: Tile,
    to: Tile,
    cost: &dyn Fn(Tile) -> u32,
    allowed: &dyn Fn(Tile) -> bool,
) -> Option<Vec<Tile>> {
    let (start, goal) = (g.idx(from)?, g.idx(to)?);
    let n = g.tiles().count();
    let mut best: Vec<u32> = vec![u32::MAX; n];
    let mut parent: Vec<usize> = vec![usize::MAX; n];
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::new();
    *best.get_mut(start)? = 0;
    heap.push(Reverse((from.manhattan(to), start)));
    while let Some(Reverse((_, i))) = heap.pop() {
        if i == goal {
            let mut path = Vec::new();
            let mut cur = i;
            while cur != start {
                let t = Tile::new(
                    i32::try_from(cur).ok()? % g.w,
                    i32::try_from(cur).ok()? / g.w,
                );
                path.push(t);
                cur = *parent.get(cur)?;
            }
            path.reverse();
            return Some(path);
        }
        let here = Tile::new(i32::try_from(i).ok()? % g.w, i32::try_from(i).ok()? / g.w);
        let g_here = *best.get(i)?;
        for nb in around(here) {
            if !g.land(nb) || !allowed(nb) {
                continue;
            }
            let ni = g.idx(nb)?;
            let g_nb = g_here.saturating_add(cost(nb));
            if g_nb < *best.get(ni)? {
                *best.get_mut(ni)? = g_nb;
                *parent.get_mut(ni)? = i;
                heap.push(Reverse((g_nb.saturating_add(nb.manhattan(to)), ni)));
            }
        }
    }
    None
}

fn lay_road(g: &mut Grid, tiles: &[Tile]) {
    for t in tiles {
        if g.land(*t) && !g.blocked_at(*t) {
            g.set_surface(*t, ROAD);
        }
    }
    for t in tiles {
        for nb in around(*t) {
            if g.land(nb) && g.surface_at(nb) == SURFACE_NONE && !g.blocked_at(nb) {
                g.set_surface(nb, SIDEWALK);
            }
        }
    }
}

fn main_roads(g: &mut Grid, seed: Seed, attempt: u32, list: &[District]) -> Result<(), String> {
    let jitter = stream(seed, Stream::WorldgenRoads, attempt, "jitter");
    let noise = |t: Tile| -> u32 {
        let counter = (t.y as u32).wrapping_mul(2_654_435_761) ^ (t.x as u32).wrapping_mul(40_503);
        jitter.draw(counter) % 3
    };
    // A spanning tree (Prim, ties by index), then loops from the shortest edges not yet used.
    let n = list.len();
    let dist = |a: usize, b: usize| -> u32 {
        match (list.get(a), list.get(b)) {
            (Some(x), Some(y)) => x.center.manhattan(y.center),
            _ => u32::MAX,
        }
    };
    let mut in_tree = BTreeSet::from([0usize]);
    let mut edges: Vec<(usize, usize)> = Vec::new();
    while in_tree.len() < n {
        let next = in_tree
            .iter()
            .flat_map(|a| {
                (0..n)
                    .filter(|b| !in_tree.contains(b))
                    .map(move |b| (*a, b))
            })
            .min_by_key(|(a, b)| (dist(*a, *b), *a, *b));
        let Some((a, b)) = next else { break };
        edges.push((a, b));
        in_tree.insert(b);
    }
    let mut extra: Vec<(usize, usize)> = (0..n)
        .flat_map(|a| ((a + 1)..n).map(move |b| (a, b)))
        .filter(|e| !edges.contains(e) && !edges.contains(&(e.1, e.0)))
        .collect();
    extra.sort_by_key(|(a, b)| (dist(*a, *b), *a, *b));
    edges.extend(extra.into_iter().take(n / 3));
    for (a, b) in edges {
        let (Some(x), Some(y)) = (list.get(a), list.get(b)) else {
            continue;
        };
        let path = route(
            g,
            x.center,
            y.center,
            &|t| 3 + noise(t) + if g.terrain_at(t) == SAND { 3 } else { 0 },
            &|_| true,
        )
        .ok_or_else(|| format!("no road between districts {a} and {b}"))?;
        let mut tiles = vec![x.center];
        tiles.extend(path);
        lay_road(g, &tiles);
    }
    Ok(())
}

// ---- 4. local streets, 5. buildings --------------------------------------------------------------------------

fn local_streets(g: &mut Grid, seed: Seed, attempt: u32, p: &WorldgenParams, list: &[District]) {
    for (i, d) in list.iter().enumerate() {
        if d.kind == DistrictKind::Park {
            continue;
        }
        let rng = stream(seed, Stream::WorldgenPlots, attempt, "blocks");
        let span = (p.block_max - p.block_min + 1).max(1);
        let counter = u32::try_from(i).unwrap_or(0);
        let spacing = i32::try_from(p.block_min).unwrap_or(14)
            + i32::try_from(rng.range(counter, span).unwrap_or(0)).unwrap_or(0);
        let zone = u16::try_from(i + 1).unwrap_or(0);
        let mut road_tiles = Vec::new();
        for t in g.tiles() {
            if g.land(t)
                && g.zone_at(t) == zone
                && ((t.x - d.center.x).rem_euclid(spacing) == 0
                    || (t.y - d.center.y).rem_euclid(spacing) == 0)
            {
                road_tiles.push(t);
            }
        }
        lay_road(g, &road_tiles);
    }
}

fn plazas(g: &mut Grid, list: &[District]) -> Vec<Tile> {
    let mut gathering = Vec::new();
    // The commercial core first, then the park.
    for kind in [DistrictKind::Commercial, DistrictKind::Park] {
        for d in list.iter().filter(|d| d.kind == kind) {
            let r = if kind == DistrictKind::Park { 3 } else { 2 };
            for dy in -r..=r {
                for dx in -r..=r {
                    let t = Tile::new(d.center.x + dx, d.center.y + dy);
                    if g.land(t) && g.surface_at(t) != ROAD {
                        g.set_surface(t, FLOOR);
                    }
                }
            }
            // Keep a plaza tile that is open ground.
            let tile = (0..=r)
                .flat_map(|rr| (-rr..=rr).flat_map(move |dy| (-rr..=rr).map(move |dx| (dx, dy))))
                .map(|(dx, dy)| Tile::new(d.center.x + dx, d.center.y + dy))
                .find(|t| g.passable(*t));
            if let Some(t) = tile {
                gathering.push(t);
            }
        }
    }
    gathering
}

fn pick_role(
    weights: &std::collections::BTreeMap<String, u32>,
    rng: &Rng,
    counter: u32,
) -> Option<BuildingRole> {
    let roles: Vec<(&String, u32)> = weights.iter().map(|(r, w)| (r, *w)).collect();
    let ws: Vec<u32> = roles.iter().map(|(_, w)| *w).collect();
    let at = rng.weighted_pick(counter, &ws)?;
    roles.get(at).and_then(|(r, _)| BuildingRole::parse(r))
}

/// Buildings stand along the streets with their doors on the street side: a sidewalk, a yard tile, then the
/// footprint. Every road tile is a candidate in a fixed order, so the result depends only on the seed.
fn buildings(
    g: &mut Grid,
    seed: Seed,
    attempt: u32,
    p: &WorldgenParams,
    list: &[District],
) -> Vec<Building> {
    let rng = stream(seed, Stream::WorldgenPlots, attempt, "frontage");
    let role_rng = stream(seed, Stream::WorldgenPlots, attempt, "roles");
    let (mut counter, mut role_counter) = (0u32, 0u32);
    let roads: Vec<Tile> = g
        .tiles()
        .filter(|t| g.land(*t) && g.surface_at(*t) == ROAD)
        .collect();
    let mut out: Vec<Building> = Vec::new();
    for r in roads {
        for (dx, dy) in NEIGHBOURS {
            // Draw for every candidate, placed or not, so one rejection does not shift later choices.
            counter = counter.wrapping_add(4);
            let draw = |k: u32, n: u32| rng.range(counter.wrapping_add(k), n.max(1)).unwrap_or(0);
            role_counter = role_counter.wrapping_add(1);
            let walk = Tile::new(r.x + dx, r.y + dy);
            let yard = Tile::new(r.x + 2 * dx, r.y + 2 * dy);
            if g.surface_at(walk) != SIDEWALK || !g.passable(yard) {
                continue;
            }
            let Some(di) = usize::from(g.zone_at(r)).checked_sub(1) else {
                continue;
            };
            let Some(district) = list.get(di) else {
                continue;
            };
            let Some(weights) = p.roles.get(district.kind.name()) else {
                continue;
            };
            let Some(role) = pick_role(weights, &role_rng, role_counter) else {
                continue;
            };
            if draw(0, 1000) < p.gap_permille {
                continue;
            }
            let span = p.building_max - p.building_min + 1;
            let along = i32::try_from(p.building_min + draw(1, span)).unwrap_or(3);
            let depth = i32::try_from(p.building_min + draw(2, span)).unwrap_or(3);
            // The footprint starts three tiles from the road (sidewalk, yard, then the wall) and is
            // centred on the road tile across the street direction.
            let (ax, ay) = (dy.abs(), dx.abs());
            let front = 3;
            let corner = |a: i32, d: i32| Tile::new(r.x + dx * d + ax * a, r.y + dy * d + ay * a);
            let lo = -(along / 2);
            let tiles: Vec<Tile> = (lo..lo + along)
                .flat_map(|a| (front..front + depth).map(move |d| (a, d)))
                .map(|(a, d)| corner(a, d))
                .collect();
            let (x0, y0) = tiles
                .iter()
                .fold((i32::MAX, i32::MAX), |m, t| (m.0.min(t.x), m.1.min(t.y)));
            let (w, h) = if dx == 0 {
                (along, depth)
            } else {
                (depth, along)
            };
            // Open land, and no other building within a tile.
            let free = tiles.iter().all(|t| {
                g.land(*t)
                    && g.surface_at(*t) == SURFACE_NONE
                    && g.terrain_at(*t) != SAND
                    && !g.blocked_at(*t)
            });
            let apart = (x0 - 1..x0 + w + 1)
                .flat_map(|x| (y0 - 1..y0 + h + 1).map(move |y| Tile::new(x, y)))
                .all(|t| !g.in_bounds(t) || !g.blocked_at(t));
            if !free || !apart {
                continue;
            }
            for t in &tiles {
                g.set_blocked(*t, true);
            }
            out.push(Building {
                role,
                x: x0,
                y: y0,
                w,
                h,
                entrance: yard,
                district: u32::try_from(di).unwrap_or(0),
            });
        }
    }
    out
}

// ---- 7. people ---------------------------------------------------------------------------------------------

/// How many of the eight tiles around `t` can be stood on.
fn open_around(g: &Grid, t: Tile) -> usize {
    (-1i32..=1)
        .flat_map(|dy| (-1i32..=1).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| (*dx, *dy) != (0, 0) && g.passable(Tile::new(t.x + dx, t.y + dy)))
        .count()
}

/// Free tiles in the yard of a building, nearest the door first, grass before pavement. A resident who
/// stands still (asleep, at a meal) must not block a one-tile passage, so only open ground counts.
fn yard(g: &Grid, b: &Building, used: &BTreeSet<Tile>) -> Vec<Tile> {
    let mut tiles: Vec<Tile> = Vec::new();
    for dy in -4i32..=4 {
        for dx in -4i32..=4 {
            let t = Tile::new(b.entrance.x + dx, b.entrance.y + dy);
            if g.passable(t)
                && !used.contains(&t)
                && t.manhattan(b.entrance) <= 4
                && open_around(g, t) >= 6
            {
                tiles.push(t);
            }
        }
    }
    tiles.sort_by_key(|t| {
        (
            g.surface_at(*t) != SURFACE_NONE,
            t.manhattan(b.entrance),
            t.y,
            t.x,
        )
    });
    tiles
}

fn place_people(
    g: &Grid,
    seed: Seed,
    attempt: u32,
    data: &GameData,
    town: &Town,
    plan: &PopulationPlan,
) -> Result<PlacePlan, String> {
    let rng = stream(seed, Stream::WorldgenPeople, attempt, "homes");
    let mut homes: Vec<usize> = town
        .buildings_with(BuildingRole::Home)
        .map(|(i, _)| i)
        .collect();
    if homes.len() < plan.households.len() {
        return Err(format!(
            "only {} home(s) for {} household(s)",
            homes.len(),
            plan.households.len()
        ));
    }
    rng.shuffle(&mut homes);
    let mut used: BTreeSet<Tile> = BTreeSet::new();
    let mut home_tile: Vec<Option<Tile>> = vec![None; plan.residents.len()];
    for (hh, home) in plan.households.iter().zip(&homes) {
        let b = town.buildings.get(*home).ok_or("a home vanished")?;
        let free = yard(g, b, &used);
        if free.len() < hh.members.len() {
            return Err("a home's yard has no room for its household".into());
        }
        for (member, tile) in hh.members.iter().zip(free) {
            used.insert(tile);
            if let Some(slot) = home_tile.get_mut(*member) {
                *slot = Some(tile);
            }
        }
    }
    let work_buildings: Vec<usize> = town
        .buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.role != BuildingRole::Home)
        .map(|(i, _)| i)
        .collect();
    let jobs = stream(seed, Stream::WorldgenPeople, attempt, "workplaces");
    let mut workplace: Vec<Option<Tile>> = vec![None; plan.residents.len()];
    for (i, r) in plan.residents.iter().enumerate() {
        let has_duties = data
            .occupations
            .get(&r.occupation)
            .is_some_and(|o| !o.duties.is_empty());
        if !has_duties || work_buildings.is_empty() {
            continue;
        }
        let counter = u32::try_from(i).unwrap_or(0);
        let Some(at) = jobs.range(counter, u32::try_from(work_buildings.len()).unwrap_or(1)) else {
            continue;
        };
        let Some(b) = work_buildings
            .get(at as usize)
            .and_then(|bi| town.buildings.get(*bi))
        else {
            continue;
        };
        if let Some(tile) = yard(g, b, &used).into_iter().next() {
            used.insert(tile);
            if let Some(slot) = workplace.get_mut(i) {
                *slot = Some(tile);
            }
        }
    }
    let homes: Vec<Tile> = home_tile
        .into_iter()
        .map(|t| t.ok_or_else(|| "a resident has no home tile".to_owned()))
        .collect::<Result<_, _>>()?;
    Ok(PlacePlan {
        homes,
        workplaces: workplace,
    })
}

// ---- 8. validation -------------------------------------------------------------------------------------------

/// Reports what is wrong with a generated layout, or nothing.
fn validate(g: &Grid, town: &Town, places: &PlacePlan) -> Result<(), String> {
    let Some(start) = town.gathering.first().copied() else {
        return Err("the town has no gathering place".into());
    };
    if !g.passable(start) {
        return Err("the main plaza cannot be stood on".into());
    }
    for (i, a) in town.buildings.iter().enumerate() {
        if a.x < 0 || a.y < 0 || a.x + a.w > g.w || a.y + a.h > g.h {
            return Err(format!("building {i} leaves the map"));
        }
        if town.buildings.iter().skip(i + 1).any(|b| a.overlaps(b)) {
            return Err(format!("building {i} overlaps another"));
        }
    }
    // Everything a resident needs must be reachable from the main plaza over open ground.
    let mut seen: BTreeSet<Tile> = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(t) = queue.pop_front() {
        for nb in around(t) {
            if g.passable(nb) && seen.insert(nb) {
                queue.push_back(nb);
            }
        }
    }
    let unreachable = |t: &Tile| !seen.contains(t);
    if let Some(b) = town.buildings.iter().find(|b| unreachable(&b.entrance)) {
        return Err(format!("the entrance at {} cannot be reached", b.entrance));
    }
    if let Some(t) = places.homes.iter().find(|t| unreachable(t)) {
        return Err(format!("the home tile {t} cannot be reached"));
    }
    if let Some(t) = places.workplaces.iter().flatten().find(|t| unreachable(t)) {
        return Err(format!("the workplace tile {t} cannot be reached"));
    }
    if town.gathering.iter().any(unreachable) {
        return Err("a gathering place cannot be reached from the main plaza".into());
    }
    Ok(())
}

// ---- the whole thing -----------------------------------------------------------------------------------------

struct Layout {
    grid: Grid,
    town: Town,
    places: PlacePlan,
}

fn layout(
    seed: Seed,
    data: &GameData,
    wg: &WorldgenParams,
    params: &GenParams,
    plan: &PopulationPlan,
    attempt: u32,
) -> Result<Layout, String> {
    let mut g = Grid::new(params.width, params.height);
    terrain(
        &mut g,
        seed,
        attempt,
        params.water_percent.min(wg.max_water_percent),
    )?;
    let list = districts(&mut g, seed, attempt, wg)?;
    main_roads(&mut g, seed, attempt, &list)?;
    local_streets(&mut g, seed, attempt, wg, &list);
    let gathering = plazas(&mut g, &list);
    let built = buildings(&mut g, seed, attempt, wg, &list);
    let town = Town {
        districts: list,
        buildings: built,
        gathering,
        starting_hash: None,
    };
    let places = place_people(&g, seed, attempt, data, &town, plan)?;
    validate(&g, &town, &places)?;
    Ok(Layout {
        grid: g,
        town,
        places,
    })
}

/// Generates a town into `world`, which must have no map yet: the map, the districts and buildings, the
/// residents with their households, relationships, memories, homes and workplaces. Everything draws from the
/// world's seed, so the same seed, parameters and data always give the same world.
pub fn generate_town(
    world: &mut WorldState,
    data: &GameData,
    params: &GenParams,
) -> Result<GenOutcome, GenError> {
    let Some(wg) = data.worldgen.as_ref() else {
        return Err(GenError::MissingData("town generator parameters"));
    };
    if !world.maps.is_empty() {
        return Err(GenError::BadParams("the world already has a map".into()));
    }
    if !(MIN_SIDE..=MAX_SIDE).contains(&params.width)
        || !(MIN_SIDE..=MAX_SIDE).contains(&params.height)
    {
        return Err(GenError::BadParams(format!(
            "a town is {MIN_SIDE} to {MAX_SIDE} tiles on each side"
        )));
    }
    let seed = world.seed();
    let plan = plan_population(data, seed, params.residents)?;
    let mut last = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        match layout(seed, data, wg, params, &plan, attempt) {
            Ok(l) => return build(world, data, params, plan, l, attempt),
            Err(reason) => last = reason,
        }
    }
    Err(GenError::Failed {
        attempts: MAX_ATTEMPTS,
        reason: last,
    })
}

fn build(
    world: &mut WorldState,
    data: &GameData,
    params: &GenParams,
    plan: PopulationPlan,
    l: Layout,
    attempt: u32,
) -> Result<GenOutcome, GenError> {
    let map = world.create_map(MapKind::Overworld, params.width, params.height)?;
    if let Some(m) = world.maps.get_mut(map) {
        for t in l.grid.tiles() {
            let _ = m.set_terrain(t, l.grid.terrain_at(t));
            let _ = m.set_surface(t, l.grid.surface_at(t));
            let _ = m.set_blocked(t, l.grid.blocked_at(t));
            let _ = m.set_zone(t, l.grid.zone_at(t));
        }
    }
    world.settings.tone = params.tone;
    world.town = l.town;
    apply_population(world, data, &plan, map, &l.places)?;
    world.rebuild_derived();
    let hash = world.state_hash().to_hex();
    world.town.starting_hash = Some(hash.clone());
    Ok(GenOutcome {
        map,
        attempt,
        districts: world.town.districts.len(),
        buildings: world.town.buildings.len(),
        residents: plan.residents.len(),
        starting_hash: hash,
    })
}

/// Checks a generated world the way the generator does: reachability from the main plaza, overlapping
/// buildings, homes for everyone. Run after load and after edits (Blueprint §12.3).
pub fn validate_world(world: &WorldState) -> Result<(), String> {
    let Some((map_id, map)) = world.maps.iter().next() else {
        return Err("the world has no map".into());
    };
    let mut g = Grid::new(map.width(), map.height());
    for t in g.clone().tiles() {
        g.set_terrain(t, map.terrain_at(t).unwrap_or(WATER));
        g.set_surface(t, map.surface_at(t).unwrap_or(SURFACE_NONE));
        g.set_blocked(t, map.is_blocked(t));
    }
    let homes: Vec<Tile> = world
        .pawns
        .iter()
        .filter_map(|(_, p)| p.home_tile)
        .collect();
    if homes.len() != world.pawns.len() {
        return Err("a resident has no home".into());
    }
    let workplaces: Vec<Option<Tile>> = world.pawns.iter().map(|(_, p)| p.workplace).collect();
    validate(&g, &world.town, &PlacePlan { homes, workplaces })?;
    for (id, p) in world.pawns.iter() {
        if p.position.map == map_id && map.is_blocked(p.position.tile) {
            return Err(format!("{id} stands on a blocked tile"));
        }
    }
    Ok(())
}

/// A text picture of a generated town for the developer tools: terrain, roads, plazas, buildings by role
/// (`H` home, `S` shop, `O` office, `C` civic) and residents' home tiles (`h`).
pub fn render_ascii(world: &WorldState) -> String {
    let Some((_, map)) = world.maps.iter().next() else {
        return String::new();
    };
    let mut out = String::new();
    for y in 0..map.height() {
        for x in 0..map.width() {
            let t = Tile::new(x, y);
            let c = if let Some(b) = world.town.buildings.iter().find(|b| b.contains(t)) {
                b.role.letter()
            } else if world.pawns.iter().any(|(_, p)| p.home_tile == Some(t)) {
                'h'
            } else if world.town.gathering.contains(&t) {
                '@'
            } else {
                match (map.terrain_at(t), map.surface_at(t)) {
                    (Some(WATER), _) => '~',
                    (_, Some(ROAD)) => '=',
                    (_, Some(SIDEWALK)) => ':',
                    (_, Some(FLOOR)) => '_',
                    (Some(SAND), _) => ',',
                    _ => '.',
                }
            };
            out.push(c);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
