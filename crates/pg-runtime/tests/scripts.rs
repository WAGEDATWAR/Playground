//! Script packs under the runtime's determinism machinery: keyframes, rewind and shadow verification
//! rebuild every VM from scratch, so they are the strongest check that scripts keep no hidden state.

use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::{SimInput, StampedInput};
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use pg_runtime::keyframes::{
    rewind, verify_span, Keyframe, KeyframeRing, ShadowOutcome, SimFactory,
};
use std::sync::Arc;

fn root(p: &str) -> String {
    format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))
}

fn content() -> Arc<ContentSet> {
    let load = |dir: &str| load_pack(&DirPack::new(root(dir)), &Limits::default()).unwrap();
    Arc::new(
        ContentSet::build(
            vec![
                load("data/base"),
                load("packs/cookbook/caffeine"),
                load("packs/cookbook/evening_legs"),
                load("packs/cookbook/birthdays"),
            ],
            ComponentRegistry::builtin(),
        )
        .unwrap(),
    )
}

fn started(factory: &SimFactory) -> Sim {
    let mut sim = factory.new_sim(WorldState::new("Scripts", "script-seed"));
    let cmd = |c| SimInput::Command {
        actor: None,
        cmd: c,
    };
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 36,
            h: 28,
            style: 1,
        }),
    )
    .unwrap();
    for i in 0..8 {
        sim.submit(
            0,
            cmd(Command::DevSpawnPawn {
                map: EntityId::new(Kind::Map, 1),
                at: None,
                name: format!("P{i}"),
            }),
        )
        .unwrap();
    }
    sim
}

struct Run {
    sim: Sim,
    ring: KeyframeRing,
    log: Vec<StampedInput>,
    logged: usize,
}

fn run(factory: &SimFactory, ticks: u64) -> Run {
    let mut r = Run {
        sim: started(factory),
        ring: KeyframeRing::new(1_800, 32),
        log: Vec::new(),
        logged: 0,
    };
    for _ in 0..ticks {
        r.sim.step().unwrap();
        let applied = r.sim.applied_inputs();
        r.log.extend_from_slice(&applied[r.logged..]);
        r.logged = applied.len();
        if r.ring.due(r.sim.world().clock.tick()) {
            r.ring.push(Keyframe::capture(&r.sim, r.log.len()));
        }
    }
    r
}

#[test]
fn shadow_verification_matches_with_scripts_at_every_thread_count() {
    let factory = SimFactory::dev(Some(content()), 1);
    assert!(factory.runs_scripts());
    let r = run(&factory, 3 * 14_400);
    assert!(r.ring.len() >= 20);
    assert!(!r.sim.world().ext.is_empty(), "the packs wrote something");
    let ticks = r.ring.ticks();
    for pair in ticks.windows(2).step_by(5) {
        let (a, b) = (
            r.ring.at_or_before(pair[0]).unwrap(),
            r.ring.at_or_before(pair[1]).unwrap(),
        );
        for threads in [1, 2, 4] {
            let out = verify_span(&factory.with_threads(threads), a, &r.log, b);
            assert_eq!(
                out,
                ShadowOutcome::Match {
                    ticks: b.tick - a.tick
                },
                "{threads} threads"
            );
        }
    }
}

#[test]
fn rewinding_rebuilds_the_scripts_and_reaches_the_same_state() {
    let factory = SimFactory::dev(Some(content()), 2);
    let r = run(&factory, 2 * 14_400);
    let reference = run(&factory, 20_000).sim.world().state_hash();
    let rewound = rewind(&factory, &r.ring, &r.log, 20_000).unwrap();
    assert_eq!(rewound.world().state_hash(), reference);
}

#[test]
fn safe_mode_runs_the_same_world_without_touching_pack_data() {
    let factory = SimFactory::dev(Some(content()), 1);
    let r = run(&factory, 14_400);
    let packs_on = r.sim.world().ext.clone();
    let safe = factory.safe_mode();
    assert!(!safe.runs_scripts());
    let mut sim = safe.restore(r.sim.snapshot());
    sim.run_ticks(14_400).unwrap();
    assert_eq!(sim.world().ext, packs_on);
}
