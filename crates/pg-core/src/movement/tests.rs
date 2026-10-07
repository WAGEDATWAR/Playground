use super::*;
use crate::input::{Command, SimInput};
use crate::map::{MapKind, WATER};
use crate::path::{BatchExecutor, PathJob, PathOutcome, SerialExecutor};
use crate::pipeline::Pipeline;
use crate::sim::Sim;
use crate::time::TICKS_PER_DAY;
use crate::world::WorldState;

fn t(x: i32, y: i32) -> Tile {
    Tile::new(x, y)
}

/// A world with one open map, and a sim that runs only the built-in pipeline (no dev wandering).
fn setup(w: i32, h: i32) -> (Sim, EntityId) {
    let mut world = WorldState::new("t", "seed");
    let map = world.create_map(MapKind::Overworld, w, h).unwrap();
    (Sim::new(world, Pipeline::new()), map)
}

fn spawn(sim: &mut Sim, map: EntityId, x: i32, y: i32, name: &str) -> EntityId {
    // Pawns are added through a snapshot so tests can place them before stepping.
    let mut snap = sim.snapshot();
    let id = snap.world.spawn_pawn(name, map, t(x, y)).unwrap();
    *sim = Sim::restore(snap, Pipeline::new());
    id
}

fn go(sim: &mut Sim, pawn: EntityId, x: i32, y: i32) {
    sim.submit_now(SimInput::Command {
        actor: None,
        cmd: Command::DevMove { pawn, to: t(x, y) },
    })
    .unwrap();
}

fn edit_blocked(sim: &mut Sim, map: EntityId, x: i32, y: i32, blocked: bool) {
    sim.submit_now(SimInput::Command {
        actor: None,
        cmd: Command::DevSetBlocked {
            map,
            at: t(x, y),
            blocked,
        },
    })
    .unwrap();
}

fn pos(sim: &Sim, pawn: EntityId) -> Tile {
    sim.world().pawns.get(pawn).unwrap().position.tile
}

fn walking(sim: &Sim, pawn: EntityId) -> bool {
    sim.world().pawns.get(pawn).unwrap().route.is_some()
}

/// Occupancy must always match the pawn table exactly, and no two pawns may share a tile.
fn assert_consistent(sim: &Sim) {
    let w = sim.world();
    let mut rebuilt = w.clone();
    rebuilt.rebuild_derived();
    assert_eq!(
        w.occupancy,
        rebuilt.occupancy,
        "derived occupancy drifted from the pawn table at tick {}",
        w.clock.tick()
    );
    let mut seen = std::collections::BTreeSet::new();
    for (_, p) in w.pawns.iter() {
        assert!(
            seen.insert((p.position.map, p.position.tile)),
            "two pawns on one tile at tick {}",
            w.clock.tick()
        );
    }
}

/// Steps until `done` or `limit` steps; returns the steps taken and every event kind seen.
fn run_until(sim: &mut Sim, limit: u64, done: impl Fn(&Sim) -> bool) -> (u64, Vec<String>) {
    let mut kinds = Vec::new();
    for n in 0..limit {
        let r = sim.step().unwrap();
        kinds.extend(r.events.into_iter().map(|e| e.kind));
        assert_consistent(sim);
        if done(sim) {
            return (n + 1, kinds);
        }
    }
    (limit, kinds)
}

fn failure_reason(events: &[crate::pipeline::Event]) -> Option<String> {
    events
        .iter()
        .find(|e| e.kind == "move.failed")
        .and_then(|e| e.detail.get("reason"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

#[test]
fn a_pawn_walks_a_straight_route_at_two_ticks_per_tile() {
    let (mut sim, map) = setup(20, 1);
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    go(&mut sim, a, 10, 0);
    let (steps, kinds) = run_until(&mut sim, 200, |s| !walking(s, a));
    assert_eq!(pos(&sim, a), t(10, 0));
    // One tick to apply the request and solve, then a step every second tick: tile k on step 2k-1.
    assert_eq!(steps, 20);
    assert!(kinds.contains(&"move.arrived".to_owned()));
    assert!(!kinds.contains(&"move.failed".to_owned()));
    assert_eq!(sim.world().pawns.get(a).unwrap().facing, Dir4::E);
}

#[test]
fn move_ticks_per_tile_is_a_world_setting() {
    let mut world = WorldState::new("t", "seed");
    let map = world.create_map(MapKind::Overworld, 20, 1).unwrap();
    world.settings.movement.move_ticks_per_tile = 5;
    let mut sim = Sim::new(world, Pipeline::new());
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    go(&mut sim, a, 10, 0);
    let (steps, _) = run_until(&mut sim, 500, |s| !walking(s, a));
    assert_eq!(steps, 50);
}

#[test]
fn already_at_the_goal_arrives_immediately() {
    let (mut sim, map) = setup(5, 5);
    let a = spawn(&mut sim, map, 2, 2, "Ann");
    go(&mut sim, a, 2, 2);
    let (steps, kinds) = run_until(&mut sim, 10, |s| !walking(s, a));
    assert_eq!(steps, 1);
    assert!(kinds.contains(&"move.arrived".to_owned()));
}

#[test]
fn the_lower_id_claims_a_contested_tile_and_the_other_waits() {
    let (mut sim, map) = setup(3, 3);
    let a = spawn(&mut sim, map, 1, 0, "Ann"); // pawn_1 wants (1,2) through (1,1)
    let b = spawn(&mut sim, map, 0, 1, "Bob"); // pawn_2 wants (2,1) through (1,1)
    assert!(a < b);
    go(&mut sim, a, 1, 2);
    go(&mut sim, b, 2, 1);
    // Both become ready to enter (1,1) on the same tick; run to that tick.
    run_until(&mut sim, 2, |_| false);
    assert_eq!(
        (pos(&sim, a), pos(&sim, b)),
        (t(1, 1), t(0, 1)),
        "the lower id moved first, the other waited"
    );
    let (_, kinds) = run_until(&mut sim, 100, |s| !walking(s, a) && !walking(s, b));
    assert_eq!((pos(&sim, a), pos(&sim, b)), (t(1, 2), t(2, 1)));
    assert!(
        !kinds.contains(&"move.sidestep".to_owned()) && !kinds.contains(&"move.failed".to_owned()),
        "{kinds:?}"
    );
}

#[test]
fn a_head_on_meeting_with_room_resolves_by_sidestepping() {
    let (mut sim, map) = setup(7, 2);
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    let b = spawn(&mut sim, map, 6, 0, "Bob");
    go(&mut sim, a, 6, 0);
    go(&mut sim, b, 0, 0);
    let (steps, kinds) = run_until(&mut sim, 600, |s| !walking(s, a) && !walking(s, b));
    assert!(steps < 600, "both must get through: {kinds:?}");
    assert_eq!((pos(&sim, a), pos(&sim, b)), (t(6, 0), t(0, 0)));
    assert!(
        kinds.contains(&"move.sidestep".to_owned()),
        "someone had to step aside"
    );
    assert!(!kinds.contains(&"move.failed".to_owned()), "{kinds:?}");
    let sidesteps = kinds.iter().filter(|k| *k == "move.sidestep").count();
    assert!(
        sidesteps <= 2,
        "a lateral sidestep should resolve it quickly, not {sidesteps} dances"
    );
}

#[test]
fn a_head_on_deadlock_in_a_one_tile_corridor_fails_instead_of_hanging() {
    let (mut sim, map) = setup(7, 1);
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    let b = spawn(&mut sim, map, 6, 0, "Bob");
    go(&mut sim, a, 6, 0);
    go(&mut sim, b, 0, 0);
    let (steps, kinds) = run_until(&mut sim, 1000, |s| !walking(s, a) && !walking(s, b));
    assert!(steps < 1000, "the deadlock must resolve");
    assert!(kinds.iter().any(|k| k == "move.failed"), "{kinds:?}");
}

#[test]
fn a_map_edit_under_the_route_causes_a_repath_around_it() {
    let (mut sim, map) = setup(12, 3);
    let a = spawn(&mut sim, map, 0, 1, "Ann");
    go(&mut sim, a, 11, 1);
    run_until(&mut sim, 7, |_| false); // three tiles: one tick to solve, then a step every second tick
    let before = pos(&sim, a);
    assert_eq!(before, t(3, 1), "the edit must land ahead of the pawn");
    edit_blocked(&mut sim, map, 6, 1, true);
    edit_blocked(&mut sim, map, 5, 1, true);
    let (_, kinds) = run_until(&mut sim, 400, |s| !walking(s, a));
    assert_eq!(pos(&sim, a), t(11, 1), "{kinds:?}");
    assert!(
        kinds.contains(&"world_edited".to_owned()) && kinds.contains(&"move.arrived".to_owned())
    );
    assert!(!kinds.contains(&"move.failed".to_owned()));
}

#[test]
fn a_goal_that_becomes_blocked_fails_with_blocked_destination() {
    let (mut sim, map) = setup(12, 1);
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    go(&mut sim, a, 11, 0);
    run_until(&mut sim, 6, |_| false);
    edit_blocked(&mut sim, map, 11, 0, true);
    let mut reason = None;
    for _ in 0..200 {
        let r = sim.step().unwrap();
        if let Some(why) = failure_reason(&r.events) {
            reason = Some(why);
            break;
        }
    }
    assert_eq!(reason.as_deref(), Some("blocked_destination"));
    assert!(!walking(&sim, a), "a failed route is cleared");
}

#[test]
fn an_unreachable_goal_fails_with_unreachable_or_too_far() {
    let (mut sim, map) = setup(9, 3);
    {
        let mut snap = sim.snapshot();
        let m = snap.world.maps.get_mut(map).unwrap();
        for y in 0..3 {
            m.set_terrain(t(4, y), WATER).unwrap();
        }
        sim = Sim::restore(snap, Pipeline::new());
    }
    let a = spawn(&mut sim, map, 0, 1, "Ann");
    go(&mut sim, a, 8, 1);
    let r = sim.step().unwrap();
    assert_eq!(
        failure_reason(&r.events).as_deref(),
        Some("unreachable_or_too_far")
    );
    assert_eq!(
        pos(&sim, a),
        t(0, 1),
        "a pawn that cannot find a path stays where it is"
    );
}

#[test]
fn dev_move_to_an_impassable_tile_is_rejected() {
    let (mut sim, map) = setup(5, 5);
    let a = spawn(&mut sim, map, 0, 0, "Ann");
    go(&mut sim, a, 9, 9);
    let r = sim.step().unwrap();
    assert!(r.events.iter().any(|e| e.kind == "input_rejected"));
    assert!(!walking(&sim, a));
}

#[test]
fn a_pawn_blocked_by_a_stationary_pawn_ends_up_failing_or_arriving_but_never_hanging() {
    let (mut sim, map) = setup(11, 3);
    let _statue = spawn(&mut sim, map, 5, 1, "Statue"); // never moves
    let a = spawn(&mut sim, map, 0, 1, "Ann");
    go(&mut sim, a, 10, 1);
    let (steps, kinds) = run_until(&mut sim, 3000, |s| !walking(s, a));
    assert!(steps < 3000, "must terminate");
    assert!(
        kinds
            .iter()
            .any(|k| k == "move.arrived" || k == "move.failed"),
        "{kinds:?}"
    );
}

struct Threaded(usize);

impl BatchExecutor for Threaded {
    fn solve(&self, jobs: &[PathJob<'_>]) -> Vec<PathOutcome> {
        let chunk = jobs.len().div_ceil(self.0.max(1)).max(1);
        std::thread::scope(|s| {
            let handles: Vec<_> = jobs
                .chunks(chunk)
                .map(|part| s.spawn(move || part.iter().map(PathJob::solve).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        })
    }
}

fn crowd(seed: &str, w: i32, h: i32, pawns: u32) -> Sim {
    let mut sim = Sim::with_dev_systems(WorldState::new("Town", seed));
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::DevCreateMap { w, h, style: 1 },
        },
    )
    .unwrap();
    for i in 0..pawns {
        sim.submit(
            0,
            SimInput::Command {
                actor: None,
                cmd: Command::DevSpawnPawn {
                    map: EntityId::new(crate::id::Kind::Map, 1),
                    at: None,
                    name: format!("P{i}"),
                },
            },
        )
        .unwrap();
    }
    sim
}

#[test]
fn walking_is_identical_with_a_threaded_executor() {
    let build = |exec: Box<dyn BatchExecutor>| {
        let mut sim = crowd("threads", 48, 36, 14);
        sim.set_executor(exec);
        sim.run_ticks(TICKS_PER_DAY / 4).unwrap();
        sim
    };
    let serial = build(Box::new(SerialExecutor));
    let walkers = serial
        .world()
        .pawns
        .iter()
        .filter(|(_, p)| p.route.is_some())
        .count();
    assert!(walkers > 0, "the wanderers should be walking");
    for n in [1, 2, 4, 7] {
        let threaded = build(Box::new(Threaded(n)));
        assert_eq!(
            threaded.world().state_hash(),
            serial.world().state_hash(),
            "{n} threads"
        );
    }
}

#[test]
fn many_wanderers_keep_occupancy_consistent_and_replay_identically() {
    let mut a = crowd("crowd", 36, 36, 18);
    for _ in 0..3000 {
        a.step().unwrap();
        assert_consistent(&a);
    }
    let mut b = crowd("crowd", 36, 36, 18);
    b.run_ticks(3000).unwrap();
    assert_eq!(a.world().state_hash(), b.world().state_hash());
    assert_eq!(a.world().pawns.len(), 18);
    // A mid-run snapshot resumes identically (route state is part of the saved state).
    let mut c = crowd("crowd", 36, 36, 18);
    c.run_ticks(1234).unwrap();
    let mut pipeline = Pipeline::new();
    crate::dev::install(&mut pipeline);
    let mut d = Sim::restore(c.snapshot(), pipeline);
    d.run_ticks(3000 - 1234).unwrap();
    assert_eq!(d.world().state_hash(), a.world().state_hash());
}
