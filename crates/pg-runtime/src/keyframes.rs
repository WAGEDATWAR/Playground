//! Keyframe snapshots, time scrub and shadow verification (suggestions S-029 and S-030, Blueprint §6.4).
//!
//! * A [`KeyframeRing`] keeps a bounded number of full snapshots, one per `interval` ticks, each with its
//!   state hash and the length of the input log at that moment.
//! * [`rewind`] restores the newest keyframe at or before a target tick and replays the logged inputs
//!   forward to it: the dev overlay's time-scrub, and the start state of a crash bundle.
//! * [`verify_span`] re-simulates the span between two keyframes in a **fresh** sim (a different executor
//!   thread count, a freshly built pipeline) and compares the result with the recorded one: a mismatch
//!   means the simulation depended on something other than its inputs.

use pg_content::ContentSet;
use pg_core::hash::StateHash;
use pg_core::input::StampedInput;
use pg_core::pipeline::Pipeline;
use pg_core::sim::{Sim, SimSnapshot};
use pg_core::time::ClockOverflow;
use pg_core::world::WorldState;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::exec::ScopedThreads;

type PipelineFn = dyn Fn() -> Pipeline + Send + Sync;

/// Builds sims with a fixed configuration, so a restored or shadow sim is configured like the original.
#[derive(Clone)]
pub struct SimFactory {
    pipeline: Arc<PipelineFn>,
    content: Option<Arc<ContentSet>>,
    threads: usize,
}

impl SimFactory {
    /// The dev pipeline (the dev plan source and scaffolding systems).
    pub fn dev(content: Option<Arc<ContentSet>>, threads: usize) -> SimFactory {
        SimFactory::with_pipeline(
            Arc::new(|| {
                let mut p = Pipeline::new();
                pg_core::dev::install(&mut p);
                p
            }),
            content,
            threads,
        )
    }

    pub fn with_pipeline(
        pipeline: Arc<PipelineFn>,
        content: Option<Arc<ContentSet>>,
        threads: usize,
    ) -> SimFactory {
        SimFactory {
            pipeline,
            content,
            threads: threads.max(1),
        }
    }

    pub fn threads(&self) -> usize {
        self.threads
    }

    /// The same factory with a different executor thread count.
    pub fn with_threads(&self, threads: usize) -> SimFactory {
        SimFactory {
            threads: threads.max(1),
            ..self.clone()
        }
    }

    fn configure(&self, mut sim: Sim) -> Sim {
        if let Some(c) = &self.content {
            sim = sim.with_content(Arc::clone(c));
        }
        sim.set_executor(Box::new(ScopedThreads::new(self.threads)));
        sim
    }

    pub fn new_sim(&self, world: WorldState) -> Sim {
        self.configure(Sim::new(world, (self.pipeline)()))
    }

    pub fn restore(&self, snapshot: SimSnapshot) -> Sim {
        self.configure(Sim::restore(snapshot, (self.pipeline)()))
    }
}

/// A full snapshot at a tick, with what is needed to verify and replay from it.
#[derive(Clone)]
pub struct Keyframe {
    pub tick: u64,
    pub snapshot: SimSnapshot,
    pub hash: StateHash,
    pub tables: Vec<(&'static str, StateHash)>,
    /// How many inputs the session's input log held when this was taken.
    pub log_len: usize,
}

impl Keyframe {
    pub fn capture(sim: &Sim, log_len: usize) -> Keyframe {
        let w = sim.world();
        Keyframe {
            tick: w.clock.tick(),
            snapshot: sim.snapshot(),
            hash: w.state_hash(),
            tables: w.table_hashes(),
            log_len,
        }
    }
}

/// A bounded ring of keyframes, taken every `interval` ticks.
pub struct KeyframeRing {
    interval: u64,
    capacity: usize,
    frames: VecDeque<Keyframe>,
}

impl KeyframeRing {
    pub fn new(interval_ticks: u64, capacity: usize) -> KeyframeRing {
        KeyframeRing {
            interval: interval_ticks.max(1),
            capacity: capacity.max(1),
            frames: VecDeque::new(),
        }
    }

    /// Whether a keyframe is due at `tick`.
    pub fn due(&self, tick: u64) -> bool {
        self.frames
            .back()
            .is_none_or(|k| tick >= k.tick + self.interval)
    }

    pub fn push(&mut self, k: Keyframe) {
        while self.frames.len() >= self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(k);
    }

    pub fn latest(&self) -> Option<&Keyframe> {
        self.frames.back()
    }

    /// The newest keyframe at or before `tick`.
    pub fn at_or_before(&self, tick: u64) -> Option<&Keyframe> {
        self.frames.iter().rev().find(|k| k.tick <= tick)
    }

    /// The keyframe immediately before the newest one.
    pub fn previous(&self) -> Option<&Keyframe> {
        let n = self.frames.len();
        n.checked_sub(2).and_then(|i| self.frames.get(i))
    }

    pub fn ticks(&self) -> Vec<u64> {
        self.frames.iter().map(|k| k.tick).collect()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Forgets keyframes newer than `tick` (after a rewind the future is gone).
    pub fn truncate_after(&mut self, tick: u64) {
        while self.frames.back().is_some_and(|k| k.tick > tick) {
            self.frames.pop_back();
        }
    }
}

/// The inputs to resubmit when replaying from `from`: those logged since it that were not already queued
/// in its snapshot (their sequence numbers are at or past the snapshot's).
fn replay_inputs<'a>(
    log: &'a [StampedInput],
    from: &Keyframe,
    until_tick: u64,
) -> impl Iterator<Item = &'a StampedInput> {
    let next_seq = from.snapshot.next_seq;
    log.get(from.log_len..)
        .unwrap_or(&[])
        .iter()
        .filter(move |s| s.seq >= next_seq && s.tick < until_tick)
}

#[derive(Debug)]
pub enum RewindError {
    NoKeyframe,
    Submit(String),
    Clock,
}

impl std::fmt::Display for RewindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RewindError::NoKeyframe => write!(f, "no keyframe at or before that tick"),
            RewindError::Submit(e) => write!(f, "could not replay a logged input: {e}"),
            RewindError::Clock => write!(f, "the clock overflowed"),
        }
    }
}

impl std::error::Error for RewindError {}

impl From<ClockOverflow> for RewindError {
    fn from(_: ClockOverflow) -> Self {
        RewindError::Clock
    }
}

/// Rebuilds the sim as it was at `target`: the newest keyframe at or before it, plus the logged inputs
/// replayed forward. Keyframes after `target` are discarded by the caller.
pub fn rewind(
    factory: &SimFactory,
    ring: &KeyframeRing,
    log: &[StampedInput],
    target: u64,
) -> Result<Sim, RewindError> {
    let kf = ring.at_or_before(target).ok_or(RewindError::NoKeyframe)?;
    let mut sim = factory.restore(kf.snapshot.clone());
    for s in replay_inputs(log, kf, target) {
        sim.submit(s.tick, s.input.clone())
            .map_err(|e| RewindError::Submit(e.to_string()))?;
    }
    let steps = target - kf.tick;
    sim.run_ticks(steps)?;
    Ok(sim)
}

/// The result of re-simulating a span in a fresh sim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadowOutcome {
    Match {
        ticks: u64,
    },
    Mismatch {
        at_tick: u64,
        expected: StateHash,
        actual: StateHash,
        /// Tables whose hashes differ.
        tables: Vec<String>,
    },
    /// The span could not be replayed (a logged input was refused).
    Failed(String),
}

/// Re-simulates `from -> to` with `factory` and compares with `to`'s recorded hashes.
pub fn verify_span(
    factory: &SimFactory,
    from: &Keyframe,
    log: &[StampedInput],
    to: &Keyframe,
) -> ShadowOutcome {
    let mut sim = factory.restore(from.snapshot.clone());
    for s in replay_inputs(log, from, to.tick) {
        if let Err(e) = sim.submit(s.tick, s.input.clone()) {
            return ShadowOutcome::Failed(e.to_string());
        }
    }
    if let Err(e) = sim.run_ticks(to.tick.saturating_sub(from.tick)) {
        return ShadowOutcome::Failed(e.to_string());
    }
    let w = sim.world();
    let actual = w.state_hash();
    if actual == to.hash {
        return ShadowOutcome::Match {
            ticks: to.tick.saturating_sub(from.tick),
        };
    }
    let tables = w
        .table_hashes()
        .iter()
        .zip(&to.tables)
        .filter(|(a, b)| a.1 != b.1)
        .map(|(a, _)| a.0.to_owned())
        .collect();
    ShadowOutcome::Mismatch {
        at_tick: to.tick,
        expected: to.hash,
        actual,
        tables,
    }
}

#[cfg(test)]
mod tests;
