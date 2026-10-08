use super::*;
use crate::action::ActionRegistry;
use crate::commands::Command;
use crate::id::Kind;
use crate::input::SimInput;
use crate::map::MapKind;
use crate::pawn::{Route, Step, Task};
use crate::sim::Sim;
use crate::social::Memory;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};

fn content() -> Arc<ContentSet> {
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap())
}

fn data() -> Arc<GameData> {
    Arc::new(content().game().clone())
}

/// A system that does nothing, to switch the built-in behaviour off in a test.
struct Off(&'static str);

impl System for Off {
    fn id(&self) -> &str {
        self.0
    }
    fn run(&mut self, _ctx: &mut TickCtx<'_>) {}
}

fn life_pipeline() -> Pipeline {
    let mut pipeline = Pipeline::new();
    for slot in [
        SystemSlot::DayPlanner,
        SystemSlot::Commitment,
        SystemSlot::ReservationActivator,
        SystemSlot::TaskPlanner,
        SystemSlot::Movement,
        SystemSlot::Activity,
    ] {
        pipeline.set_builtin(slot, Box::new(Off(slot.name())));
    }
    pipeline.set_builtin(
        SystemSlot::Needs,
        Box::new(NeedsSystem::new(
            data(),
            Arc::new(ActionRegistry::builtin()),
        )),
    );
    pipeline.set_builtin(SystemSlot::Mood, Box::new(MoodSystem::new(data())));
    pipeline
}

/// A map with `n` pawns standing still and only the needs and mood systems installed (no plans), so a
/// test sees what the needs system does and nothing else.
fn quiet(n: usize) -> (Sim, Vec<EntityId>) {
    let mut w = WorldState::new("Life", "life-seed");
    let m = w.create_map(MapKind::Overworld, 16, 16).unwrap();
    let ids = (0..n)
        .map(|i| {
            w.spawn_pawn(&format!("P{i}"), m, Tile::new(2 + 2 * i as i32, 2))
                .unwrap()
        })
        .collect();
    (Sim::new(w, life_pipeline()), ids)
}

/// Runs `minutes` game minutes and returns the events (kind, detail) that were emitted.
fn run_minutes(sim: &mut Sim, minutes: u64) -> Vec<(String, Canon)> {
    let mut events = Vec::new();
    for _ in 0..minutes * TICKS_PER_GAME_MINUTE {
        let report = sim.step().unwrap();
        events.extend(report.events.into_iter().map(|e| (e.kind, e.detail)));
    }
    events
}

fn count(events: &[(String, Canon)], kind: &str) -> usize {
    events.iter().filter(|(k, _)| k == kind).count()
}

fn pawn(sim: &Sim, id: EntityId) -> &crate::pawn::Pawn {
    sim.world().pawns.get(id).unwrap()
}

/// Edits the world of a running sim through its snapshot (tests only).
fn set_world(sim: &mut Sim, f: impl FnOnce(&mut WorldState)) {
    let mut snap = sim.snapshot();
    f(&mut snap.world);
    snap.world.rebuild_derived();
    *sim = Sim::restore(snap, life_pipeline());
}

fn set_need(sim: &mut Sim, id: EntityId, need: &str, v: i32) {
    set_world(sim, |w| {
        w.pawns.get_mut(id).unwrap().needs.insert(need.into(), v);
    });
}

fn performing(action: &str) -> Task {
    Task {
        reservation: 1,
        action: pg_content::ActionId::new(action).unwrap(),
        steps: vec![Step::PerformUntil(u64::MAX)],
        current: 0,
        moving: false,
        interruptible: true,
    }
}

#[test]
fn any_sixty_minutes_add_up_to_exactly_the_hourly_rate() {
    for rate in [0u32, 1, 35, 55, 66, 100, 450, 1000] {
        for start in [0u64, 1, 7, 59, 60, 1439, 100_000] {
            let total: i32 = (start..start + 60).map(|m| per_minute(m, rate)).sum();
            assert_eq!(total, rate as i32, "rate {rate} from minute {start}");
        }
    }
    assert_eq!(decay_rate(55, 1200, false), 55);
    assert_eq!(decay_rate(55, 1200, true), 66);
    assert_eq!(decay_rate(50, 1300, true), 65);
}

#[test]
fn new_pawns_get_their_needs_a_home_and_steady_decay_and_walking_is_hungrier() {
    let (mut sim, ids) = quiet(2);
    let d = data();
    let [a, b] = [ids[0], ids[1]];
    set_world(&mut sim, |w| {
        w.pawns.get_mut(b).unwrap().route = Some(Route::to(Tile::new(9, 9)));
    });
    run_minutes(&mut sim, 60);
    let (pa, pb) = (pawn(&sim, a), pawn(&sim, b));
    assert_eq!(
        pa.home_tile,
        Some(Tile::new(2, 2)),
        "home defaults to where the pawn started"
    );
    // Exactly one hour at the data's rates: 55, 50 and 35 points an hour at rest.
    assert_eq!(pa.needs["hunger"], d.needs["hunger"].start - 55);
    assert_eq!(pa.needs["energy"], d.needs["energy"].start - 50);
    assert_eq!(pa.needs["social"], d.needs["social"].start - 35);
    // Walking costs 20 percent and 30 percent more hunger and energy, and the same social.
    assert_eq!(pb.needs["hunger"], d.needs["hunger"].start - 66);
    assert_eq!(pb.needs["energy"], d.needs["energy"].start - 65);
    assert_eq!(pb.needs["social"], d.needs["social"].start - 35);
}

#[test]
fn a_need_raises_urgent_then_critical_once_each_and_asks_for_a_replan() {
    let (mut sim, ids) = quiet(1);
    let id = ids[0];
    set_need(&mut sim, id, "hunger", 305);
    let events = run_minutes(&mut sim, 5);
    assert_eq!(
        count(&events, "need.urgent"),
        0,
        "still above urgent after 5 minutes"
    );
    let events = run_minutes(&mut sim, 5);
    assert_eq!(count(&events, "need.urgent"), 1, "{events:?}");
    let p = pawn(&sim, id);
    assert!(p
        .replan
        .as_ref()
        .is_some_and(|r| r.why.contains("hunger became urgent")));
    assert_eq!(p.mood, "uneasy");
    assert_eq!(
        p.capacities.manipulation, 900,
        "urgent hunger fumbles a little"
    );
    set_world(&mut sim, |w| {
        let p = w.pawns.get_mut(id).unwrap();
        p.replan = None;
        p.needs.insert("hunger".into(), 105);
    });
    let events = run_minutes(&mut sim, 10);
    assert_eq!(count(&events, "need.critical"), 1, "{events:?}");
    assert_eq!(
        count(&events, "need.urgent"),
        0,
        "already below urgent: not raised again"
    );
    let p = pawn(&sim, id);
    assert!(p
        .replan
        .as_ref()
        .is_some_and(|r| r.why.contains("critical")));
    assert_eq!(
        (
            p.capacities.manipulation,
            p.capacities.moving,
            p.capacities.consciousness
        ),
        (600, 750, 1000),
        "critical hunger is worse but does not knock a pawn out"
    );
}

#[test]
fn an_exhausted_pawn_collapses_in_place_recovers_slowly_and_does_not_flicker() {
    let (mut sim, ids) = quiet(1);
    let id = ids[0];
    set_world(&mut sim, |w| {
        let p = w.pawns.get_mut(id).unwrap();
        p.needs.insert("energy".into(), 82);
        p.task = Some(performing("idle_at"));
        p.route = Some(Route::to(Tile::new(9, 9)));
    });
    let events = run_minutes(&mut sim, 3);
    assert_eq!(count(&events, "pawn.collapsed"), 1, "{events:?}");
    let p = pawn(&sim, id);
    assert!(!p.capacities.can_act() && p.task.is_none() && p.route.is_none());
    assert_eq!(p.capacities.consciousness, 300);
    // While collapsed it recovers at the rest rate minus the decay (+50 an hour): back to "urgent" takes
    // about three and a half hours, and until then it stays down rather than flickering at the threshold.
    let events = run_minutes(&mut sim, 120);
    assert_eq!(count(&events, "pawn.recovered"), 0);
    assert!(!pawn(&sim, id).capacities.can_act());
    let mut woke = None;
    for minute in 0..180 {
        let events = run_minutes(&mut sim, 1);
        if count(&events, "pawn.recovered") == 1 {
            woke = Some(minute);
            break;
        }
    }
    assert!(woke.is_some(), "it recovered within the next three hours");
    let p = pawn(&sim, id);
    assert!(p.capacities.can_act());
    assert!(
        p.needs["energy"] >= 250,
        "woke at the urgent level: {}",
        p.needs["energy"]
    );
    assert!(p.replan.as_ref().is_some());
}

#[test]
fn performing_a_restoring_action_restores_and_company_actions_need_company() {
    let (mut sim, ids) = quiet(3);
    let [a, b, c] = [ids[0], ids[1], ids[2]];
    set_need(&mut sim, a, "hunger", 300);
    set_world(&mut sim, |w| {
        w.pawns.get_mut(a).unwrap().task = Some(performing("eat"));
        w.pawns.get_mut(b).unwrap().task = Some(performing("socialise"));
        w.pawns.get_mut(c).unwrap().task = Some(performing("socialise"));
    });
    run_minutes(&mut sim, 60);
    let d = data();
    let start = |need: &str| d.needs[need].start;
    // Eating: +700 an hour, -55 decay.
    assert_eq!(pawn(&sim, a).needs["hunger"], 300 + 645);
    // b and c stand two tiles apart (within company range): +300 -35.
    assert_eq!(pawn(&sim, b).needs["social"], start("social") + 265);
    assert_eq!(pawn(&sim, c).needs["social"], start("social") + 265);
    // Alone, the same action restores nothing.
    let (mut alone, ids) = quiet(1);
    set_world(&mut alone, |w| {
        w.pawns.get_mut(ids[0]).unwrap().task = Some(performing("socialise"));
    });
    run_minutes(&mut alone, 60);
    assert_eq!(pawn(&alone, ids[0]).needs["social"], start("social") - 35);
}

fn memory(sev: u8, impact: i32, tick: u64, n: u32) -> Memory {
    Memory::new(
        EntityId::new(Kind::Memory, n),
        "conversation",
        tick,
        vec![],
        sev,
        impact,
        60,
    )
}

#[test]
fn the_mood_rule_table_decides_in_order_and_starting_memories_do_not_count() {
    let (mut sim, ids) = quiet(1);
    let id = ids[0];
    let settle = |sim: &mut Sim| {
        run_minutes(sim, 1);
        pawn(sim, id).mood.clone()
    };
    set_world(&mut sim, |w| {
        let p = w.pawns.get_mut(id).unwrap();
        for n in ["hunger", "energy", "social"] {
            p.needs.insert(n.into(), 900);
        }
    });
    assert_eq!(settle(&mut sim), "content");
    // Social critical beats everything below it in the table.
    set_need(&mut sim, id, "social", 100);
    assert_eq!(settle(&mut sim), "lonely");
    // An urgent need makes a pawn uneasy; a bad memory beats that.
    set_need(&mut sim, id, "social", 900);
    set_need(&mut sim, id, "hunger", 200);
    assert_eq!(settle(&mut sim), "uneasy");
    let now = sim.world().clock.tick();
    set_world(&mut sim, |w| {
        w.pawns
            .get_mut(id)
            .unwrap()
            .memories
            .push(memory(3, -20, now, 1)); // importance 90
    });
    assert_eq!(settle(&mut sim), "sad", "a moderate bad memory");
    set_world(&mut sim, |w| {
        w.pawns
            .get_mut(id)
            .unwrap()
            .memories
            .push(memory(5, -40, now, 2)); // importance 250
    });
    assert_eq!(settle(&mut sim), "upset", "a strong one outranks it");
    // Six game hours later the memories no longer count.
    run_minutes(&mut sim, 6 * 60 + 1);
    assert_ne!(pawn(&sim, id).mood, "upset");
    // A good recent memory cheers a pawn whose needs are fine.
    set_world(&mut sim, |w| {
        let p = w.pawns.get_mut(id).unwrap();
        p.memories.clear();
        for n in ["hunger", "energy", "social"] {
            p.needs.insert(n.into(), 900);
        }
    });
    let now = sim.world().clock.tick();
    set_world(&mut sim, |w| {
        w.pawns
            .get_mut(id)
            .unwrap()
            .memories
            .push(memory(4, 30, now, 3)); // importance 160
    });
    assert_eq!(settle(&mut sim), "cheerful");
    // A memory from before the world began (tick 0) moves nobody.
    set_world(&mut sim, |w| {
        let p = w.pawns.get_mut(id).unwrap();
        p.memories.clear();
        p.memories.push(memory(4, 30, 0, 4));
    });
    assert_eq!(settle(&mut sim), "content");
}

#[test]
fn a_pawn_with_nothing_to_do_for_hours_gets_bored_and_a_busy_one_does_not() {
    let (mut sim, ids) = quiet(2);
    let [idle, busy] = [ids[0], ids[1]];
    // A little under 600 so "content" does not apply, and above urgent so nothing else does.
    for p in [idle, busy] {
        for n in ["hunger", "energy", "social"] {
            set_need(&mut sim, p, n, 640);
        }
    }
    set_world(&mut sim, |w| {
        w.pawns.get_mut(busy).unwrap().task = Some(performing("idle_at"));
    });
    run_minutes(&mut sim, 2 * 60);
    assert_eq!(pawn(&sim, idle).mood, "neutral");
    run_minutes(&mut sim, 90);
    assert_eq!(pawn(&sim, idle).mood, "bored", "idle for over three hours");
    assert_eq!(pawn(&sim, busy).mood, "neutral");
    assert!(pawn(&sim, busy).idle_since.is_none());
}

fn input(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

fn submit_town(sim: &mut Sim, pawns: usize, style: u8) {
    sim.submit(
        0,
        input(Command::DevCreateMap {
            w: 40,
            h: 30,
            style,
        }),
    )
    .unwrap();
    for i in 0..pawns {
        sim.submit(
            0,
            input(Command::DevSpawnPawn {
                map: EntityId::new(Kind::Map, 1),
                at: None,
                name: format!("P{i}"),
            }),
        )
        .unwrap();
    }
}

fn dev_town(pawns: usize, seed: &str) -> Sim {
    let mut sim = Sim::with_dev_systems(WorldState::new("Dev", seed)).with_content(content());
    submit_town(&mut sim, pawns, 1);
    sim
}

#[test]
fn the_plan_source_adds_the_days_routine_and_an_urgent_need_is_wanted_at_once() {
    let mut sim = dev_town(1, "plan");
    sim.run_ticks(12).unwrap();
    let id = EntityId::new(Kind::Pawn, 1);
    let plans = LifePlanSource::new(data());
    let w = sim.world();
    let slot = w.settings.slot_minutes.get();
    let inputs = plans.inputs(w, id, 0);
    let of =
        |n: &str| -> Vec<&UrgentNeed> { inputs.urgent.iter().filter(|u| u.need == n).collect() };
    // Three meals, two sleeps and three chances to be with people, each in its window and at its place.
    assert_eq!(
        (of("hunger").len(), of("energy").len(), of("social").len()),
        (3, 2, 3)
    );
    let breakfast = of("hunger")[0];
    assert_eq!(breakfast.action.as_str(), "eat");
    assert_eq!(
        (breakfast.earliest, breakfast.predicted_slot),
        (390 / slot, 540 / slot + 1)
    );
    assert_eq!(
        breakfast.len, 2,
        "40 minutes of eating is two 30-minute slots"
    );
    let home = w.pawns.get(id).unwrap().position.tile;
    assert_eq!(
        breakfast.params.get("at"),
        Some(&home.to_canon()),
        "meals are at home"
    );
    let night = of("energy")[0];
    assert_eq!(
        (night.action.as_str(), night.earliest, night.len),
        ("sleep", 0, 12)
    );
    assert_eq!(of("social")[0].action.as_str(), "socialise");
    // Planning later in the day leaves out the windows that are behind the pawn.
    let mut late = w.clone();
    late.clock = crate::time::Clock::from_tick(14 * 60 * TICKS_PER_GAME_MINUTE);
    let later = plans.inputs(&late, id, 0);
    assert!(later.urgent.iter().filter(|u| u.need == "hunger").count() < 3);
    // A need that is already urgent is wanted at once, on top of the routine.
    let mut w2 = w.clone();
    w2.pawns
        .get_mut(id)
        .unwrap()
        .needs
        .insert("hunger".into(), 120);
    let now_slot = w2.clock.minute_of_day() / slot;
    let inputs = plans.inputs(&w2, id, 0);
    let hungry: Vec<_> = inputs
        .urgent
        .iter()
        .filter(|u| u.need == "hunger")
        .collect();
    assert_eq!(hungry.len(), 4);
    assert!(hungry
        .iter()
        .any(|u| (u.earliest, u.predicted_slot) == (now_slot, now_slot)));
}

#[test]
fn residents_look_after_themselves_over_several_days() {
    // Open ground: crowded gathering places and one-tile corridors are a routing problem (pawn-aware
    // routing, milestone 1.3), not a needs problem, so this checks the needs loop on its own.
    let mut sim =
        Sim::with_dev_systems(WorldState::new("Dev", "look-after")).with_content(content());
    submit_town(&mut sim, 4, 0);
    let mut lowest = (1000, 1000, 1000);
    let (mut collapsed, mut ate, mut slept) = (0, 0, 0);
    for _ in 0..(4 * 1440) {
        for _ in 0..TICKS_PER_GAME_MINUTE {
            let r = sim.step().unwrap();
            for e in &r.events {
                match e.kind.as_str() {
                    "pawn.collapsed" => collapsed += 1,
                    "task.started" => match e.detail.get("action").and_then(Canon::as_str) {
                        Some("eat") => ate += 1,
                        Some("sleep") => slept += 1,
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        for (_, p) in sim.world().pawns.iter() {
            let g = |n: &str| p.needs.get(n).copied().unwrap_or(1000);
            lowest = (
                lowest.0.min(g("hunger")),
                lowest.1.min(g("energy")),
                lowest.2.min(g("social")),
            );
        }
    }
    assert_eq!(
        collapsed, 0,
        "nobody collapsed; lowest (hunger, energy, social) = {lowest:?}"
    );
    assert!(
        ate >= 30 && slept >= 16,
        "everyone ate and slept most days: ate {ate}, slept {slept}"
    );
    assert!(lowest.0 > 0 && lowest.1 > 0, "lowest {lowest:?}");
}

#[test]
fn life_is_deterministic_and_survives_a_snapshot() {
    let build = || {
        let mut s = Sim::new(WorldState::new("Dev", "determinism"), Pipeline::new())
            .with_content(content());
        submit_town(&mut s, 5, 1);
        s
    };
    let mut whole = build();
    whole.run_ticks(16_000).unwrap();
    let mut again = build();
    again.run_ticks(16_000).unwrap();
    assert_eq!(whole.world().state_hash(), again.world().state_hash());
    let mut first = build();
    first.run_ticks(7_000).unwrap();
    let mut resumed = Sim::restore(first.snapshot(), Pipeline::new()).with_content(content());
    resumed.run_ticks(9_000).unwrap();
    assert_eq!(resumed.world().state_hash(), whole.world().state_hash());
}

#[test]
fn the_places_resolve_sensibly_and_the_plaza_is_near_the_middle() {
    let mut sim = dev_town(1, "places");
    sim.run_ticks(2).unwrap();
    let w = sim.world();
    let id = EntityId::new(Kind::Pawn, 1);
    let map = EntityId::new(Kind::Map, 1);
    let middle = plaza(w, map).unwrap();
    assert!(w.is_passable(map, middle));
    assert!(middle.manhattan(Tile::new(20, 15)) <= 8, "{middle}");
    assert_eq!(resolve_place(w, id, PlaceKind::Gathering, 0), Some(middle));
    assert_eq!(
        resolve_place(w, id, PlaceKind::Home, 0),
        Some(w.pawns.get(id).unwrap().position.tile)
    );
    for kind in PlaceKind::ALL {
        assert!(resolve_place(w, id, kind, 3).is_some(), "{kind:?}");
    }
    assert!(resolve_place(w, EntityId::new(Kind::Pawn, 99), PlaceKind::Home, 0).is_none());
}
