//! The simulation loop (Blueprint §6.1, §6.4): everything the simulation thread does between frames,
//! written as a plain struct driven by [`SimLoop::frame`], so it can be tested with a fake clock and no
//! threads. [`crate::session::Session`] runs the same struct on a dedicated thread.
//!
//! One frame: apply queued work, turn the real time since the last frame into a whole number of ticks
//! (fixed-step accumulator), run each tick under the panic guard, take keyframes, hand a keyframe pair to
//! shadow verification, poll finished saves, start an autosave if one is due, and publish a
//! [`RenderSnapshot`]. Saves are encoded and written on the worker pool from a clone taken at a tick
//! boundary, so a slow disk never stalls the simulation.

use crate::control::{Accumulator, AutosaveTimer, Effect, RunEvent, RunState};
use crate::guard::{guarded, PanicReport};
use crate::keyframes::{rewind, verify_span, Keyframe, KeyframeRing, ShadowOutcome, SimFactory};
use crate::pool::{JobHandle, WorkerPool};
use crate::snapshot::{RenderSnapshot, SnapshotPublisher};
use pg_core::canon::ToCanon;
use pg_core::commands::Command;
use pg_core::input::{SimInput, StampedInput};
use pg_core::replay::{ContentRefRecord, ReplayLog, StartState, PROFILE_DEV};
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use pg_host::{Clock, Level, LogSink, Storage};
use pg_persist::crash::{write_report, CrashInfo};
use pg_persist::logfile::Bundle;
use pg_persist::store::{SaveReport, SlotStore};
use std::sync::Arc;
use std::time::Duration;

pub struct LoopServices {
    pub clock: Arc<dyn Clock>,
    pub storage: Arc<dyn Storage>,
    pub log: Arc<dyn LogSink>,
    pub pool: Arc<WorkerPool>,
    /// Makes the small picture saved with each generation (suggestion S-024); `None` saves without one.
    pub thumbnailer: Option<Thumbnailer>,
    /// Where the simulation's events and the loop's own messages go for the developer console.
    pub console: Option<pg_host::Console>,
    /// Prepares the lines of conversations someone can hear (milestone 1.5).
    pub dialogue: Option<crate::dialogue::DialogueHook>,
}

/// Renders a world to PNG bytes for the Saved Worlds list.
pub type Thumbnailer = Arc<dyn Fn(&pg_core::world::WorldState) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct LoopConfig {
    pub world_id: String,
    pub app_version: String,
    pub content_refs: Vec<ContentRefRecord>,
    pub max_ticks_per_frame: u64,
    pub keyframe_interval_ticks: u64,
    pub keyframe_capacity: usize,
    pub autosave_minutes: u64,
    /// With `Some(n)`, every new keyframe pair is re-simulated with `n` executor threads and compared
    /// (suggestion S-030): for dev and soak runs.
    pub shadow_threads: Option<usize>,
}

impl LoopConfig {
    pub fn new(world_id: &str) -> LoopConfig {
        LoopConfig {
            world_id: world_id.to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            content_refs: Vec::new(),
            max_ticks_per_frame: 50,
            keyframe_interval_ticks: 1_800, // one simulated hour
            keyframe_capacity: 24,
            autosave_minutes: 5,
            shadow_threads: None,
        }
    }
}

/// Requests to the loop.
#[derive(Clone, Debug)]
pub enum Control {
    Run(RunEvent),
    Command(Command),
    SaveNow,
    /// Time scrub: rebuild the world as it was at this tick (the future is discarded).
    RewindTo(u64),
    SetAutosaveMinutes(u64),
    /// Writes a bug bundle of the running world now (suggestion S-001).
    CutBundle,
}

/// What happened, for the UI and the dev overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopEvent {
    State(RunState),
    Saved {
        generation: u64,
        tick: u64,
        autosave: bool,
    },
    SaveFailed(String),
    /// Shadow verification re-simulated a span and it matched.
    Verified {
        ticks: u64,
    },
    /// Shadow verification found a difference; the world was paused and a bundle written.
    Diverged {
        at_tick: u64,
        tables: Vec<String>,
        bundle: Option<String>,
    },
    /// A tick panicked; the world is frozen. `report` and `bundle` are storage names.
    Crashed {
        message: String,
        report: Option<String>,
        bundle: Option<String>,
    },
    Rewound {
        to: u64,
    },
    /// A bundle was written (name in storage), or could not be.
    BundleWritten(Option<String>),
    Refused(String),
}

type SaveJob = JobHandle<Result<SaveReport, String>>;

pub struct SimLoop {
    factory: SimFactory,
    sim: Sim,
    ring: KeyframeRing,
    log: Vec<StampedInput>,
    logged: usize,
    state: RunState,
    acc: Accumulator,
    autosave: AutosaveTimer,
    last_frame: Option<Duration>,
    services: LoopServices,
    cfg: LoopConfig,
    publisher: Arc<SnapshotPublisher<RenderSnapshot>>,
    saves: Vec<(SaveJob, u64, bool)>,
    shadows: Vec<JobHandle<ShadowOutcome>>,
    /// The previous keyframe pair a shadow job was started for, kept to build a bundle on divergence.
    shadow_span: Option<(Keyframe, Keyframe, Vec<StampedInput>)>,
    day_hash: String,
    dirty: bool,
}

impl SimLoop {
    /// A loop over `world`, starting **paused** (the player resumes).
    pub fn new(
        factory: SimFactory,
        world: WorldState,
        services: LoopServices,
        cfg: LoopConfig,
    ) -> SimLoop {
        let sim = factory.restore(pg_core::sim::SimSnapshot {
            world,
            pending: Vec::new(),
            next_seq: 0,
        });
        let state = RunState::Paused;
        let first = RenderSnapshot::build(sim.world(), state, None, &[], "");
        let mut ring = KeyframeRing::new(cfg.keyframe_interval_ticks, cfg.keyframe_capacity);
        ring.push(Keyframe::capture(&sim, 0));
        SimLoop {
            factory,
            sim,
            ring,
            log: Vec::new(),
            logged: 0,
            state,
            acc: Accumulator::new(cfg.max_ticks_per_frame),
            autosave: AutosaveTimer::new(cfg.autosave_minutes),
            last_frame: None,
            services,
            cfg,
            publisher: Arc::new(SnapshotPublisher::new(first)),
            saves: Vec::new(),
            shadows: Vec::new(),
            shadow_span: None,
            day_hash: String::new(),
            dirty: false,
        }
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    pub fn publisher(&self) -> Arc<SnapshotPublisher<RenderSnapshot>> {
        Arc::clone(&self.publisher)
    }

    pub fn keyframe_ticks(&self) -> Vec<u64> {
        self.ring.ticks()
    }

    pub fn input_log(&self) -> &[StampedInput] {
        &self.log
    }

    fn note(&self, level: Level, text: &str) {
        self.services.log.log(level, text);
    }

    // ---- control -------------------------------------------------------------------------------------

    pub fn control(&mut self, c: Control) -> Vec<LoopEvent> {
        let mut out = Vec::new();
        match c {
            Control::Run(e) => self.run_event(e, &mut out),
            Control::Command(cmd) => {
                if self.state == RunState::Crashed || self.state == RunState::Stopped {
                    out.push(LoopEvent::Refused("the world is not running".to_owned()));
                } else if let Err(e) = self.sim.submit_now(SimInput::Command { actor: None, cmd }) {
                    out.push(LoopEvent::Refused(e.to_string()));
                }
            }
            Control::SaveNow => self.start_save(false),
            Control::RewindTo(t) => self.rewind_to(t, &mut out),
            Control::SetAutosaveMinutes(m) => self.autosave.set_interval_minutes(m),
            Control::CutBundle => {
                let name = self.ring.latest().cloned().and_then(|kf| {
                    let log = self.log.clone();
                    let until = self.sim.world().clock.tick();
                    self.write_bundle(&kf, &log, until, "bundle cut by the developer overlay")
                });
                out.push(LoopEvent::BundleWritten(name));
            }
        }
        out
    }

    fn run_event(&mut self, e: RunEvent, out: &mut Vec<LoopEvent>) {
        let (next, effect) = self.state.on(e);
        if next != self.state {
            self.state = next;
            self.dirty = true;
            out.push(LoopEvent::State(next));
        }
        if effect == Effect::SaveNow {
            self.start_save(false);
        }
    }

    // ---- saving --------------------------------------------------------------------------------------

    fn start_save(&mut self, autosave: bool) {
        let world = self.sim.world().clone();
        let tick = world.clock.tick();
        let storage = Arc::clone(&self.services.storage);
        let thumbnailer = self.services.thumbnailer.clone();
        let (id, refs, iso) = (
            self.cfg.world_id.clone(),
            self.cfg.content_refs.clone(),
            self.services.clock.wall_clock_iso(),
        );
        let job = self.services.pool.submit(move || {
            let thumb = thumbnailer.as_ref().and_then(|f| f(&world));
            SlotStore::new(storage.as_ref())
                .save_with(&id, &world, &refs, &iso, thumb.as_deref())
                .map_err(|e| e.to_string())
        });
        self.saves.push((job, tick, autosave));
        self.autosave.saved();
    }

    fn poll_saves(&mut self, out: &mut Vec<LoopEvent>, wait: bool) {
        let mut still = Vec::new();
        for (job, tick, autosave) in std::mem::take(&mut self.saves) {
            let result = if wait {
                job.join()
            } else {
                match job.try_join() {
                    Some(r) => r,
                    None => {
                        still.push((job, tick, autosave));
                        continue;
                    }
                }
            };
            match result {
                Ok(Ok(r)) => out.push(LoopEvent::Saved {
                    generation: r.generation,
                    tick,
                    autosave,
                }),
                Ok(Err(e)) => {
                    self.note(Level::Warn, &format!("save failed: {e}"));
                    out.push(LoopEvent::SaveFailed(e));
                }
                Err(p) => {
                    self.note(Level::Error, &format!("{p}"));
                    out.push(LoopEvent::SaveFailed(p.to_string()));
                }
            }
        }
        self.saves = still;
    }

    /// Waits for every save in flight (used when closing and by tests).
    pub fn finish_saves(&mut self) -> Vec<LoopEvent> {
        let mut out = Vec::new();
        self.poll_saves(&mut out, true);
        out
    }

    // ---- ticking -------------------------------------------------------------------------------------

    /// Sends this tick's events to the developer console at their catalog severities.
    fn to_console(&self, events: &[pg_core::pipeline::Event]) {
        if let Some(console) = &self.services.console {
            crate::console::push_events(console, events);
        }
    }

    fn log_applied(&mut self) {
        let applied = self.sim.applied_inputs();
        self.log
            .extend_from_slice(applied.get(self.logged..).unwrap_or(&[]));
        self.logged = applied.len();
    }

    /// Runs one tick under the guard. False means stop ticking this frame.
    fn tick(
        &mut self,
        events: &mut Vec<pg_core::pipeline::Event>,
        out: &mut Vec<LoopEvent>,
    ) -> bool {
        let stepped = guarded(|| self.sim.step());
        match stepped {
            Ok(Ok(report)) => {
                if let Some(h) = report.day_hash {
                    self.day_hash = h.short();
                }
                self.to_console(&report.events);
                if let Some(d) = &self.services.dialogue {
                    d.after_tick(&report.events, &mut self.sim, report.tick);
                }
                events.extend(report.events);
                self.log_applied();
                if self.ring.due(self.sim.world().clock.tick()) {
                    self.take_keyframe(out);
                }
                true
            }
            Ok(Err(_)) => {
                out.push(LoopEvent::Refused("the clock overflowed".to_owned()));
                self.run_event(RunEvent::Pause, out);
                false
            }
            Err(p) => {
                self.crash(p, out);
                false
            }
        }
    }

    fn take_keyframe(&mut self, out: &mut Vec<LoopEvent>) {
        let prev = self.ring.latest().cloned();
        let kf = Keyframe::capture(&self.sim, self.log.len());
        self.ring.push(kf.clone());
        if let (Some(threads), Some(prev)) = (self.cfg.shadow_threads, prev) {
            let factory = self.factory.with_threads(threads);
            let log: Vec<StampedInput> = self.log.clone();
            self.shadow_span = Some((prev.clone(), kf.clone(), log.clone()));
            self.shadows.push(
                self.services
                    .pool
                    .submit(move || verify_span(&factory, &prev, &log, &kf)),
            );
        }
        let _ = out;
    }

    fn handle_shadow(
        &mut self,
        r: Result<ShadowOutcome, crate::pool::JobPanic>,
        out: &mut Vec<LoopEvent>,
    ) {
        match r {
            Ok(ShadowOutcome::Match { ticks }) => out.push(LoopEvent::Verified { ticks }),
            Ok(ShadowOutcome::Mismatch {
                at_tick, tables, ..
            }) => {
                let bundle = self.shadow_span.clone().and_then(|(from, to, log)| {
                    self.write_bundle(
                        &from,
                        &log,
                        to.tick,
                        &format!(
                            "shadow verification diverged at tick {at_tick} in {}",
                            tables.join(", ")
                        ),
                    )
                });
                self.note(
                    Level::Error,
                    &format!("shadow verification diverged at tick {at_tick}"),
                );
                out.push(LoopEvent::Diverged {
                    at_tick,
                    tables,
                    bundle,
                });
                self.run_event(RunEvent::Pause, out);
            }
            Ok(ShadowOutcome::Failed(why)) => out.push(LoopEvent::Refused(format!(
                "shadow verification could not run: {why}"
            ))),
            Err(p) => out.push(LoopEvent::Refused(format!(
                "shadow verification failed: {p}"
            ))),
        }
    }

    fn poll_shadows(&mut self, out: &mut Vec<LoopEvent>) {
        let mut still = Vec::new();
        for job in std::mem::take(&mut self.shadows) {
            match job.try_join() {
                None => still.push(job),
                Some(r) => self.handle_shadow(r, out),
            }
        }
        self.shadows = still;
    }

    /// Waits for shadow jobs (tests and shutdown).
    pub fn finish_shadows(&mut self) -> Vec<LoopEvent> {
        let mut out = Vec::new();
        for job in std::mem::take(&mut self.shadows) {
            let r = job.join();
            self.handle_shadow(r, &mut out);
        }
        out
    }

    // ---- crash and bundles ---------------------------------------------------------------------------

    /// A bundle that replays from `from` for the inputs logged since it, up to `until` ticks.
    fn write_bundle(
        &self,
        from: &Keyframe,
        log: &[StampedInput],
        until: u64,
        note: &str,
    ) -> Option<String> {
        let w = &from.snapshot.world;
        let inputs: Vec<StampedInput> = log
            .get(from.log_len..)
            .unwrap_or(&[])
            .iter()
            .filter(|s| s.seq >= from.snapshot.next_seq)
            .cloned()
            .collect();
        let replay = ReplayLog {
            profile: PROFILE_DEV.to_owned(),
            world_name: w.meta.name.clone(),
            seed_text: w.meta.seed_text.clone(),
            content: self.cfg.content_refs.clone(),
            ticks: until.saturating_sub(from.tick),
            inputs,
            day_hashes: Vec::new(),
            final_hash: String::new(),
            final_tables: Vec::new(),
            start: Some(StartState {
                tick: from.tick,
                world: w.to_canon(),
                pending: from.snapshot.pending.clone(),
                next_seq: from.snapshot.next_seq,
            }),
        };
        let bundle = Bundle {
            note: note.to_owned(),
            created_iso: self.services.clock.wall_clock_iso(),
            app_version: self.cfg.app_version.clone(),
            log: replay,
        };
        let name = format!(
            "crash/{}.pgbundle",
            self.services
                .clock
                .wall_clock_iso()
                .replace([':', '.'], "-")
        );
        self.services
            .storage
            .write_atomic(&name, &bundle.encode())
            .ok()?;
        Some(name)
    }

    fn crash(&mut self, p: PanicReport, out: &mut Vec<LoopEvent>) {
        self.note(Level::Error, &format!("a tick panicked: {}", p.message));
        if let Some(c) = &self.services.console {
            c.fatal(
                "sim",
                &format!("the world was frozen: a tick panicked: {}", p.message),
            );
        }
        let tick = self.sim.world().clock.tick();
        let recent: Vec<String> = self
            .publisher
            .latest()
            .recent_events
            .iter()
            .rev()
            .take(20)
            .rev()
            .map(|e| format!("@{} {}", e.tick, e.kind))
            .collect();
        let tick_text = tick.to_string();
        let iso = self.services.clock.wall_clock_iso();
        let report = write_report(
            self.services.storage.as_ref(),
            &CrashInfo {
                message: &p.message,
                location: p.location.as_deref(),
                backtrace: Some(&p.backtrace),
                app_version: &self.cfg.app_version,
                os: std::env::consts::OS,
                time_iso: &iso,
                recent_log: &recent,
                context: &[("world", &self.cfg.world_id), ("tick", &tick_text)],
            },
            &[],
        )
        .ok();
        let bundle = self.ring.latest().cloned().and_then(|kf| {
            let log = self.log.clone();
            self.write_bundle(
                &kf,
                &log,
                tick + 1,
                &format!("automatic bundle: a tick panicked: {}", p.message),
            )
        });
        self.state = RunState::Crashed;
        self.dirty = true;
        out.push(LoopEvent::State(RunState::Crashed));
        out.push(LoopEvent::Crashed {
            message: p.message,
            report,
            bundle,
        });
    }

    // ---- time scrub ----------------------------------------------------------------------------------

    fn rewind_to(&mut self, target: u64, out: &mut Vec<LoopEvent>) {
        if matches!(
            self.state,
            RunState::Crashed | RunState::Stopped | RunState::Closing
        ) {
            out.push(LoopEvent::Refused(
                "the world cannot be rewound now".to_owned(),
            ));
            return;
        }
        if target > self.sim.world().clock.tick() {
            out.push(LoopEvent::Refused(
                "cannot scrub forward past the present".to_owned(),
            ));
            return;
        }
        match rewind(&self.factory, &self.ring, &self.log, target) {
            Ok(sim) => {
                self.sim = sim;
                self.ring.truncate_after(target);
                let keep = self.log.partition_point(|s| s.tick < target);
                self.log.truncate(keep);
                self.logged = self.sim.applied_inputs().len();
                self.shadows.clear();
                self.shadow_span = None;
                self.dirty = true;
                out.push(LoopEvent::Rewound { to: target });
                // Scrubbing pauses: the player decides when to run on from the past.
                self.run_event(RunEvent::Pause, out);
            }
            Err(e) => out.push(LoopEvent::Refused(e.to_string())),
        }
    }

    // ---- the frame -----------------------------------------------------------------------------------

    /// One iteration of the loop at the clock's current time.
    pub fn frame(&mut self) -> Vec<LoopEvent> {
        let mut out = Vec::new();
        let now = self.services.clock.now_monotonic();
        let elapsed = self
            .last_frame
            .map_or(Duration::ZERO, |l| now.saturating_sub(l));
        self.last_frame = Some(now);

        self.poll_saves(&mut out, false);
        self.poll_shadows(&mut out);

        let mut events = Vec::new();
        if let RunState::Running(speed) = self.state {
            let ticks = self.acc.advance(elapsed, speed);
            for _ in 0..ticks {
                if !self.tick(&mut events, &mut out) {
                    break;
                }
            }
            self.dirty |= ticks > 0;
        } else {
            self.acc.reset();
        }

        if self.autosave.poll(now, self.state.is_running()) {
            self.start_save(true);
        }
        if self.state == RunState::Closing {
            self.poll_saves(&mut out, true);
            self.run_event(RunEvent::Finished, &mut out);
        }

        if self.dirty || !events.is_empty() {
            let prev = self.publisher.latest();
            let mut snap = RenderSnapshot::build(
                self.sim.world(),
                self.state,
                Some(&prev),
                &events,
                &self.day_hash,
            );
            snap.keyframes = self.ring.ticks();
            snap.keyframe_hash = self
                .ring
                .latest()
                .map_or_else(String::new, |k| k.hash.short());
            self.publisher.publish(snap);
            self.dirty = false;
        }
        out
    }
}

#[cfg(test)]
mod tests;
