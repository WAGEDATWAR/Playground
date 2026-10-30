use super::*;
use crate::activity::{CommitmentSystem, PlanSource};
use crate::map::MapKind;
use crate::pipeline::{Event, Pipeline, SystemSlot};
use crate::schedule::PlanInputs;
use crate::sim::Sim;
use crate::social::{PairKey, Relationship};
use crate::world::WorldState;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use std::collections::BTreeMap;
use std::sync::Arc;

fn data() -> Arc<GameData> {
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    let set = ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap();
    Arc::new(set.game().clone())
}

/// Everyone's day is open, so any stretch is free to meet in.
struct Open;

impl PlanSource for Open {
    fn id(&self) -> &str {
        "test.open"
    }
    fn inputs(&self, _w: &WorldState, _p: EntityId, _d: u64) -> PlanInputs {
        PlanInputs::default()
    }
}

/// `n` pawns on an open map; `bond(i, j)` gives the affinity of each pair (`None` for strangers).
fn town(n: usize, bond: impl Fn(usize, usize) -> Option<i32>) -> (Sim, Vec<EntityId>) {
    let data = data();
    let mut w = WorldState::new("t", "meetup-seed");
    let map = w.create_map(MapKind::Overworld, 24, 24).unwrap();
    let ids: Vec<EntityId> = (0..n)
        .map(|i| {
            let id = w
                .spawn_pawn(
                    &format!("P{i}"),
                    map,
                    Tile::new(1 + 2 * i as i32 % 20, 1 + 2 * (i as i32 / 10)),
                )
                .unwrap();
            w.pawns
                .get_mut(id)
                .unwrap()
                .needs
                .insert("social".into(), 300);
            id
        })
        .collect();
    let rp = data.relationships.as_ref().unwrap();
    for i in 0..n {
        for j in i + 1..n {
            if let Some(affinity) = bond(i, j) {
                w.relationships.set(Relationship {
                    key: PairKey::new(ids[i], ids[j]).unwrap(),
                    affinity,
                    label: rp.label_for(affinity).unwrap().id.clone(),
                    last_interaction_tick: 0,
                    day: 0,
                    day_change: 0,
                    last_topic: None,
                    forgotten: 0,
                });
            }
        }
    }
    let plans: Arc<dyn PlanSource> = Arc::new(Open);
    let mut pipeline = Pipeline::with_plan_source(plans);
    pipeline.set_builtin(
        SystemSlot::Commitment,
        Box::new(CommitmentSystem::new(Some(data))),
    );
    (Sim::new(w, pipeline), ids)
}

fn run_days(sim: &mut Sim, days: u64) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..days * TICKS_PER_DAY {
        all.extend(sim.step().unwrap().events);
    }
    all
}

fn of_kind<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

fn who(e: &Event, key: &str) -> EntityId {
    e.detail
        .get(key)
        .and_then(Canon::as_str)
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn friends_with_open_time_arrange_to_meet_and_the_meeting_happens() {
    let (mut sim, ids) = town(2, |_, _| Some(450));
    let events = run_days(&mut sim, 4);
    let asked = of_kind(&events, "commitment.proposed");
    assert!(
        !asked.is_empty(),
        "two friends with empty days meet up sometimes"
    );
    for e in &asked {
        let pair = [who(e, "proposer"), who(e, "invitee")];
        assert!(pair.contains(&ids[0]) && pair.contains(&ids[1]));
    }
    let kept = of_kind(&events, "commitment.completed");
    assert!(!kept.is_empty(), "at least one meeting was kept");
    // Keeping them left their mark on the pair.
    let rel = sim.world().relationships.get(ids[0], ids[1]).unwrap();
    assert!(rel.affinity > 450, "{}", rel.affinity);
    let remembered = sim
        .world()
        .pawns
        .get(ids[0])
        .unwrap()
        .memories
        .iter()
        .any(|m| m.summary_key.as_deref() == Some("memory.commitment.kept"));
    assert!(remembered);
}

#[test]
fn strangers_and_acquaintances_are_not_asked() {
    // Everyone knows everyone, but nobody is a friend.
    let (mut sim, _) = town(6, |_, _| Some(200));
    let events = run_days(&mut sim, 3);
    assert!(of_kind(&events, "commitment.proposed").is_empty());
}

#[test]
fn a_busy_town_stays_bounded_and_a_pair_is_not_asked_twice_in_a_day() {
    let (mut sim, ids) = town(14, |_, _| Some(600));
    let params = data().relationships.as_ref().unwrap().meetups.clone();
    let mut asks: BTreeMap<(EntityId, EntityId), Vec<u64>> = BTreeMap::new();
    let mut per_tick: BTreeMap<u64, u32> = BTreeMap::new();
    let mut peak_rows = 0usize;
    for _ in 0..8 * TICKS_PER_DAY {
        let report = sim.step().unwrap();
        for e in of_kind(&report.events, "commitment.proposed") {
            let (a, b) = (who(e, "proposer"), who(e, "invitee"));
            asks.entry(pair(a, b)).or_default().push(e.tick);
            *per_tick.entry(e.tick).or_default() += 1;
        }
        // No resident is tied up in two live plans at once.
        let mut live: BTreeMap<EntityId, u32> = BTreeMap::new();
        for (_, c) in sim.world().commitments.iter() {
            if !c.state.is_terminal() {
                for p in c.parties() {
                    *live.entry(p).or_default() += 1;
                }
            }
        }
        assert!(live.values().all(|n| *n <= 1), "{live:?}");
        peak_rows = peak_rows.max(sim.world().commitments.len());
    }
    assert!(!asks.is_empty());
    assert!(per_tick.values().all(|n| *n <= params.max_per_boundary));
    let cooldown = u64::from(params.cooldown_minutes) * crate::time::TICKS_PER_GAME_MINUTE;
    for (pair, ticks) in &asks {
        for w in ticks.windows(2) {
            assert!(
                w[1] - w[0] >= cooldown,
                "{pair:?} asked again too soon: {ticks:?}"
            );
        }
    }
    // Finished plans are pruned: the table holds a couple of days of plans, not eight.
    let per_day = (14 / 2) * (params.keep_days as usize + 1) * 4;
    assert!(peak_rows <= per_day, "{peak_rows} rows");
    let _ = ids;
}

#[test]
fn friends_the_resident_has_not_talked_to_for_longest_are_asked_first() {
    // Pawn 0 has two friends; the one talked to a long time ago is asked.
    let (sim, ids) = town(3, |i, j| (i == 0 || j == 0).then_some(500));
    let mut snap = sim.snapshot();
    snap.world
        .relationships
        .get_mut(ids[0], ids[1])
        .unwrap()
        .last_interaction_tick = 50_000;
    snap.world
        .relationships
        .get_mut(ids[0], ids[2])
        .unwrap()
        .last_interaction_tick = 10;
    let plans: Arc<dyn PlanSource> = Arc::new(Open);
    let mut pipeline = Pipeline::with_plan_source(plans);
    pipeline.set_builtin(
        SystemSlot::Commitment,
        Box::new(CommitmentSystem::new(Some(data()))),
    );
    let mut sim = Sim::restore(snap, pipeline);
    let events = run_days(&mut sim, 2);
    let first = of_kind(&events, "commitment.proposed")
        .into_iter()
        .find(|e| who(e, "proposer") == ids[0])
        .expect("pawn 0 asked someone");
    assert_eq!(who(first, "invitee"), ids[2]);
}

#[test]
fn the_same_seed_gives_the_same_asks() {
    let run = || {
        let (mut sim, _) = town(8, |_, _| Some(500));
        run_days(&mut sim, 3);
        sim.world().state_hash()
    };
    assert_eq!(run(), run());
}
