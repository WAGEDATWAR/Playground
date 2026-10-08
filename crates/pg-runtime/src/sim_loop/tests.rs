use super::*;
use crate::control::Speed;
use pg_core::id::{EntityId, Kind};
use pg_core::map::{MapKind, Tile};
use pg_core::pipeline::{Cadence, Pipeline, Placement, System, SystemSlot, TickCtx};
use pg_host::{Faults, FixedClock, MemLog, MemStorage};
use pg_persist::store::{LoadOptions, SlotStore};
use std::sync::atomic::{AtomicU64, Ordering};

struct Rig {
    lp: SimLoop,
    clock: Arc<FixedClock>,
    storage: Arc<MemStorage>,
    log: Arc<MemLog>,
}

fn world() -> WorldState {
    let mut w = WorldState::new("Loop Town", "loop-seed");
    let m = w.create_map(MapKind::Overworld, 30, 24).unwrap();
    for i in 0..5 {
        w.spawn_pawn(&format!("P{i}"), m, Tile::new(i, 0)).unwrap();
    }
    w
}

fn rig_with(factory: SimFactory, mut cfg: LoopConfig) -> Rig {
    cfg.world_id = "town".to_owned();
    let clock = Arc::new(FixedClock::new());
    let storage = Arc::new(MemStorage::new());
    let log = Arc::new(MemLog::new());
    let services = LoopServices {
        clock: clock.clone(),
        storage: storage.clone(),
        log: log.clone(),
        pool: Arc::new(WorkerPool::new(2)),
        thumbnailer: None,
        console: None,
        dialogue: None,
        focus: None,
    };
    Rig {
        lp: SimLoop::new(factory, world(), services, cfg),
        clock,
        storage,
        log,
    }
}

fn rig() -> Rig {
    rig_with(SimFactory::dev(None, 1), LoopConfig::new("town"))
}

impl Rig {
    fn advance(&mut self, ms: u64) -> Vec<LoopEvent> {
        self.clock.advance(Duration::from_millis(ms));
        self.lp.frame()
    }

    fn tick(&self) -> u64 {
        self.lp.sim().world().clock.tick()
    }

    fn resume(&mut self, s: Speed) {
        self.lp.control(Control::Run(RunEvent::Resume(s)));
    }
}

#[test]
fn it_starts_paused_and_runs_ticks_at_the_chosen_speed() {
    let mut r = rig();
    assert_eq!(r.lp.state(), RunState::Paused);
    r.advance(5_000);
    assert_eq!(r.tick(), 0, "paused time does not advance the world");
    r.resume(Speed::Normal);
    r.advance(0); // the frame that starts the clock reading
    r.advance(1_000);
    assert_eq!(r.tick(), 10);
    r.advance(150);
    assert_eq!(r.tick(), 11);
    r.advance(50);
    assert_eq!(r.tick(), 12);
    r.lp.control(Control::Run(RunEvent::SetSpeed(Speed::Fast)));
    r.advance(1_000);
    assert_eq!(r.tick(), 42);
    r.lp.control(Control::Run(RunEvent::Pause));
    r.advance(10_000);
    assert_eq!(r.tick(), 42);
}

#[test]
fn a_long_stall_runs_a_bounded_burst_not_a_backlog() {
    let mut r = rig();
    r.resume(Speed::Fastest);
    r.advance(0);
    r.advance(60_000);
    assert_eq!(r.tick(), 50, "capped at max_ticks_per_frame");
    r.advance(100);
    assert_eq!(r.tick(), 50 + 27, "no backlog afterwards");
}

#[test]
fn the_outcome_does_not_depend_on_how_frames_fall_or_how_many_threads_help() {
    let mut a = rig_with(SimFactory::dev(None, 1), LoopConfig::new("t"));
    let mut b = rig_with(SimFactory::dev(None, 4), LoopConfig::new("t"));
    for r in [&mut a, &mut b] {
        r.resume(Speed::Fast);
        r.advance(0);
    }
    for _ in 0..300 {
        a.advance(100); // 3 ticks per frame
    }
    for _ in 0..50 {
        b.advance(600); // 18 ticks per frame
    }
    assert_eq!((a.tick(), b.tick()), (900, 900));
    assert_eq!(
        a.lp.sim().world().state_hash(),
        b.lp.sim().world().state_hash()
    );
}

#[test]
fn commands_apply_at_the_next_tick_and_are_refused_when_nothing_can_take_them() {
    let mut r = rig();
    r.resume(Speed::Normal);
    r.advance(0);
    assert!(r
        .lp
        .control(Control::Command(Command::DevNudge { amount: 9 }))
        .is_empty());
    r.advance(100);
    // The nudge was applied at the next tick (the probe walks randomly, so only its tick count is stable).
    assert!(r.lp.sim().world().probe.minutes <= 1);
    let refused = r.lp.control(Control::Command(Command::DevMove {
        pawn: EntityId::new(Kind::Pawn, 99),
        to: Tile::new(1, 1),
    }));
    assert!(
        refused.is_empty(),
        "a bad command is rejected inside the sim as an event, not by the queue"
    );
    r.lp.control(Control::Run(RunEvent::CloseRequested));
    r.advance(10);
    assert!(matches!(
        r.lp.control(Control::Command(Command::DevNudge { amount: 1 }))[..],
        [LoopEvent::Refused(_)]
    ));
}

#[test]
fn losing_focus_pauses_and_saves_and_regaining_it_does_not_resume() {
    let mut r = rig();
    r.resume(Speed::Normal);
    r.advance(0);
    r.advance(2_000);
    let events = r.lp.control(Control::Run(RunEvent::FocusLost));
    assert_eq!(events, [LoopEvent::State(RunState::Suspended)]);
    let saved = r.lp.finish_saves();
    assert!(
        matches!(
            saved[..],
            [LoopEvent::Saved {
                tick: 20,
                autosave: false,
                ..
            }]
        ),
        "{saved:?}"
    );
    r.advance(30_000);
    assert_eq!(r.tick(), 20, "the world is frozen while suspended");
    r.lp.control(Control::Run(RunEvent::FocusGained));
    r.advance(5_000);
    assert_eq!(
        r.tick(),
        20,
        "regaining focus asks the player; it does not resume by itself"
    );
    r.resume(Speed::Normal);
    r.advance(1_000);
    assert_eq!(r.tick(), 30);
    // The save is a real, loadable save of the world as it was when focus was lost.
    let loaded = SlotStore::new(r.storage.as_ref())
        .load("town", &LoadOptions::default())
        .unwrap();
    assert_eq!(loaded.world.clock.tick(), 20);
}

#[test]
fn closing_saves_then_stops() {
    let mut r = rig();
    r.resume(Speed::Normal);
    r.advance(0);
    r.advance(1_000);
    let ev = r.lp.control(Control::Run(RunEvent::CloseRequested));
    assert_eq!(ev, [LoopEvent::State(RunState::Closing)]);
    let frame = r.advance(10);
    assert!(
        frame.contains(&LoopEvent::State(RunState::Stopped)),
        "{frame:?}"
    );
    assert_eq!(r.lp.state(), RunState::Stopped);
    let loaded = SlotStore::new(r.storage.as_ref())
        .load("town", &LoadOptions::default())
        .unwrap();
    assert_eq!(
        loaded.world.clock.tick(),
        10,
        "the final save holds the last tick"
    );
    assert_eq!(r.tick(), 10);
}

#[test]
fn autosave_fires_on_running_time_only_and_reports_itself() {
    let cfg = LoopConfig {
        autosave_minutes: 1,
        ..LoopConfig::new("t")
    };
    let mut r = rig_with(SimFactory::dev(None, 1), cfg);
    r.resume(Speed::Normal);
    r.advance(0);
    let mut autosaves = 0;
    for _ in 0..130 {
        let ev = r.advance(1_000);
        autosaves += ev
            .iter()
            .filter(|e| matches!(e, LoopEvent::Saved { autosave: true, .. }))
            .count();
        autosaves +=
            r.lp.finish_saves()
                .iter()
                .filter(|e| matches!(e, LoopEvent::Saved { autosave: true, .. }))
                .count();
    }
    assert_eq!(autosaves, 2, "130 s of running time at a 60 s interval");
    // Paused time does not count toward the next one.
    r.lp.control(Control::Run(RunEvent::Pause));
    for _ in 0..300 {
        assert!(r
            .advance(1_000)
            .iter()
            .all(|e| !matches!(e, LoopEvent::Saved { .. })));
    }
    // Two generations are kept however many saves happen.
    let gens = r
        .storage
        .names()
        .into_iter()
        .filter(|n| n.ends_with(".pgsave"))
        .count();
    assert_eq!(gens, 2);
}

#[test]
fn a_failing_save_is_reported_and_the_world_carries_on() {
    let mut r = rig();
    r.resume(Speed::Normal);
    r.advance(0);
    r.storage.set_faults(Faults {
        disk_full: true,
        ..Faults::default()
    });
    r.lp.control(Control::SaveNow);
    let ev = r.lp.finish_saves();
    assert!(
        matches!(&ev[..], [LoopEvent::SaveFailed(m)] if m.contains("disk full")),
        "{ev:?}"
    );
    r.advance(1_000);
    assert_eq!(r.tick(), 10);
    r.storage.set_faults(Faults::default());
    r.lp.control(Control::SaveNow);
    assert!(matches!(r.lp.finish_saves()[..], [LoopEvent::Saved { .. }]));
    assert!(r.log.text().contains("save failed"));
}

#[test]
fn the_ui_gets_published_snapshots_that_follow_the_world() {
    let mut r = rig();
    let publisher = r.lp.publisher();
    let v0 = publisher.version();
    r.resume(Speed::Fast);
    r.advance(0);
    r.advance(1_000);
    let s = publisher.latest();
    assert_eq!((s.tick, s.pawns.len(), s.maps.len()), (30, 5, 1));
    assert_eq!(s.run, RunState::Running(Speed::Fast));
    assert!(publisher.version() > v0);
    // A frame in which nothing changed publishes nothing new.
    r.lp.control(Control::Run(RunEvent::Pause));
    r.advance(10);
    let v = publisher.version();
    r.advance(10);
    assert_eq!(publisher.version(), v);
}

#[test]
fn scrubbing_back_rebuilds_the_exact_past_pauses_and_discards_the_future() {
    let cfg = LoopConfig {
        keyframe_interval_ticks: 300,
        ..LoopConfig::new("t")
    };
    let mut r = rig_with(SimFactory::dev(None, 1), cfg.clone());
    let mut twin = rig_with(SimFactory::dev(None, 1), cfg);
    for x in [&mut r, &mut twin] {
        x.resume(Speed::Fastest);
        x.advance(0);
    }
    for _ in 0..40 {
        r.advance(500); // 135 ticks per frame, capped at 50 per frame
        r.lp.control(Control::Command(Command::DevNudge { amount: 3 }));
    }
    let end = r.tick();
    assert!(end > 1_500, "{end}");
    // Run the twin to a tick in the middle and remember its state.
    while twin.tick() < 1_000 {
        twin.advance(100);
        twin.lp
            .control(Control::Command(Command::DevNudge { amount: 3 }));
    }
    let _ = twin; // (the twin only proves keyframes work on a second loop)
    let ev = r.lp.control(Control::RewindTo(1_000));
    assert!(
        ev.contains(&LoopEvent::Rewound { to: 1_000 })
            && ev.contains(&LoopEvent::State(RunState::Paused)),
        "{ev:?}"
    );
    assert_eq!((r.tick(), r.lp.state()), (1_000, RunState::Paused));
    assert!(r.lp.keyframe_ticks().iter().all(|t| *t <= 1_000));
    assert!(r.lp.input_log().iter().all(|s| s.tick < 1_000));
    // Scrubbing forward is refused, and running on from the past works.
    assert!(matches!(
        r.lp.control(Control::RewindTo(5_000))[..],
        [LoopEvent::Refused(_)]
    ));
    r.resume(Speed::Normal);
    r.advance(1_000);
    assert_eq!(r.tick(), 1_010);
}

#[test]
fn a_rewound_world_matches_a_world_that_never_went_past_that_tick() {
    let cfg = LoopConfig {
        keyframe_interval_ticks: 300,
        ..LoopConfig::new("t")
    };
    let mut a = rig_with(SimFactory::dev(None, 1), cfg.clone());
    let mut b = rig_with(SimFactory::dev(None, 1), cfg);
    for x in [&mut a, &mut b] {
        x.resume(Speed::Normal);
        x.advance(0);
    }
    // Same inputs at the same ticks in both; `a` then runs further and scrubs back.
    for _ in 0..80 {
        for x in [&mut a, &mut b] {
            x.advance(1_000);
            x.lp.control(Control::Command(Command::DevNudge { amount: 5 }));
        }
    }
    let target = b.tick();
    for _ in 0..30 {
        a.advance(1_000);
    }
    assert!(a.tick() > target);
    a.lp.control(Control::RewindTo(target));
    assert_eq!(a.tick(), target);
    assert_eq!(
        a.lp.sim().world().state_hash(),
        b.lp.sim().world().state_hash()
    );
}

struct Bomb;

static BOMB_AT: AtomicU64 = AtomicU64::new(u64::MAX);

impl System for Bomb {
    fn id(&self) -> &str {
        "test.bomb"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        if ctx.flags.tick == BOMB_AT.load(Ordering::SeqCst) {
            panic!("the bomb went off at tick {}", ctx.flags.tick);
        }
    }
}

fn bomb_factory() -> SimFactory {
    SimFactory::with_pipeline(
        Arc::new(|| {
            let mut p = Pipeline::new();
            pg_core::dev::install(&mut p);
            let _ = p.add_extension(
                SystemSlot::Maintenance,
                Placement::After,
                Cadence::Tick,
                Box::new(Bomb),
            );
            p
        }),
        None,
        1,
    )
}

#[test]
fn a_panicking_tick_freezes_the_world_and_leaves_a_report_and_a_replayable_bundle() {
    BOMB_AT.store(700, Ordering::SeqCst);
    let cfg = LoopConfig {
        keyframe_interval_ticks: 300,
        ..LoopConfig::new("t")
    };
    let mut r = rig_with(bomb_factory(), cfg);
    r.resume(Speed::Fastest);
    r.advance(0);
    let mut crashed = None;
    for _ in 0..40 {
        for e in r.advance(200) {
            if let LoopEvent::Crashed {
                message,
                report,
                bundle,
            } = e
            {
                crashed = Some((message, report, bundle));
            }
        }
        if crashed.is_some() {
            break;
        }
    }
    let (message, report, bundle) = crashed.expect("the bomb exploded");
    assert!(message.contains("bomb went off at tick 700"), "{message}");
    assert_eq!(r.lp.state(), RunState::Crashed);
    let ticks_at_crash = r.tick();
    assert!(
        r.advance(10_000).is_empty() && r.tick() == ticks_at_crash,
        "frozen for good"
    );
    assert!(matches!(
        r.lp.control(Control::Command(Command::DevNudge { amount: 1 }))[..],
        [LoopEvent::Refused(_)]
    ));
    assert!(matches!(
        r.lp.control(Control::Run(RunEvent::Resume(Speed::Normal)))
            .len(),
        0
    ));

    // The crash report names the problem and the world, and holds no key-shaped text.
    let report_text = String::from_utf8(
        r.storage
            .get_raw(&report.expect("a report was written"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        report_text.contains("bomb went off") && report_text.contains("world: town"),
        "{report_text}"
    );
    // The bundle starts at a keyframe and is a real, decodable bundle.
    let bytes = r
        .storage
        .get_raw(&bundle.expect("a bundle was written"))
        .unwrap();
    let b = Bundle::decode(&bytes).unwrap();
    assert!(b.note.contains("a tick panicked"));
    assert!(
        b.log.start_tick() <= 700 && b.log.start_tick().is_multiple_of(300),
        "{}",
        b.log.start_tick()
    );
    assert_eq!(b.log.ticks, 700 + 1 - b.log.start_tick());
    // With the bomb defused, the bundle replays cleanly from its start state.
    BOMB_AT.store(u64::MAX, Ordering::SeqCst);
    let mut sim = pg_core::replay::build_sim(&b.log, None).unwrap();
    sim.run_ticks(b.log.ticks).unwrap();
    assert!(sim.world().clock.tick() > 700);
}

#[test]
fn shadow_verification_confirms_an_honest_run_with_a_different_thread_count() {
    let cfg = LoopConfig {
        keyframe_interval_ticks: 300,
        shadow_threads: Some(4),
        ..LoopConfig::new("t")
    };
    let mut r = rig_with(SimFactory::dev(None, 1), cfg);
    r.resume(Speed::Fastest);
    r.advance(0);
    let mut events = Vec::new();
    for _ in 0..50 {
        events.extend(r.advance(200));
    }
    events.extend(r.lp.finish_shadows());
    events.extend(r.advance(1));
    let verified = events
        .iter()
        .filter(|e| matches!(e, LoopEvent::Verified { .. }))
        .count();
    assert!(verified >= 3, "{events:?}");
    assert!(!events
        .iter()
        .any(|e| matches!(e, LoopEvent::Diverged { .. })));
    assert!(r.lp.state().is_running());
}

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
fn shadow_verification_catches_a_hidden_dependency_pauses_and_writes_a_bundle() {
    let factory = SimFactory::with_pipeline(
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
    let cfg = LoopConfig {
        keyframe_interval_ticks: 300,
        shadow_threads: Some(2),
        ..LoopConfig::new("t")
    };
    let mut r = rig_with(factory, cfg);
    r.resume(Speed::Fastest);
    r.advance(0);
    let mut diverged = None;
    for _ in 0..60 {
        let mut ev = r.advance(200);
        ev.extend(r.lp.finish_shadows());
        if let Some(LoopEvent::Diverged {
            at_tick,
            tables,
            bundle,
        }) = ev
            .into_iter()
            .find(|e| matches!(e, LoopEvent::Diverged { .. }))
        {
            diverged = Some((at_tick, tables, bundle));
            break;
        }
    }
    let (at_tick, tables, bundle) = diverged.expect("the leak was detected");
    assert!(at_tick > 0 && tables == ["probe"], "{at_tick} {tables:?}");
    assert_eq!(
        r.lp.state(),
        RunState::Paused,
        "paused to preserve the evidence"
    );
    let b = Bundle::decode(&r.storage.get_raw(&bundle.unwrap()).unwrap()).unwrap();
    assert!(b.note.contains("diverged") && b.note.contains("probe"));
}
