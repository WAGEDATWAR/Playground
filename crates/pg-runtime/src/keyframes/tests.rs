use super::*;
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::map::Tile;
use pg_core::pipeline::{Cadence, Placement, System, SystemSlot, TickCtx};
use std::sync::atomic::{AtomicU64, Ordering};

fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

/// A sim, its input log and its keyframe ring, stepped together the way the session loop does.
struct Rig {
    factory: SimFactory,
    sim: Sim,
    ring: KeyframeRing,
    log: Vec<StampedInput>,
    logged: usize,
}

impl Rig {
    fn new(factory: SimFactory, interval: u64, capacity: usize) -> Rig {
        let mut sim = factory.new_sim(WorldState::new("Keyframes", "kf-seed"));
        let map = EntityId::new(Kind::Map, 1);
        sim.submit(
            0,
            cmd(Command::DevCreateMap {
                w: 36,
                h: 28,
                style: 1,
            }),
        )
        .unwrap();
        for i in 0..6 {
            sim.submit(
                0,
                cmd(Command::DevSpawnPawn {
                    map,
                    at: None,
                    name: format!("P{i}"),
                }),
            )
            .unwrap();
        }
        // Inputs scattered through the run, including ones submitted late.
        sim.submit(
            600,
            cmd(Command::DevMove {
                pawn: EntityId::new(Kind::Pawn, 1),
                to: Tile::new(20, 20),
            }),
        )
        .unwrap();
        sim.submit(4000, cmd(Command::DevNudge { amount: 77 }))
            .unwrap();
        Rig {
            factory,
            sim,
            ring: KeyframeRing::new(interval, capacity),
            log: Vec::new(),
            logged: 0,
        }
    }

    fn step(&mut self) {
        self.sim.step().unwrap();
        let applied = self.sim.applied_inputs();
        self.log.extend_from_slice(&applied[self.logged..]);
        self.logged = applied.len();
        if self.ring.due(self.sim.world().clock.tick()) {
            self.ring.push(Keyframe::capture(&self.sim, self.log.len()));
        }
    }

    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    fn submit_late(&mut self, tick: u64, c: Command) {
        self.sim.submit(tick, cmd(c)).unwrap();
    }
}

fn factory(threads: usize) -> SimFactory {
    SimFactory::dev(None, threads)
}

/// The state hash a straight run reaches at `tick`, with the same late inputs.
fn reference_hash(tick: u64, late: &[(u64, Command)]) -> StateHash {
    let mut r = Rig::new(factory(1), 1_000_000, 1);
    for (t, c) in late {
        r.submit_late(*t, c.clone());
    }
    r.run(tick);
    r.sim.world().state_hash()
}

#[test]
fn the_ring_takes_keyframes_on_the_interval_and_stays_bounded() {
    let mut r = Rig::new(factory(1), 500, 4);
    r.run(4_000);
    assert_eq!(r.ring.len(), 4);
    let ticks = r.ring.ticks();
    assert!(ticks.windows(2).all(|w| w[1] - w[0] >= 500), "{ticks:?}");
    assert_eq!(*ticks.last().unwrap(), r.ring.latest().unwrap().tick);
    assert!(
        ticks[0] > 500,
        "the oldest keyframes were dropped: {ticks:?}"
    );
    assert!(r.ring.previous().unwrap().tick < r.ring.latest().unwrap().tick);
    assert!(!r.ring.is_empty());
    assert_eq!(r.ring.at_or_before(ticks[1] + 1).unwrap().tick, ticks[1]);
    assert!(
        r.ring.at_or_before(1).is_none(),
        "nothing before the oldest kept keyframe"
    );
    r.ring.truncate_after(ticks[1]);
    assert_eq!(r.ring.latest().unwrap().tick, ticks[1]);
}

#[test]
fn rewinding_reproduces_the_exact_state_at_any_tick() {
    let late = [
        (2_500u64, Command::DevNudge { amount: -5 }),
        (
            9_000,
            Command::DevMove {
                pawn: EntityId::new(Kind::Pawn, 3),
                to: Tile::new(4, 4),
            },
        ),
    ];
    let mut r = Rig::new(factory(1), 1_800, 24);
    for (t, c) in &late {
        r.submit_late(*t, c.clone());
    }
    r.run(14_000);
    let final_hash = r.sim.world().state_hash();
    for target in [1_800, 1_801, 2_600, 4_000, 5_399, 9_000, 9_001, 13_999] {
        let rewound = rewind(&r.factory, &r.ring, &r.log, target).unwrap();
        assert_eq!(rewound.world().clock.tick(), target);
        assert_eq!(
            rewound.world().state_hash(),
            reference_hash(target, &late),
            "rewind to {target}"
        );
    }
    // The original run is untouched by all of that.
    assert_eq!(r.sim.world().state_hash(), final_hash);
    // And a rewound sim runs forward to the same final state as the original.
    let mut again = rewind(&r.factory, &r.ring, &r.log, 5_000).unwrap();
    for s in r.log.iter().filter(|s| {
        s.tick >= 5_000 && s.seq >= r.ring.at_or_before(5_000).unwrap().snapshot.next_seq
    }) {
        again.submit(s.tick, s.input.clone()).unwrap();
    }
    again.run_ticks(14_000 - 5_000).unwrap();
    assert_eq!(again.world().state_hash(), final_hash);
}

#[test]
fn rewinding_before_the_oldest_keyframe_is_an_error() {
    let mut r = Rig::new(factory(1), 1_000, 2);
    r.run(5_000);
    let first = r.ring.ticks()[0];
    assert!(matches!(
        rewind(&r.factory, &r.ring, &r.log, first - 1),
        Err(RewindError::NoKeyframe)
    ));
    assert!(rewind(&r.factory, &r.ring, &r.log, first).is_ok());
    assert!(RewindError::NoKeyframe.to_string().contains("keyframe"));
}

#[test]
fn shadow_verification_matches_across_thread_counts() {
    let mut r = Rig::new(factory(1), 1_500, 24);
    r.run(9_000);
    assert!(r.ring.len() >= 5);
    let frames: Vec<Keyframe> = (0..r.ring.len())
        .filter_map(|i| r.ring.at_or_before(r.ring.ticks()[i]).cloned())
        .collect();
    for pair in frames.windows(2) {
        for threads in [1, 2, 4, 8] {
            let out = verify_span(&r.factory.with_threads(threads), &pair[0], &r.log, &pair[1]);
            assert_eq!(
                out,
                ShadowOutcome::Match {
                    ticks: pair[1].tick - pair[0].tick
                },
                "{threads} threads, {} -> {}",
                pair[0].tick,
                pair[1].tick
            );
        }
    }
    assert_eq!(r.factory.with_threads(8).threads(), 8);
}

/// A deliberately broken system: its effect depends on a counter shared by every sim in the process,
/// i.e. on something that is not part of the simulation's inputs.
struct Leaky;

static LEAK: AtomicU64 = AtomicU64::new(0);

impl System for Leaky {
    fn id(&self) -> &str {
        "test.leaky"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        ctx.world.probe.value +=
            i64::try_from(LEAK.fetch_add(1, Ordering::SeqCst) % 7).unwrap_or(0);
    }
}

#[test]
fn shadow_verification_catches_hidden_nondeterminism_and_names_the_table() {
    let leaky = SimFactory::with_pipeline(
        Arc::new(|| {
            let mut p = Pipeline::new();
            pg_core::dev::install(&mut p);
            let _ = p.add_extension(
                SystemSlot::Maintenance,
                Placement::After,
                Cadence::Tick,
                Box::new(Leaky),
            );
            p
        }),
        None,
        1,
    );
    let mut r = Rig::new(leaky, 1_000, 8);
    r.run(3_000);
    let (a, b) = (
        r.ring.at_or_before(1_000).unwrap().clone(),
        r.ring.at_or_before(2_000).unwrap().clone(),
    );
    match verify_span(&r.factory.with_threads(2), &a, &r.log, &b) {
        ShadowOutcome::Mismatch {
            at_tick,
            tables,
            expected,
            actual,
        } => {
            assert_eq!(at_tick, b.tick);
            assert_ne!(expected, actual);
            assert_eq!(tables, ["probe"], "only the leaked state differs");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_span_whose_inputs_cannot_be_replayed_fails_cleanly() {
    let mut r = Rig::new(factory(1), 1_000, 8);
    r.run(2_500);
    let (a, b) = (
        r.ring.at_or_before(1_000).unwrap().clone(),
        r.ring.at_or_before(2_000).unwrap().clone(),
    );
    // A corrupt log: an input stamped before the keyframe cannot be submitted.
    let mut bad = r.log.clone();
    bad.push(StampedInput {
        tick: 0,
        seq: 10_000,
        input: cmd(Command::DevNudge { amount: 1 }),
    });
    let out = verify_span(&r.factory, &a, &bad, &b);
    assert!(
        matches!(out, ShadowOutcome::Failed(_)) || matches!(out, ShadowOutcome::Match { .. }),
        "{out:?}"
    );
}
