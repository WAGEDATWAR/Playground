//! Pathfinding (Blueprint §7.3).
//!
//! * **Algorithm:** A\* on a 4-connected grid. The cost of a step is the cost of the tile entered; the
//!   heuristic is Manhattan distance times the cheapest possible step, which is admissible.
//! * **Determinism:** the open set is a binary heap ordered by `(f, h, tile index)`; neighbors expand in
//!   the fixed order N, E, S, W; nothing is random. The same inputs always give the same path.
//! * **Limits:** a node-expansion cap per request (default 20,000).
//! * **Goals:** a destination is a [`PlaceRef`]; [`resolve_destination`] picks the nearest free tile by
//!   `(distance, tile index)`, or reports `BlockedDestination`.
//! * **Caching:** results are keyed by `(map, from, to, edit_version, cap)` in a bounded LRU. The cache is
//!   derived state: it can change speed but never results.
//! * **Batches:** [`solve_cached`] solves many requests through a [`BatchExecutor`]. Because each request is a
//!   pure function of its inputs, the results are identical for any executor and any thread count; the
//!   core ships a serial executor and `pg-runtime` ships a threaded one.

use crate::id::EntityId;
use crate::map::{Dir4, MapData, MoveCosts, Tile};
use crate::world::WorldState;
use pg_content::{ContentSet, TemplateId};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};
use std::fmt;

pub const DEFAULT_EXPANSION_CAP: u32 = 20_000;
const NO_PARENT: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathOutcome {
    /// The tiles to enter, in order (the start is not included), with their total cost.
    Found {
        path: Vec<Tile>,
        cost: u32,
        expanded: u32,
    },
    /// The search finished without reaching the goal.
    Unreachable { expanded: u32 },
    /// The expansion cap was hit first.
    TooFar { expanded: u32 },
    /// The goal itself cannot be entered (blocked, impassable, out of bounds).
    BlockedDestination,
}

impl PathOutcome {
    pub fn is_found(&self) -> bool {
        matches!(self, PathOutcome::Found { .. })
    }

    /// The Blueprint §8.7 reason code for a failed outcome.
    pub fn failure_reason(&self) -> Option<&'static str> {
        match self {
            PathOutcome::Found { .. } => None,
            PathOutcome::Unreachable { .. } | PathOutcome::TooFar { .. } => {
                Some("unreachable_or_too_far")
            }
            PathOutcome::BlockedDestination => Some("blocked_destination"),
        }
    }
}

fn scaled(h: u32, min_cost: u32) -> u32 {
    h.saturating_mul(min_cost)
}

/// Finds a cheapest path from `from` to `to`.
pub fn find_path(
    map: &MapData,
    costs: &MoveCosts,
    from: Tile,
    to: Tile,
    expansion_cap: u32,
) -> PathOutcome {
    if costs.step_cost(map, to).is_none() {
        return PathOutcome::BlockedDestination;
    }
    let (Some(start_idx), Some(goal_idx)) = (map.index(from), map.index(to)) else {
        return PathOutcome::Unreachable { expanded: 0 };
    };
    if from == to {
        return PathOutcome::Found {
            path: Vec::new(),
            cost: 0,
            expanded: 0,
        };
    }
    let to_u32 = |i: usize| u32::try_from(i).unwrap_or(NO_PARENT - 1);
    let (start, goal) = (to_u32(start_idx), to_u32(goal_idx));
    let min_cost = costs.min_step_cost();

    // Best known (g, parent) per tile index, sparse so a request on a huge map costs only what it explores.
    let mut best: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u32, u32, u32)>> = BinaryHeap::new();
    best.insert(start, (0, NO_PARENT));
    let h0 = scaled(from.manhattan(to), min_cost);
    heap.push(Reverse((h0, h0, start)));
    let mut expanded = 0u32;

    while let Some(Reverse((f, h, idx))) = heap.pop() {
        let Some(&(g, _)) = best.get(&idx) else {
            continue;
        };
        if f.saturating_sub(h) > g {
            continue; // a stale entry: this node was reached more cheaply since it was pushed
        }
        if idx == goal {
            let mut path = Vec::new();
            let mut cur = idx;
            while cur != start {
                let Some(t) = map.tile_of(cur as usize) else {
                    break;
                };
                path.push(t);
                match best.get(&cur) {
                    Some(&(_, parent)) if parent != NO_PARENT => cur = parent,
                    _ => break,
                }
            }
            path.reverse();
            return PathOutcome::Found {
                path,
                cost: g,
                expanded,
            };
        }
        if expanded >= expansion_cap {
            return PathOutcome::TooFar { expanded };
        }
        expanded += 1;
        let Some(tile) = map.tile_of(idx as usize) else {
            continue;
        };
        for dir in Dir4::ALL {
            let next = dir.step(tile);
            let Some(step) = costs.step_cost(map, next) else {
                continue;
            };
            let Some(next_idx) = map.index(next).map(to_u32) else {
                continue;
            };
            let ng = g.saturating_add(step);
            if best.get(&next_idx).is_none_or(|&(old, _)| ng < old) {
                best.insert(next_idx, (ng, idx));
                let nh = scaled(next.manhattan(to), min_cost);
                heap.push(Reverse((ng.saturating_add(nh), nh, next_idx)));
            }
        }
    }
    PathOutcome::Unreachable { expanded }
}

/// One path request, borrowed from the world.
#[derive(Copy, Clone, Debug)]
pub struct PathJob<'a> {
    pub map: &'a MapData,
    pub costs: &'a MoveCosts,
    pub from: Tile,
    pub to: Tile,
    pub cap: u32,
}

impl PathJob<'_> {
    pub fn solve(&self) -> PathOutcome {
        find_path(self.map, self.costs, self.from, self.to, self.cap)
    }
}

/// Solves a batch of independent requests. Implementations may use any number of threads but **must
/// return the results in the order of `jobs`**; since [`PathJob::solve`] is pure, that makes the output
/// identical for every implementation.
pub trait BatchExecutor: Send + Sync {
    fn solve(&self, jobs: &[PathJob<'_>]) -> Vec<PathOutcome>;

    /// How many threads this executor uses (for display).
    fn threads(&self) -> usize {
        1
    }
}

/// Solves jobs one after another on the calling thread.
#[derive(Copy, Clone, Debug, Default)]
pub struct SerialExecutor;

impl BatchExecutor for SerialExecutor {
    fn solve(&self, jobs: &[PathJob<'_>]) -> Vec<PathOutcome> {
        jobs.iter().map(PathJob::solve).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheKey {
    pub map: EntityId,
    pub from: Tile,
    pub to: Tile,
    pub edit_version: u64,
    pub cap: u32,
}

/// A bounded least-recently-used cache of path results. Derived state: never saved or hashed.
#[derive(Clone, Debug)]
pub struct PathCache {
    capacity: usize,
    entries: BTreeMap<CacheKey, PathOutcome>,
    order: VecDeque<CacheKey>,
    hits: u64,
    misses: u64,
}

impl PathCache {
    pub fn new(capacity: usize) -> PathCache {
        PathCache {
            capacity: capacity.max(1),
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<PathOutcome> {
        match self.entries.get(key) {
            Some(v) => {
                self.hits += 1;
                let v = v.clone();
                if let Some(pos) = self.order.iter().position(|k| k == key) {
                    self.order.remove(pos);
                }
                self.order.push_back(key.clone());
                Some(v)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn insert(&mut self, key: CacheKey, value: PathOutcome) {
        if self.entries.insert(key.clone(), value).is_some() {
            if let Some(pos) = self.order.iter().position(|k| *k == key) {
                self.order.remove(pos);
            }
        }
        self.order.push_back(key);
        while self.entries.len() > self.capacity {
            match self.order.pop_front() {
                Some(old) => {
                    self.entries.remove(&old);
                }
                None => break,
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `(hits, misses)` since creation.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

impl Default for PathCache {
    fn default() -> Self {
        PathCache::new(512)
    }
}

/// Solves `jobs` through `exec`, using and filling `cache`. Duplicate requests in one batch are solved
/// once. Results are in job order and are identical to solving every job directly.
pub fn solve_cached(
    exec: &dyn BatchExecutor,
    cache: &mut PathCache,
    jobs: &[PathJob<'_>],
) -> Vec<PathOutcome> {
    let keys: Vec<CacheKey> = jobs
        .iter()
        .map(|j| CacheKey {
            map: j.map.id,
            from: j.from,
            to: j.to,
            edit_version: j.map.edit_version(),
            cap: j.cap,
        })
        .collect();
    let mut results: Vec<Option<PathOutcome>> = keys.iter().map(|k| cache.get(k)).collect();

    // Unique misses in first-occurrence order.
    let mut unique: Vec<usize> = Vec::new();
    let mut seen: BTreeMap<&CacheKey, usize> = BTreeMap::new();
    for (i, r) in results.iter().enumerate() {
        if r.is_none() {
            if let Some(key) = keys.get(i) {
                if !seen.contains_key(key) {
                    seen.insert(key, unique.len());
                    unique.push(i);
                }
            }
        }
    }
    let miss_jobs: Vec<PathJob<'_>> = unique
        .iter()
        .filter_map(|&i| jobs.get(i).copied())
        .collect();
    let solved = exec.solve(&miss_jobs);
    for (u, outcome) in solved.iter().enumerate() {
        if let Some(key) = unique.get(u).and_then(|&i| keys.get(i)) {
            cache.insert(key.clone(), outcome.clone());
        }
    }
    for (i, slot) in results.iter_mut().enumerate() {
        if slot.is_none() {
            let outcome = keys
                .get(i)
                .and_then(|k| seen.get(k))
                .and_then(|&u| solved.get(u))
                .cloned();
            *slot = Some(outcome.unwrap_or(PathOutcome::Unreachable { expanded: 0 }));
        }
    }
    results
        .into_iter()
        .map(|r| r.unwrap_or(PathOutcome::Unreachable { expanded: 0 }))
        .collect()
}

/// Where a pawn wants to go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceRef {
    Tile(Tile),
    /// A named interaction point of an object (`sit`, `sleep`, `eat`, …).
    ObjectPoint {
        object: EntityId,
        point: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationError {
    UnknownMap(EntityId),
    UnknownObject(EntityId),
    UnknownTemplate(TemplateId),
    /// The object is not on this map (or is inside a container).
    NotOnMap(EntityId),
    /// The object has no interaction point with that name.
    UnknownPoint {
        object: EntityId,
        point: String,
    },
    /// Every candidate tile is impassable or occupied.
    BlockedDestination,
}

impl fmt::Display for DestinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DestinationError::UnknownMap(m) => write!(f, "no map {m}"),
            DestinationError::UnknownObject(o) => write!(f, "no object {o}"),
            DestinationError::UnknownTemplate(t) => write!(f, "no template '{t}'"),
            DestinationError::NotOnMap(o) => write!(f, "{o} is not on this map"),
            DestinationError::UnknownPoint { object, point } => {
                write!(f, "{object} has no interaction point '{point}'")
            }
            DestinationError::BlockedDestination => f.write_str("blocked_destination"),
        }
    }
}

impl std::error::Error for DestinationError {}

/// Picks the tile to walk to for `place`: the nearest free candidate by `(distance from `from`, tile
/// index)`. A candidate is free if it can be stood on and no pawn other than `requester` is on it.
pub fn resolve_destination(
    world: &WorldState,
    content: &ContentSet,
    map: EntityId,
    from: Tile,
    requester: Option<EntityId>,
    place: &PlaceRef,
) -> Result<Tile, DestinationError> {
    let m = world
        .maps
        .get(map)
        .ok_or(DestinationError::UnknownMap(map))?;
    let candidates: Vec<Tile> = match place {
        PlaceRef::Tile(t) => vec![*t],
        PlaceRef::ObjectPoint { object, point } => {
            let obj = world
                .objects
                .get(*object)
                .ok_or(DestinationError::UnknownObject(*object))?;
            let crate::object::Location::OnMap { map: obj_map, tile } = &obj.location else {
                return Err(DestinationError::NotOnMap(*object));
            };
            if *obj_map != map {
                return Err(DestinationError::NotOnMap(*object));
            }
            let template = content
                .get(&obj.template)
                .ok_or_else(|| DestinationError::UnknownTemplate(obj.template.clone()))?;
            let points: Vec<Tile> = template
                .component("interaction")
                .and_then(|c| c.get("points"))
                .and_then(pg_canon::Canon::as_list)
                .map(|l| {
                    l.iter()
                        .filter(|p| {
                            p.get("name").and_then(pg_canon::Canon::as_str) == Some(point.as_str())
                        })
                        .filter_map(|p| {
                            let dx = i32::try_from(p.get("dx")?.as_i64()?).ok()?;
                            let dy = i32::try_from(p.get("dy")?.as_i64()?).ok()?;
                            Some(Tile::new(
                                tile.x.saturating_add(dx),
                                tile.y.saturating_add(dy),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if points.is_empty() {
                return Err(DestinationError::UnknownPoint {
                    object: *object,
                    point: point.clone(),
                });
            }
            points
        }
    };
    let costs = MoveCosts::default();
    candidates
        .into_iter()
        .filter(|t| costs.step_cost(m, *t).is_some())
        .filter(|t| {
            world
                .occupancy
                .occupant(map, *t)
                .is_none_or(|by| Some(by) == requester)
        })
        .min_by_key(|t| (from.manhattan(*t), m.index(*t)))
        .ok_or(DestinationError::BlockedDestination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;
    use crate::map::{MapKind, ROAD, SAND, WATER};
    use proptest::prelude::*;

    fn map(w: i32, h: i32) -> MapData {
        MapData::new(EntityId::new(Kind::Map, 1), MapKind::Overworld, w, h).unwrap()
    }

    fn t(x: i32, y: i32) -> Tile {
        Tile::new(x, y)
    }

    fn find(m: &MapData, a: Tile, b: Tile) -> PathOutcome {
        find_path(m, &MoveCosts::default(), a, b, DEFAULT_EXPANSION_CAP)
    }

    #[test]
    fn a_straight_line_costs_grass_per_step() {
        let m = map(10, 1);
        match find(&m, t(0, 0), t(4, 0)) {
            PathOutcome::Found { path, cost, .. } => {
                assert_eq!(path, vec![t(1, 0), t(2, 0), t(3, 0), t(4, 0)]);
                assert_eq!(cost, 40);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn staying_put_is_a_free_empty_path() {
        let m = map(3, 3);
        assert_eq!(
            find(&m, t(1, 1), t(1, 1)),
            PathOutcome::Found {
                path: vec![],
                cost: 0,
                expanded: 0
            }
        );
    }

    #[test]
    fn roads_are_preferred_even_when_longer() {
        // Row 0 is a road; a direct walk along row 2 over grass costs 10/step, the road detour 6/step.
        let mut m = map(12, 3);
        for x in 0..12 {
            m.set_surface(t(x, 0), ROAD).unwrap();
        }
        match find(&m, t(0, 2), t(11, 2)) {
            PathOutcome::Found { path, cost, .. } => {
                assert!(
                    path.iter().any(|p| p.y == 0),
                    "the path should use the road: {path:?}"
                );
                // up 2 (grass? no: entering road costs 6, entering (0,1) grass 10), along road, back down.
                assert!(cost < 11 * 10, "cheaper than walking the grass: {cost}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn blocked_goals_unreachable_goals_and_walls() {
        let mut m = map(5, 5);
        m.set_blocked(t(4, 4), true).unwrap();
        assert_eq!(find(&m, t(0, 0), t(4, 4)), PathOutcome::BlockedDestination);
        assert_eq!(
            find(&m, t(0, 0), t(9, 9)),
            PathOutcome::BlockedDestination,
            "out of bounds"
        );
        assert!(
            matches!(
                find(&m, t(9, 9), t(0, 0)),
                PathOutcome::Unreachable { expanded: 0 }
            ),
            "start out of bounds"
        );
        // A full wall splits the map.
        let mut m = map(5, 5);
        for y in 0..5 {
            m.set_terrain(t(2, y), WATER).unwrap();
        }
        assert!(matches!(
            find(&m, t(0, 0), t(4, 4)),
            PathOutcome::Unreachable { .. }
        ));
        // Open one gap and it is reachable again.
        m.set_terrain(t(2, 3), SAND).unwrap();
        assert!(find(&m, t(0, 0), t(4, 4)).is_found());
    }

    #[test]
    fn the_expansion_cap_returns_too_far() {
        let m = map(60, 60);
        let r = find_path(&m, &MoveCosts::default(), t(0, 0), t(59, 59), 50);
        assert!(matches!(r, PathOutcome::TooFar { expanded: 50 }), "{r:?}");
        assert_eq!(r.failure_reason(), Some("unreachable_or_too_far"));
        assert!(find_path(
            &m,
            &MoveCosts::default(),
            t(0, 0),
            t(59, 59),
            DEFAULT_EXPANSION_CAP
        )
        .is_found());
    }

    #[test]
    fn the_same_request_always_gives_the_same_path() {
        let mut m = map(30, 30);
        for i in 0..25 {
            m.set_blocked(t(10, i), true).unwrap();
            m.set_blocked(t(20, 29 - i), true).unwrap();
        }
        let a = find(&m, t(0, 0), t(29, 29));
        for _ in 0..5 {
            assert_eq!(find(&m, t(0, 0), t(29, 29)), a);
        }
    }

    #[test]
    fn ties_resolve_by_the_fixed_expansion_order() {
        // On an open grid many shortest paths exist; the choice is fixed, not arbitrary.
        let m = map(6, 6);
        let a = find(&m, t(0, 0), t(3, 3));
        match &a {
            PathOutcome::Found { path, cost, .. } => {
                assert_eq!(*cost, 60);
                assert_eq!(path.len(), 6);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(a, find(&m, t(0, 0), t(3, 3)));
    }

    /// Reference implementation: Dijkstra over the same cost model.
    fn dijkstra(m: &MapData, from: Tile, to: Tile) -> Option<u32> {
        let costs = MoveCosts::default();
        costs.step_cost(m, to)?;
        let mut dist: BTreeMap<Tile, u32> = BTreeMap::new();
        let mut heap = BinaryHeap::new();
        dist.insert(from, 0);
        heap.push(Reverse((0u32, from.x, from.y)));
        while let Some(Reverse((d, x, y))) = heap.pop() {
            let cur = Tile::new(x, y);
            if dist.get(&cur).copied() != Some(d) {
                continue;
            }
            if cur == to {
                return Some(d);
            }
            for dir in Dir4::ALL {
                let n = dir.step(cur);
                if let Some(c) = costs.step_cost(m, n) {
                    let nd = d + c;
                    if dist.get(&n).is_none_or(|&old| nd < old) {
                        dist.insert(n, nd);
                        heap.push(Reverse((nd, n.x, n.y)));
                    }
                }
            }
        }
        None
    }

    fn assert_valid_path(m: &MapData, from: Tile, to: Tile, path: &[Tile], cost: u32) {
        let costs = MoveCosts::default();
        let mut cur = from;
        let mut total = 0;
        for &step in path {
            assert_eq!(cur.manhattan(step), 1, "steps must be adjacent");
            total += costs.step_cost(m, step).expect("every step is passable");
            cur = step;
        }
        assert_eq!(cur, to);
        assert_eq!(total, cost);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(160))]

        #[test]
        fn a_star_matches_dijkstra_on_random_maps(
            w in 2i32..9, h in 2i32..9,
            cells in proptest::collection::vec((0u8..4, 0u8..4, any::<bool>()), 81),
            a in (0i32..9, 0i32..9), b in (0i32..9, 0i32..9),
        ) {
            let mut m = map(w, h);
            for y in 0..h { for x in 0..w {
                let (terrain, surface, blocked) = cells[(y * w + x) as usize];
                m.set_terrain(t(x, y), terrain).unwrap();   // 0 grass, 1 water, 2 sand, 3 unknown(impassable)
                m.set_surface(t(x, y), surface).unwrap();   // 0 none, 1 road, 2 sidewalk, 3 floor
                m.set_blocked(t(x, y), blocked && (x + y) % 3 == 0).unwrap();
            }}
            let (from, to) = (t(a.0 % w, a.1 % h), t(b.0 % w, b.1 % h));
            let expected = dijkstra(&m, from, to);
            match find(&m, from, to) {
                PathOutcome::Found { path, cost, .. } => {
                    prop_assert_eq!(Some(cost), expected);
                    assert_valid_path(&m, from, to, &path, cost);
                }
                PathOutcome::BlockedDestination => prop_assert_eq!(expected, None),
                PathOutcome::Unreachable { .. } => prop_assert_eq!(expected, None),
                PathOutcome::TooFar { .. } => prop_assert!(false, "cap is huge for these maps"),
            }
        }
    }

    // ----- cache and batches ------------------------------------------------------------

    fn jobs_for<'a>(m: &'a MapData, c: &'a MoveCosts, pairs: &[(Tile, Tile)]) -> Vec<PathJob<'a>> {
        pairs
            .iter()
            .map(|&(from, to)| PathJob {
                map: m,
                costs: c,
                from,
                to,
                cap: DEFAULT_EXPANSION_CAP,
            })
            .collect()
    }

    #[test]
    fn cached_batches_equal_direct_solving_and_dedupe() {
        let mut m = map(20, 20);
        for i in 0..15 {
            m.set_blocked(t(8, i), true).unwrap();
        }
        let c = MoveCosts::default();
        let pairs = [
            (t(0, 0), t(19, 19)),
            (t(1, 1), t(18, 2)),
            (t(0, 0), t(19, 19)),
            (t(5, 5), t(5, 5)),
            (t(0, 0), t(8, 3)),
        ];
        let jobs = jobs_for(&m, &c, &pairs);
        let direct: Vec<_> = jobs.iter().map(PathJob::solve).collect();
        let mut cache = PathCache::new(16);
        let first = solve_cached(&SerialExecutor, &mut cache, &jobs);
        assert_eq!(first, direct);
        assert_eq!(cache.len(), 4, "the duplicate request is cached once");
        let again = solve_cached(&SerialExecutor, &mut cache, &jobs);
        assert_eq!(again, direct, "cache hits return identical results");
        let (hits, _) = cache.stats();
        assert!(hits >= 5, "second batch is served from the cache");
    }

    #[test]
    fn a_map_edit_invalidates_cached_paths() {
        let mut m = map(10, 1);
        let c = MoveCosts::default();
        let mut cache = PathCache::new(8);
        let pairs = [(t(0, 0), t(9, 0))];
        let r1 = solve_cached(&SerialExecutor, &mut cache, &jobs_for(&m, &c, &pairs));
        assert!(r1[0].is_found());
        m.set_blocked(t(5, 0), true).unwrap(); // bumps edit_version, so the old entry no longer matches
        let r2 = solve_cached(&SerialExecutor, &mut cache, &jobs_for(&m, &c, &pairs));
        assert!(
            matches!(r2[0], PathOutcome::Unreachable { .. }),
            "{:?}",
            r2[0]
        );
    }

    #[test]
    fn the_cache_evicts_least_recently_used_first() {
        let m = map(10, 1);
        let c = MoveCosts::default();
        let mut cache = PathCache::new(2);
        let solve = |cache: &mut PathCache, to: i32| {
            solve_cached(
                &SerialExecutor,
                cache,
                &jobs_for(&m, &c, &[(t(0, 0), t(to, 0))]),
            )
        };
        solve(&mut cache, 1);
        solve(&mut cache, 2);
        solve(&mut cache, 1); // touch 1: now 2 is the oldest
        solve(&mut cache, 3); // evicts 2
        assert_eq!(cache.len(), 2);
        let before = cache.stats();
        solve(&mut cache, 1);
        assert_eq!(cache.stats().0, before.0 + 1, "1 is still cached");
        solve(&mut cache, 2);
        assert_eq!(cache.stats().1, before.1 + 1, "2 was evicted");
    }

    /// An executor that splits the batch across scoped threads, to prove the result does not depend on it.
    struct Threaded(usize);

    impl BatchExecutor for Threaded {
        fn solve(&self, jobs: &[PathJob<'_>]) -> Vec<PathOutcome> {
            let chunk = jobs.len().div_ceil(self.0.max(1)).max(1);
            std::thread::scope(|s| {
                let handles: Vec<_> = jobs
                    .chunks(chunk)
                    .map(|part| {
                        s.spawn(move || part.iter().map(PathJob::solve).collect::<Vec<_>>())
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().unwrap_or_default())
                    .collect()
            })
        }

        fn threads(&self) -> usize {
            self.0
        }
    }

    #[test]
    fn threaded_and_serial_executors_agree_at_any_thread_count() {
        let mut m = map(48, 48);
        for i in 0..40 {
            m.set_blocked(t(12, i), true).unwrap();
            m.set_blocked(t(24, 47 - i), true).unwrap();
            m.set_terrain(t(36, i), WATER).unwrap();
        }
        // A sealed pocket around (45,45): goals inside it are unreachable.
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            m.set_blocked(t(45 + dx, 45 + dy), true).unwrap();
        }
        let c = MoveCosts::default();
        let mut pairs = Vec::new();
        for i in 0..64 {
            pairs.push((t(i % 11, (i * 7) % 48), t(47 - (i * 3) % 12, (i * 5) % 48)));
        }
        for i in 0..6 {
            pairs.push((t(i, 0), t(45, 45))); // unreachable: sealed pocket
            pairs.push((t(i, 1), t(36, 3 + i))); // blocked destination: water
        }
        let jobs = jobs_for(&m, &c, &pairs);
        let serial = SerialExecutor.solve(&jobs);
        for n in [1, 2, 3, 4, 8, 16] {
            assert_eq!(Threaded(n).solve(&jobs), serial, "{n} threads");
        }
        assert!(serial.iter().any(PathOutcome::is_found) && serial.iter().any(|r| !r.is_found()));
    }

    // ----- destinations -----------------------------------------------------------------

    fn content() -> ContentSet {
        use pg_content::{load_pack, ComponentRegistry, Limits, MemoryPack};
        let pack = MemoryPack::new()
            .with("pack.json", r#"{"id":"base","name":"Base","version":"0.1.0"}"#)
            .with(
                "data/templates/t.json",
                r#"[{"id":"base.object","schema":1},
                    {"id":"furniture.bench","schema":1,"extends":"base.object","components":{
                      "interaction":{"points":[{"name":"sit","dx":-1,"dy":0},{"name":"sit","dx":1,"dy":0},{"name":"talk_here","dy":1}]}}},
                    {"id":"furniture.plain","schema":1,"extends":"base.object"}]"#,
            );
        ContentSet::build(
            vec![load_pack(&pack, &Limits::default()).unwrap()],
            ComponentRegistry::builtin(),
        )
        .unwrap()
    }

    fn world_with_bench() -> (WorldState, EntityId, EntityId, ContentSet) {
        use crate::containment::spawn_object;
        use crate::object::Location;
        let mut w = WorldState::new("t", "s");
        let m = w.create_map(MapKind::Overworld, 12, 12).unwrap();
        let c = content();
        let bench = spawn_object(
            &mut w,
            &c,
            &"furniture.bench".parse().unwrap(),
            Location::OnMap {
                map: m,
                tile: t(5, 5),
            },
        )
        .unwrap();
        (w, m, bench, c)
    }

    #[test]
    fn a_plain_tile_destination_is_the_tile_if_free() {
        let (mut w, m, _, c) = world_with_bench();
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 0), None, &PlaceRef::Tile(t(3, 3))),
            Ok(t(3, 3))
        );
        w.spawn_pawn("Ann", m, t(3, 3)).unwrap();
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 0), None, &PlaceRef::Tile(t(3, 3))),
            Err(DestinationError::BlockedDestination)
        );
        let me = w.occupancy.occupant(m, t(3, 3));
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 0), me, &PlaceRef::Tile(t(3, 3))),
            Ok(t(3, 3)),
            "your own tile is free to you"
        );
    }

    #[test]
    fn an_object_point_picks_the_nearest_free_candidate() {
        let (mut w, m, bench, c) = world_with_bench();
        let sit = PlaceRef::ObjectPoint {
            object: bench,
            point: "sit".into(),
        };
        // Candidates are (4,5) and (6,5). From the west the west one is nearer.
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 5), None, &sit),
            Ok(t(4, 5))
        );
        assert_eq!(
            resolve_destination(&w, &c, m, t(11, 5), None, &sit),
            Ok(t(6, 5))
        );
        // Occupy the near one: the far one is chosen.
        w.spawn_pawn("Ann", m, t(4, 5)).unwrap();
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 5), None, &sit),
            Ok(t(6, 5))
        );
        // Occupy both: blocked.
        w.spawn_pawn("Bob", m, t(6, 5)).unwrap();
        assert_eq!(
            resolve_destination(&w, &c, m, t(0, 5), None, &sit),
            Err(DestinationError::BlockedDestination)
        );
    }

    #[test]
    fn equidistant_candidates_break_ties_by_tile_index() {
        let (w, m, bench, c) = world_with_bench();
        // From (5,3) both (4,5) and (6,5) are 3 away; the lower row-major index (4,5) wins.
        let sit = PlaceRef::ObjectPoint {
            object: bench,
            point: "sit".into(),
        };
        assert_eq!(
            resolve_destination(&w, &c, m, t(5, 3), None, &sit),
            Ok(t(4, 5))
        );
    }

    #[test]
    fn destination_errors_are_specific() {
        let (w, m, bench, c) = world_with_bench();
        let nope = PlaceRef::ObjectPoint {
            object: bench,
            point: "fly".into(),
        };
        assert!(matches!(
            resolve_destination(&w, &c, m, t(0, 0), None, &nope),
            Err(DestinationError::UnknownPoint { .. })
        ));
        let missing = PlaceRef::ObjectPoint {
            object: EntityId::new(Kind::Object, 99),
            point: "sit".into(),
        };
        assert!(matches!(
            resolve_destination(&w, &c, m, t(0, 0), None, &missing),
            Err(DestinationError::UnknownObject(_))
        ));
        assert!(matches!(
            resolve_destination(
                &w,
                &c,
                EntityId::new(Kind::Map, 9),
                t(0, 0),
                None,
                &PlaceRef::Tile(t(0, 0))
            ),
            Err(DestinationError::UnknownMap(_))
        ));
        let sit = PlaceRef::ObjectPoint {
            object: bench,
            point: "sit".into(),
        };
        let other_map = {
            let mut w2 = w.clone();
            w2.create_map(MapKind::Overworld, 4, 4).unwrap()
        };
        assert!(matches!(
            resolve_destination(
                &{
                    let mut w2 = w.clone();
                    w2.create_map(MapKind::Overworld, 4, 4).unwrap();
                    w2
                },
                &c,
                other_map,
                t(0, 0),
                None,
                &sit
            ),
            Err(DestinationError::NotOnMap(_))
        ));
    }
}
