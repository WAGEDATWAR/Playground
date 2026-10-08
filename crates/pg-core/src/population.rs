//! Generating a town's people from a seed (Stage 1, milestone 1.0; used by worldgen in 1.2).
//!
//! [`plan_population`] turns the game data (name pools, occupations, needs, memory and relationship
//! parameters) and a seed into a [`PopulationPlan`]: households of one to four adults, each with a name, an
//! occupation, a little seeded variation in how full their needs start, a few starting relationships and a
//! few shared memories. It reads no world state, so the same seed and content give the same plan on every
//! machine. [`apply_population`] puts a plan into a world.

use crate::action::ActionRegistry;
use crate::id::{EntityId, Kind};
use crate::map::Tile;
use crate::rng::{Key, Rng, Seed, Stream};
use crate::social::{Household, Memory, Occupation, PairKey, Relationship};
use crate::world::{WorldError, WorldState};
use pg_content::gamedata::{GameData, MemoryParams, RelationshipParams};
use pg_content::{ActionId, ValidationReport};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The most residents one town generates (Roadmap Stage 1: 10 to 20; the editor allows more later).
pub const MAX_RESIDENTS: usize = 50;

/// Household sizes and how likely each is (weights).
const HOUSEHOLD_SIZES: [(usize, u32); 4] = [(1, 25), (2, 35), (3, 20), (4, 20)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentPlan {
    pub given: String,
    pub family: String,
    pub occupation: String,
    pub variation: i32,
    pub needs: BTreeMap<String, i32>,
    /// Index into [`PopulationPlan::households`].
    pub household: usize,
}

impl ResidentPlan {
    /// The name a pawn gets: given and family, or just the given name if the pair would be too long.
    pub fn full_name(&self) -> String {
        let full = format!("{} {}", self.given, self.family);
        if full.chars().count() <= 32 {
            full
        } else {
            self.given.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HouseholdPlan {
    pub family: String,
    /// Indices into [`PopulationPlan::residents`], ascending.
    pub members: Vec<usize>,
}

/// Why two residents start out knowing each other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tie {
    Household,
    Coworkers,
    Neighbors,
}

impl Tie {
    /// The string-table key of the starting memory this tie can give.
    pub fn summary_key(self) -> &'static str {
        match self {
            Tie::Household => "memory.start.household",
            Tie::Coworkers => "memory.start.coworkers",
            Tie::Neighbors => "memory.start.neighbors",
        }
    }

    fn impact(self) -> i32 {
        match self {
            Tie::Household => 30,
            Tie::Coworkers => 15,
            Tie::Neighbors => 10,
        }
    }

    /// Chance (permille) that a pair with this tie also shares a starting memory.
    fn memory_chance(self) -> u32 {
        match self {
            Tie::Household => 500,
            Tie::Coworkers => 300,
            Tie::Neighbors => 250,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tie::Household => "household",
            Tie::Coworkers => "coworkers",
            Tie::Neighbors => "neighbors",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartingRelationship {
    pub a: usize,
    pub b: usize,
    pub affinity: i32,
    pub tie: Tie,
    /// The pair also shares a starting memory.
    pub shared_memory: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopulationPlan {
    pub residents: Vec<ResidentPlan>,
    pub households: Vec<HouseholdPlan>,
    pub relationships: Vec<StartingRelationship>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    TooMany(usize),
    NoNames,
    NoOccupations,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::TooMany(n) => write!(
                f,
                "{n} residents is more than the {MAX_RESIDENTS} a town generates"
            ),
            PlanError::NoNames => {
                f.write_str("the game data has no given or family names to generate residents from")
            }
            PlanError::NoOccupations => {
                f.write_str("the game data has no occupations to give residents")
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// A sequence of draws from one stream (the counter advances with each draw).
struct Draws {
    rng: Rng,
    next: u32,
}

impl Draws {
    fn new(seed: Seed, purpose: &str) -> Draws {
        Draws {
            rng: Rng::new(seed, Stream::WorldgenPeople, &[Key::Str(purpose)]),
            next: 0,
        }
    }

    fn counter(&mut self) -> u32 {
        let c = self.next;
        self.next = self.next.wrapping_add(1);
        c
    }

    fn int(&mut self, min: i32, max: i32) -> i32 {
        let c = self.counter();
        self.rng.int_in(c, min, max).unwrap_or(min)
    }

    fn range(&mut self, n: u32) -> usize {
        let c = self.counter();
        self.rng.range(c, n).unwrap_or(0) as usize
    }

    fn chance(&mut self, permille: u32) -> bool {
        let c = self.counter();
        self.rng.range(c, 1000).is_some_and(|r| r < permille)
    }

    fn weighted(&mut self, weights: &[u32]) -> usize {
        let c = self.counter();
        self.rng.weighted_pick(c, weights).unwrap_or(0)
    }
}

/// Checks what the content crate cannot: that every action an occupation uses exists in the action registry
/// and takes a tile (occupations point their duties at places, which resolve to tiles).
pub fn check_occupations(data: &GameData, actions: &ActionRegistry) -> ValidationReport {
    let mut report = ValidationReport::new();
    for occ in data.occupations.values() {
        let uses = occ
            .duties
            .iter()
            .map(|d| (&d.activity, "duties"))
            .chain(occ.leisure.iter().map(|l| (&l.activity, "leisure")));
        for (i, (activity, kind)) in uses.enumerate() {
            let path = format!("data/game/occupations.json.{}.{kind}[{i}]", occ.id);
            match ActionId::new(activity).ok().and_then(|id| actions.get(&id)) {
                None => report.error(
                    "unknown_action",
                    path,
                    format!(
                        "'{activity}' is not a registered action{}",
                        pg_content::hints::hint(activity, actions.iter().map(|a| a.id.as_str()))
                    ),
                ),
                Some(def) if !def.ai_proposable && def.id.as_str() == "meet_at" => report.warn(
                    "commitment_action",
                    path,
                    "'meet_at' is for commitments; occupations normally use idle_at or move_to",
                ),
                Some(_) => {}
            }
        }
    }
    report
}

/// Builds a plan for `count` residents. Pure: depends only on the arguments.
pub fn plan_population(
    data: &GameData,
    seed: Seed,
    count: usize,
) -> Result<PopulationPlan, PlanError> {
    if count > MAX_RESIDENTS {
        return Err(PlanError::TooMany(count));
    }
    if count == 0 {
        return Ok(PopulationPlan::default());
    }
    if data.names.given.is_empty() || data.names.family.is_empty() {
        return Err(PlanError::NoNames);
    }
    let occupations: Vec<(&String, u32)> = data
        .occupations
        .iter()
        .map(|(id, o)| (id, o.weight))
        .collect();
    if occupations.iter().all(|(_, w)| *w == 0) {
        return Err(PlanError::NoOccupations);
    }
    let occ_weights: Vec<u32> = occupations.iter().map(|(_, w)| *w).collect();

    // Names are drawn without replacement from a shuffled pool, wrapping if the pool runs out.
    let mut given = data.names.given.clone();
    let mut family = data.names.family.clone();
    Draws::new(seed, "given").rng.shuffle(&mut given);
    Draws::new(seed, "family").rng.shuffle(&mut family);

    let mut sizes = Draws::new(seed, "households");
    let mut jobs = Draws::new(seed, "occupations");
    let mut needs_rng = Draws::new(seed, "needs");
    let size_weights: Vec<u32> = HOUSEHOLD_SIZES.iter().map(|(_, w)| *w).collect();

    let mut plan = PopulationPlan::default();
    let mut given_at = 0usize;
    while plan.residents.len() < count {
        let left = count - plan.residents.len();
        let size = HOUSEHOLD_SIZES
            .get(sizes.weighted(&size_weights))
            .map_or(1, |(n, _)| *n)
            .min(left);
        let hh_index = plan.households.len();
        let family_name = family
            .get(hh_index % family.len())
            .cloned()
            .unwrap_or_default();
        let mut members = Vec::new();
        for _ in 0..size {
            let name = given
                .get(given_at % given.len())
                .cloned()
                .unwrap_or_default();
            given_at += 1;
            let occupation = occupations
                .get(jobs.weighted(&occ_weights))
                .map(|(id, _)| (*id).clone())
                .unwrap_or_default();
            let variation = jobs.int(0, 999);
            let mut needs = BTreeMap::new();
            for (id, def) in &data.needs {
                let level = (def.start + needs_rng.int(-100, 100)).clamp(0, 1000);
                needs.insert(id.clone(), level);
            }
            members.push(plan.residents.len());
            plan.residents.push(ResidentPlan {
                given: name,
                family: family_name.clone(),
                occupation,
                variation,
                needs,
                household: hh_index,
            });
        }
        plan.households.push(HouseholdPlan {
            family: family_name,
            members,
        });
    }
    plan_relationships(data, seed, &mut plan);
    Ok(plan)
}

fn plan_relationships(data: &GameData, seed: Seed, plan: &mut PopulationPlan) {
    let Some(rel) = &data.relationships else {
        return;
    };
    let mut d = Draws::new(seed, "relationships");
    let mut linked: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut add = |plan: &mut PopulationPlan,
                   d: &mut Draws,
                   a: usize,
                   b: usize,
                   lo: i32,
                   hi: i32,
                   tie: Tie| {
        let key = (a.min(b), a.max(b));
        if a == b || !linked.insert(key) {
            return;
        }
        let affinity = d.int(lo, hi).clamp(-1000, 1000);
        let shared_memory = d.chance(tie.memory_chance());
        plan.relationships.push(StartingRelationship {
            a: key.0,
            b: key.1,
            affinity,
            tie,
            shared_memory,
        });
    };
    let _ = rel;
    // People who share a home know each other well.
    let households = plan.households.clone();
    for hh in &households {
        for (i, a) in hh.members.iter().enumerate() {
            for b in hh.members.iter().skip(i + 1) {
                add(plan, &mut d, *a, *b, 350, 600, Tie::Household);
            }
        }
    }
    // People with the same occupation have often worked together.
    let n = plan.residents.len();
    for a in 0..n {
        for b in (a + 1)..n {
            let same_job = matches!(
                (plan.residents.get(a), plan.residents.get(b)),
                (Some(x), Some(y)) if x.occupation == y.occupation && x.household != y.household
            );
            if same_job && d.chance(600) {
                add(plan, &mut d, a, b, 100, 250, Tie::Coworkers);
            }
        }
    }
    // Everyone else knows a few people by sight.
    for a in 0..n {
        if d.chance(400) {
            let b = d.range(u32::try_from(n).unwrap_or(1));
            add(plan, &mut d, a, b, 60, 220, Tie::Neighbors);
        }
    }
    plan.relationships.sort_by_key(|r| (r.a, r.b));
}

/// Where each resident lives and works: one entry per resident, in plan order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacePlan {
    /// The tile each resident calls home (they start the world standing on it).
    pub homes: Vec<Tile>,
    /// Where each resident works, if their occupation has a place.
    pub workplaces: Vec<Option<Tile>>,
}

impl PlacePlan {
    /// Everyone at home on the given tiles, with no workplaces (tests and tools).
    pub fn at(homes: Vec<Tile>) -> PlacePlan {
        PlacePlan {
            workplaces: vec![None; homes.len()],
            homes,
        }
    }
}

/// Puts `plan` into `world`: pawns standing on their home tiles of `map`, their households, starting
/// relationships and shared memories. Returns the new pawns' ids in resident order. Refuses before changing
/// anything if there are too few tiles.
pub fn apply_population(
    world: &mut WorldState,
    data: &GameData,
    plan: &PopulationPlan,
    map: EntityId,
    places: &PlacePlan,
) -> Result<Vec<EntityId>, WorldError> {
    if places.homes.len() < plan.residents.len() {
        return Err(WorldError::NotPassable(Tile::new(-1, -1)));
    }
    let tiles = &places.homes;
    let memory: Option<&MemoryParams> = data.memory.as_ref();
    let rel_params: Option<&RelationshipParams> = data.relationships.as_ref();

    let mut ids = Vec::with_capacity(plan.residents.len());
    for (r, tile) in plan.residents.iter().zip(tiles) {
        let id = world.spawn_pawn(&r.full_name(), map, *tile)?;
        if let Some(p) = world.pawns.get_mut(id) {
            p.occupation = Some(Occupation {
                template: r.occupation.clone(),
                variation: r.variation,
            });
            p.needs = r.needs.clone();
            p.home_tile = Some(*tile);
            p.workplace = places.workplaces.get(ids.len()).copied().flatten();
        }
        ids.push(id);
    }
    for hh in &plan.households {
        let hid = world.id_counters.allocate(Kind::Household)?;
        let mut members: Vec<EntityId> = hh
            .members
            .iter()
            .filter_map(|i| ids.get(*i).copied())
            .collect();
        members.sort();
        for m in &members {
            if let Some(p) = world.pawns.get_mut(*m) {
                p.household = Some(hid);
            }
        }
        let _ = world.households.insert(
            hid,
            Household {
                id: hid,
                name: hh.family.clone(),
                members,
                home: None,
            },
        );
    }
    for link in &plan.relationships {
        let (Some(&a), Some(&b)) = (ids.get(link.a), ids.get(link.b)) else {
            continue;
        };
        let Some(key) = PairKey::new(a, b) else {
            continue;
        };
        if let Some(params) = rel_params {
            let label = params
                .label_for(link.affinity)
                .map(|l| l.id.clone())
                .unwrap_or_default();
            world.relationships.set(Relationship {
                key,
                affinity: link.affinity,
                label,
                last_interaction_tick: 0,
                day: 0,
                day_change: 0,
                last_topic: None,
                forgotten: 0,
            });
        }
        if let (true, Some(mem)) = (link.shared_memory, memory) {
            let severity = mem
                .types
                .iter()
                .find(|t| t.id == "shared_event")
                .map_or(3, |t| t.severity);
            for (owner, other) in [(a, b), (b, a)] {
                let mid = world.id_counters.allocate(Kind::Memory)?;
                let mut m = Memory::new(
                    mid,
                    "shared_event",
                    0,
                    vec![other],
                    severity,
                    link.tie.impact(),
                    mem.decay_k,
                );
                m.summary_key = Some(link.tie.summary_key().to_owned());
                if let Some(p) = world.pawns.get_mut(owner) {
                    p.memories.push(m);
                }
            }
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::ToCanon;
    use crate::map::MapKind;
    use pg_canon::json;
    use pg_content::gamedata::{parse_files, KNOWN_FILES};
    use pg_content::report::ValidationReport;

    fn data() -> GameData {
        let root = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let files = KNOWN_FILES
            .iter()
            .filter_map(|p| {
                let text = std::fs::read_to_string(format!("{root}/{p}")).ok()?;
                Some(((*p).to_owned(), json::parse(&text).ok()?))
            })
            .collect();
        let mut report = ValidationReport::new();
        let d = parse_files(&files, &mut report);
        assert!(report.is_ok(), "{report}");
        d
    }

    fn plan(seed: &str, n: usize) -> PopulationPlan {
        plan_population(&data(), Seed::from_text(seed), n).unwrap()
    }

    #[test]
    fn the_same_seed_gives_the_same_town_and_another_seed_a_different_one() {
        assert_eq!(plan("alpha", 14), plan("alpha", 14));
        assert_ne!(plan("alpha", 14), plan("beta", 14));
        assert_ne!(plan("alpha", 14), plan("alpha", 15));
    }

    #[test]
    fn every_resident_has_a_unique_name_a_known_occupation_and_a_household_and_sizes_add_up() {
        let d = data();
        for n in [0usize, 1, 5, 10, 14, 20, 50] {
            let p = plan("towns", n);
            assert_eq!(p.residents.len(), n);
            let names: BTreeSet<String> = p.residents.iter().map(ResidentPlan::full_name).collect();
            assert_eq!(names.len(), n, "names are unique at {n}");
            for r in &p.residents {
                assert!(d.occupations.contains_key(&r.occupation));
                assert_eq!(r.needs.len(), d.needs.len());
                assert!(r.needs.values().all(|v| (0..=1000).contains(v)));
                assert!((0..=999).contains(&r.variation));
            }
            let in_households: usize = p.households.iter().map(|h| h.members.len()).sum();
            assert_eq!(in_households, n);
            assert!(p
                .households
                .iter()
                .all(|h| (1..=4).contains(&h.members.len())));
            for (i, h) in p.households.iter().enumerate() {
                assert!(h.members.iter().all(|m| p.residents[*m].household == i));
                assert!(h.members.iter().all(|m| p.residents[*m].family == h.family));
            }
        }
    }

    #[test]
    fn starting_relationships_are_in_range_unique_and_cover_every_household_pair() {
        let p = plan("ties", 20);
        let mut seen = BTreeSet::new();
        for r in &p.relationships {
            assert!(r.a < r.b && seen.insert((r.a, r.b)));
            assert!((-1000..=1000).contains(&r.affinity));
        }
        for h in &p.households {
            for (i, a) in h.members.iter().enumerate() {
                for b in h.members.iter().skip(i + 1) {
                    let rel = p.relationships.iter().find(|r| (r.a, r.b) == (*a, *b));
                    assert!(
                        rel.is_some_and(|r| r.tie == Tie::Household && r.affinity >= 350),
                        "household members know each other well"
                    );
                }
            }
        }
        assert!(
            p.relationships.iter().any(|r| r.shared_memory),
            "a few shared memories exist"
        );
        assert!(
            p.relationships.iter().filter(|r| r.shared_memory).count()
                <= p.relationships.len() / 2 + 2,
            "only a few"
        );
    }

    #[test]
    fn too_many_residents_and_missing_data_are_refused_with_a_reason() {
        assert_eq!(
            plan_population(&data(), Seed::from_text("x"), 51).unwrap_err(),
            PlanError::TooMany(51)
        );
        let mut d = data();
        d.names.given.clear();
        assert_eq!(
            plan_population(&d, Seed::from_text("x"), 3).unwrap_err(),
            PlanError::NoNames
        );
        let mut d = data();
        d.occupations.clear();
        assert_eq!(
            plan_population(&d, Seed::from_text("x"), 3).unwrap_err(),
            PlanError::NoOccupations
        );
        assert!(plan_population(&GameData::default(), Seed::from_text("x"), 0).is_ok());
    }

    fn town(seed: &str, n: usize) -> (WorldState, Vec<EntityId>) {
        let d = data();
        let mut w = WorldState::new("Town", seed);
        let m = w.create_map(MapKind::Overworld, 20, 20).unwrap();
        let p = plan_population(&d, w.seed(), n).unwrap();
        let tiles: Vec<Tile> = (0..n as i32).map(|i| Tile::new(i % 20, i / 20)).collect();
        let ids = apply_population(&mut w, &d, &p, m, &PlacePlan::at(tiles)).unwrap();
        (w, ids)
    }

    #[test]
    fn applying_a_plan_makes_pawns_households_relationships_and_memories_that_agree() {
        let (w, _) = town("apply", 14);
        assert_eq!(w.pawns.len(), 14);
        let hh_members: usize = w.households.iter().map(|(_, h)| h.members.len()).sum();
        assert_eq!(hh_members, 14);
        for (hid, h) in w.households.iter() {
            for m in &h.members {
                assert_eq!(w.pawns.get(*m).unwrap().household, Some(hid));
            }
        }
        for rel in w.relationships.iter() {
            assert!(w.pawns.contains(rel.key.a) && w.pawns.contains(rel.key.b));
            assert!(!rel.label.is_empty());
        }
        let memories: usize = w.pawns.iter().map(|(_, p)| p.memories.len()).sum();
        assert!(
            memories > 0 && memories.is_multiple_of(2),
            "each shared memory has two owners"
        );
        for (id, p) in w.pawns.iter() {
            assert_eq!(p.needs.len(), 3);
            assert!(p.occupation.is_some());
            for m in &p.memories {
                assert!(
                    m.participants.iter().all(|o| *o != id),
                    "a memory names the other person"
                );
                assert!(m
                    .summary_key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("memory.start.")));
            }
        }
        // The state hashes the same when built twice, and survives a save and load.
        let (w2, _) = town("apply", 14);
        assert_eq!(w.state_hash(), w2.state_hash());
        let back = WorldState::from_canon(
            &crate::canon::json::parse(&w.to_canon().to_canonical_string()).unwrap(),
        )
        .unwrap();
        assert_eq!(back.state_hash(), w.state_hash());
        assert_eq!(back.relationships.len(), w.relationships.len());
    }

    #[test]
    fn occupations_may_only_use_registered_actions_and_unknown_ones_get_a_hint() {
        let actions = ActionRegistry::builtin();
        assert!(check_occupations(&data(), &actions).is_empty());
        let mut d = data();
        if let Some(o) = d.occupations.get_mut("barista") {
            if let Some(duty) = o.duties.first_mut() {
                duty.activity = "idel_at".into();
            }
            if let Some(l) = o.leisure.first_mut() {
                l.activity = "meet_at".into();
            }
        }
        let r = check_occupations(&d, &actions);
        let text = r.to_string();
        assert!(
            r.has_code("unknown_action") && text.contains("did you mean 'idle_at'"),
            "{text}"
        );
        assert!(r.has_code("commitment_action"), "{text}");
    }

    proptest::proptest! {
        #[test]
        fn any_seed_and_size_gives_a_consistent_town(seed in "[a-z0-9 ]{0,12}", n in 0usize..=50) {
            let d = data();
            let p = plan_population(&d, Seed::from_text(&seed), n).unwrap();
            proptest::prop_assert_eq!(p.residents.len(), n);
            let names: BTreeSet<String> = p.residents.iter().map(ResidentPlan::full_name).collect();
            proptest::prop_assert_eq!(names.len(), n);
            let sizes: usize = p.households.iter().map(|h| h.members.len()).sum();
            proptest::prop_assert_eq!(sizes, n);
            let mut pairs = BTreeSet::new();
            for r in &p.relationships {
                proptest::prop_assert!(r.a < r.b && r.b < n && pairs.insert((r.a, r.b)));
            }
            // And the plan applies to a world that hashes the same twice.
            let mut w = WorldState::new("T", &seed);
            let m = w.create_map(MapKind::Overworld, 10, 10).unwrap();
            let tiles: Vec<Tile> = (0..n as i32).map(|i| Tile::new(i % 10, i / 10)).collect();
            apply_population(&mut w, &d, &p, m, &PlacePlan::at(tiles)).unwrap();
            proptest::prop_assert_eq!(w.pawns.len(), n);
        }
    }

    #[test]
    fn too_few_tiles_changes_nothing() {
        let d = data();
        let mut w = WorldState::new("T", "s");
        let m = w.create_map(MapKind::Overworld, 10, 10).unwrap();
        let p = plan_population(&d, w.seed(), 5).unwrap();
        let before = w.state_hash();
        assert!(
            apply_population(&mut w, &d, &p, m, &PlacePlan::at(vec![Tile::new(0, 0)])).is_err()
        );
        assert_eq!(w.state_hash(), before);
    }
}
