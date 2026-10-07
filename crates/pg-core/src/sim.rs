//! The simulation driver: one `step()` is one tick (Blueprint §6.2).
//!
//! 1. Apply the inputs stamped with the current tick, in `(kind, actor, seq)` order.
//! 2. Advance the clock and compute the time-boundary flags for the tick entered.
//! 3. Run the pipeline's due systems in their fixed order.
//! 4. On a day boundary, record the state hash.
//! 5. Return a [`TickReport`].
//!
//! `Sim` owns no clock and no randomness source; the runtime decides when to call `step()`. It also owns
//! the derived services systems use (the path cache and the batch executor) and, optionally, the loaded
//! content that commands such as object spawning need.

use crate::canon::{Canon, ToCanon};
use crate::commands;
use crate::dev;
use crate::hash::{combine_table_hashes, StateHash};
use crate::input::{InputQueue, SettingChange, SimInput, StampedInput, SubmitError};
use crate::path::BatchExecutor;
use crate::pipeline::{Event, Pipeline, Services, TickReport};
use crate::time::{flags_for, ClockOverflow, SlotMinutes};
use crate::world::WorldState;
use pg_content::ContentSet;
use std::sync::Arc;

/// The hash recorded when a day boundary was crossed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayHash {
    /// Index of the day that just began.
    pub day: u64,
    /// The tick the boundary was crossed on.
    pub tick: u64,
    /// The combined state hash.
    pub hash: StateHash,
    /// The per-table hashes it was combined from, so a divergence can be localized.
    pub tables: Vec<(&'static str, StateHash)>,
}

/// Everything needed to resume a run: the world plus inputs that are queued but not yet applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimSnapshot {
    pub world: WorldState,
    pub pending: Vec<StampedInput>,
    pub next_seq: u64,
}

pub struct Sim {
    world: WorldState,
    pipeline: Pipeline,
    queue: InputQueue,
    applied: Vec<StampedInput>,
    day_hashes: Vec<DayHash>,
    trace: bool,
    services: Services,
    content: Option<Arc<ContentSet>>,
}

impl Sim {
    pub fn new(world: WorldState, pipeline: Pipeline) -> Sim {
        Sim {
            world,
            pipeline,
            queue: InputQueue::new(),
            applied: Vec::new(),
            day_hashes: Vec::new(),
            trace: false,
            services: Services::new(),
            content: None,
        }
    }

    /// A sim whose pipeline has the dev scaffolding systems installed.
    pub fn with_dev_systems(world: WorldState) -> Sim {
        let mut pipeline = Pipeline::new();
        dev::install(&mut pipeline);
        Sim::new(world, pipeline)
    }

    /// Gives the sim loaded content (needed by commands that create objects).
    pub fn with_content(mut self, content: Arc<ContentSet>) -> Sim {
        self.content = Some(content);
        self
    }

    pub fn content(&self) -> Option<&Arc<ContentSet>> {
        self.content.as_ref()
    }

    /// Chooses how path batches are solved. Results never depend on this choice, only speed does.
    pub fn set_executor(&mut self, exec: Box<dyn BatchExecutor>) {
        self.services.exec = exec;
    }

    /// `(hits, misses)` of the path cache.
    pub fn path_cache_stats(&self) -> (u64, u64) {
        self.services.paths.stats()
    }

    /// Records the ids of the systems that ran in each `TickReport`.
    pub fn set_trace(&mut self, on: bool) {
        self.trace = on;
    }

    pub fn world(&self) -> &WorldState {
        &self.world
    }

    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// Inputs applied so far, in application order. This is the replay log's input list.
    pub fn applied_inputs(&self) -> &[StampedInput] {
        &self.applied
    }

    pub fn day_hashes(&self) -> &[DayHash] {
        &self.day_hashes
    }

    /// Queues `input` to apply when the clock is at `tick`.
    pub fn submit(&mut self, tick: u64, input: SimInput) -> Result<(), SubmitError> {
        self.queue
            .submit(self.world.clock.tick(), tick, input)
            .map(|_| ())
    }

    /// Queues `input` for the very next step.
    pub fn submit_now(&mut self, input: SimInput) -> Result<(), SubmitError> {
        self.submit(self.world.clock.tick(), input)
    }

    pub fn snapshot(&self) -> SimSnapshot {
        let (pending, next_seq) = self.queue.snapshot();
        SimSnapshot {
            world: self.world.clone(),
            pending,
            next_seq,
        }
    }

    /// Resumes from a snapshot with the given pipeline (which must be configured like the original).
    /// Derived state (occupancy, caches) is rebuilt.
    pub fn restore(snapshot: SimSnapshot, pipeline: Pipeline) -> Sim {
        let mut world = snapshot.world;
        world.rebuild_derived();
        let mut sim = Sim::new(world, pipeline);
        sim.queue = InputQueue::restore(snapshot.next_seq, snapshot.pending);
        sim
    }

    /// Advances the world by one tick.
    pub fn step(&mut self) -> Result<TickReport, ClockOverflow> {
        // 1. Apply the inputs due on the current tick.
        let now = self.world.clock.tick();
        let due = self.queue.drain_for(now);
        let mut events = Vec::new();
        let content = self.content.clone();
        for stamped in &due {
            apply_input(&mut self.world, content.as_deref(), stamped, &mut events);
        }
        self.applied.extend(due.iter().cloned());

        // 2. Advance the clock; flags describe the tick just entered.
        self.world.clock.advance()?;
        let flags = flags_for(self.world.clock.tick(), self.world.settings.slot_minutes);

        // 3. Systems.
        let mut report = TickReport::new(flags);
        report.inputs_applied = u32::try_from(due.len()).unwrap_or(u32::MAX);
        report.events = events;
        self.pipeline.run(
            &mut self.world,
            &flags,
            &mut report,
            &mut self.services,
            self.trace,
        );

        // Debug builds check every event against the catalog (S-022): a drifting emitter fails fast.
        #[cfg(debug_assertions)]
        {
            let problems = crate::events::EventCatalog::shared().validate_all(&report.events);
            debug_assert!(problems.is_empty(), "invalid events: {problems:?}");
        }

        // 4. Day boundary: record the state hash.
        if flags.new_day {
            let tables = self.world.table_hashes();
            let hash = combine_table_hashes(tables.iter().map(|(name, h)| (*name, *h)));
            report.day_hash = Some(hash);
            self.day_hashes.push(DayHash {
                day: flags.day,
                tick: flags.tick,
                hash,
                tables,
            });
        }
        Ok(report)
    }

    /// Steps `ticks` times.
    pub fn run_ticks(&mut self, ticks: u64) -> Result<(), ClockOverflow> {
        for _ in 0..ticks {
            self.step()?;
        }
        Ok(())
    }
}

fn apply_input(
    world: &mut WorldState,
    content: Option<&ContentSet>,
    stamped: &StampedInput,
    events: &mut Vec<Event>,
) {
    let tick = stamped.tick;
    match &stamped.input {
        SimInput::SettingChange(SettingChange::SlotMinutes(m)) => match SlotMinutes::new(*m) {
            Ok(slot) => {
                let changed = world.settings.slot_minutes != slot;
                world.settings.slot_minutes = slot;
                if changed {
                    // Commitments are agreed in slots, so a different slot length voids them; schedules
                    // are replanned at the next slot boundary because their length no longer matches.
                    for id in crate::commitment::cancel_live(world, "the slot length changed") {
                        events.push(Event {
                            tick,
                            kind: "commitment.cancelled".into(),
                            detail: Canon::map([("commitment", id.to_canon())]),
                        });
                    }
                }
                events.push(Event {
                    tick,
                    kind: "setting_changed".into(),
                    detail: Canon::map([
                        ("key", Canon::str("slot_minutes")),
                        ("value", m.to_canon()),
                    ]),
                });
            }
            Err(e) => events.push(rejected(tick, &e.to_string())),
        },
        SimInput::Command { cmd, .. } => commands::apply(world, content, tick, cmd, events),
    }
}

fn rejected(tick: u64, reason: &str) -> Event {
    Event {
        tick,
        kind: "input_rejected".into(),
        detail: Canon::map([("reason", Canon::str(reason))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{EntityId, Kind};
    use crate::input::Command;
    use crate::time::TICKS_PER_DAY;
    use proptest::prelude::*;

    fn sim(seed: &str) -> Sim {
        Sim::with_dev_systems(WorldState::new("Test Town", seed))
    }

    fn nudge(amount: i32) -> SimInput {
        SimInput::Command {
            actor: None,
            cmd: Command::DevNudge { amount },
        }
    }

    fn run_days(s: &mut Sim, days: u64) {
        s.run_ticks(days * TICKS_PER_DAY).unwrap();
    }

    #[test]
    fn same_seed_same_hashes_different_seed_different_hashes() {
        let (mut a, mut b, mut c) = (sim("alpha"), sim("alpha"), sim("beta"));
        for s in [&mut a, &mut b, &mut c] {
            run_days(s, 3);
        }
        assert_eq!(a.day_hashes(), b.day_hashes());
        assert_eq!(a.world().state_hash(), b.world().state_hash());
        assert_ne!(a.world().state_hash(), c.world().state_hash());
    }

    #[test]
    fn one_hash_per_day_boundary_at_the_right_ticks() {
        let mut s = sim("alpha");
        run_days(&mut s, 3);
        let days: Vec<_> = s.day_hashes().iter().map(|d| (d.day, d.tick)).collect();
        assert_eq!(days, vec![(1, 14_400), (2, 28_800), (3, 43_200)]);
        // Consecutive days differ because the probe walks.
        assert_ne!(s.day_hashes()[0].hash, s.day_hashes()[1].hash);
    }

    #[test]
    fn systems_run_on_their_cadence() {
        let mut s = sim("alpha");
        run_days(&mut s, 1);
        assert_eq!(
            s.world().probe.minutes,
            1440,
            "one probe step per game minute"
        );
        assert_eq!(s.world().probe.days, 1, "one day boundary crossed");
    }

    #[test]
    fn inputs_apply_exactly_on_their_stamped_tick() {
        let mut s = sim("alpha");
        s.submit(5, nudge(1_000_000)).unwrap();
        s.run_ticks(5).unwrap(); // ticks 0..=4 processed; the input (tick 5) has not applied yet
        assert_eq!(s.world().probe.value, 0);
        let report = s.step().unwrap(); // processes tick 5
        assert_eq!(report.inputs_applied, 1);
        assert_eq!(s.world().probe.value, 1_000_000);
        assert_eq!(
            report.events.first().map(|e| e.kind.as_str()),
            Some("dev.nudged")
        );
        assert_eq!(s.applied_inputs().len(), 1);
        assert_eq!(s.applied_inputs()[0].tick, 5);
    }

    #[test]
    fn past_inputs_are_refused() {
        let mut s = sim("alpha");
        s.run_ticks(10).unwrap();
        assert!(s.submit(9, nudge(1)).is_err());
        assert!(s.submit(10, nudge(1)).is_ok());
    }

    #[test]
    fn slot_setting_changes_apply_and_invalid_ones_are_rejected_visibly() {
        let mut s = sim("alpha");
        s.submit_now(SimInput::SettingChange(SettingChange::SlotMinutes(60)))
            .unwrap();
        let r = s.step().unwrap();
        assert_eq!(s.world().settings.slot_minutes.get(), 60);
        assert_eq!(r.events[0].kind, "setting_changed");

        let before = s.world().settings.clone();
        s.submit_now(SimInput::SettingChange(SettingChange::SlotMinutes(7)))
            .unwrap();
        let r = s.step().unwrap();
        assert_eq!(
            s.world().settings,
            before,
            "an invalid value must not change anything"
        );
        assert_eq!(r.events[0].kind, "input_rejected");
    }

    #[test]
    fn a_slot_setting_change_moves_the_slot_boundaries() {
        let mut s = sim("alpha");
        s.set_trace(true);
        s.submit_now(SimInput::SettingChange(SettingChange::SlotMinutes(60)))
            .unwrap();
        s.run_ticks(1).unwrap();
        // Tick 300 is a 30-minute boundary but not a 60-minute one.
        let mut slot_ticks = Vec::new();
        for _ in 0..700 {
            let r = s.step().unwrap();
            if r.flags.slot {
                slot_ticks.push(r.tick);
            }
        }
        assert_eq!(slot_ticks, vec![600]);
    }

    #[test]
    fn trace_lists_the_systems_that_ran() {
        let mut s = sim("alpha");
        s.set_trace(true);
        let r = s.step().unwrap();
        assert!(r.ran.contains(&"Maintenance".to_owned()));
        assert!(
            !r.ran.contains(&"dev.probe".to_owned()),
            "not a minute boundary"
        );
        s.set_trace(false);
        assert!(s.step().unwrap().ran.is_empty());
    }

    #[test]
    fn the_day_hash_matches_the_world_hash_at_that_moment() {
        let mut s = sim("alpha");
        s.run_ticks(TICKS_PER_DAY - 1).unwrap();
        let report = s.step().unwrap();
        assert_eq!(report.day_hash, Some(s.world().state_hash()));
    }

    #[test]
    fn snapshot_and_restore_continue_identically_including_pending_inputs() {
        let mut full = sim("gamma");
        full.submit(100, nudge(7)).unwrap();
        full.submit(20_000, nudge(-9)).unwrap();
        full.run_ticks(TICKS_PER_DAY * 2).unwrap();

        let mut first = sim("gamma");
        first.submit(100, nudge(7)).unwrap();
        first.submit(20_000, nudge(-9)).unwrap();
        first.run_ticks(5_000).unwrap(); // snapshot is taken with the tick-20000 input still pending
        let snap = first.snapshot();
        assert_eq!(snap.pending.len(), 1);
        let mut resumed = {
            let mut p = Pipeline::new();
            dev::install(&mut p);
            Sim::restore(snap, p)
        };
        resumed.run_ticks(TICKS_PER_DAY * 2 - 5_000).unwrap();
        assert_eq!(resumed.world().state_hash(), full.world().state_hash());
        assert_eq!(resumed.world(), full.world());
    }

    #[test]
    fn input_order_on_one_tick_does_not_depend_on_submission_order() {
        let a_id = Some(EntityId::new(Kind::Pawn, 1));
        let b_id = Some(EntityId::new(Kind::Pawn, 2));
        let mk = |actor, amount| SimInput::Command {
            actor,
            cmd: Command::DevNudge { amount },
        };
        let mut x = sim("delta");
        x.submit(3, mk(a_id, 5)).unwrap();
        x.submit(3, mk(b_id, 9)).unwrap();
        let mut y = sim("delta");
        y.submit(3, mk(b_id, 9)).unwrap();
        y.submit(3, mk(a_id, 5)).unwrap();
        x.run_ticks(10).unwrap();
        y.run_ticks(10).unwrap();
        // Applied order is by actor, so both runs applied (a, b) and logged the same actors in order.
        let actors = |s: &Sim| {
            s.applied_inputs()
                .iter()
                .map(|i| i.input.actor())
                .collect::<Vec<_>>()
        };
        assert_eq!(actors(&x), vec![a_id, b_id]);
        assert_eq!(actors(&y), vec![a_id, b_id]);
        assert_eq!(x.world().state_hash(), y.world().state_hash());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        #[test]
        fn splitting_a_run_anywhere_gives_the_same_result(
            seed in "[a-z]{1,6}",
            split in 1u64..19_000,
            inputs in proptest::collection::vec((0u64..30_000, -1000i32..1000), 0..8),
        ) {
            let total = TICKS_PER_DAY + 5_000;
            let build = |inputs: &[(u64, i32)]| {
                let mut s = sim(&seed);
                for &(t, a) in inputs { s.submit(t, nudge(a)).unwrap(); }
                s
            };
            let mut whole = build(&inputs);
            whole.run_ticks(total).unwrap();

            let mut part = build(&inputs);
            part.run_ticks(split).unwrap();
            let snap = part.snapshot();
            let mut p = Pipeline::new();
            dev::install(&mut p);
            let mut resumed = Sim::restore(snap, p);
            resumed.run_ticks(total - split).unwrap();
            prop_assert_eq!(resumed.world().state_hash(), whole.world().state_hash());
        }
    }
}
