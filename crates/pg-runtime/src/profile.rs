//! A per-system tick profiler (Blueprint §19, milestone 0.8 profiling pass).
//!
//! The core never reads a clock, so timing lives here: [`Profiler`] implements the pipeline's
//! [`SystemProbe`] with `Instant`, and a [`crate::keyframes::SimFactory`] built with a profiler installs it
//! on every pipeline it creates. The overlay shows the same numbers per frame; `pg profile` prints them for
//! a headless run. Profiling adds a lock per system call, so it is a dev mode, not the default.

use pg_core::pipeline::SystemProbe;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    pub calls: u64,
    pub total: Duration,
    pub max: Duration,
}

#[derive(Default)]
struct Data {
    systems: BTreeMap<String, Stat>,
    started: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct Profiler {
    data: Arc<Mutex<Data>>,
}

struct Probe {
    data: Arc<Mutex<Data>>,
}

impl SystemProbe for Probe {
    fn begin(&mut self, _system: &str) {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).started = Some(Instant::now());
    }

    fn end(&mut self, system: &str) {
        let mut d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = d.started.take() {
            let took = t.elapsed();
            let s = d.systems.entry(system.to_owned()).or_default();
            s.calls += 1;
            s.total += took;
            s.max = s.max.max(took);
        }
    }
}

/// Rows of a report, slowest total first.
#[derive(Clone, Debug, Default)]
pub struct ProfileReport {
    pub rows: Vec<(String, Stat)>,
    pub total: Duration,
}

impl Profiler {
    pub fn new() -> Profiler {
        Profiler::default()
    }

    /// A probe feeding this profiler (install with `Pipeline::set_probe`).
    pub fn probe(&self) -> Box<dyn SystemProbe> {
        Box::new(Probe {
            data: Arc::clone(&self.data),
        })
    }

    pub fn report(&self) -> ProfileReport {
        let d = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows: Vec<(String, Stat)> =
            d.systems.iter().map(|(k, v)| (k.clone(), *v)).collect();
        rows.sort_by(|a, b| b.1.total.cmp(&a.1.total).then_with(|| a.0.cmp(&b.0)));
        let total = rows.iter().map(|r| r.1.total).sum();
        ProfileReport { rows, total }
    }

    pub fn reset(&self) {
        *self.data.lock().unwrap_or_else(|e| e.into_inner()) = Data::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyframes::SimFactory;
    use pg_core::commands::Command;
    use pg_core::id::{EntityId, Kind};
    use pg_core::input::SimInput;
    use pg_core::world::WorldState;

    #[test]
    fn a_profiled_run_times_every_system_that_ran() {
        let prof = Profiler::new();
        let factory = SimFactory::dev(None, 1).with_profiler(Some(prof.clone()));
        let mut sim = factory.new_sim(WorldState::new("Prof", "p"));
        let cmd = |c| SimInput::Command {
            actor: None,
            cmd: c,
        };
        sim.submit(
            0,
            cmd(Command::DevCreateMap {
                w: 30,
                h: 30,
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
        sim.run_ticks(2_000).unwrap();
        let r = prof.report();
        let names: Vec<&str> = r.rows.iter().map(|(n, _)| n.as_str()).collect();
        for want in [
            "MovementSystem",
            "ActivitySystem",
            "TaskPlanner",
            "ReservationActivator",
        ] {
            assert!(names.contains(&want), "{want} not in {names:?}");
        }
        let movement = r
            .rows
            .iter()
            .find(|(n, _)| n == "MovementSystem")
            .unwrap()
            .1;
        assert_eq!(
            movement.calls, 2_000,
            "a tick-cadence system runs every tick"
        );
        assert!(movement.total >= movement.max && r.total >= movement.total);
        let slot = r
            .rows
            .iter()
            .find(|(n, _)| n == "ReservationActivator")
            .unwrap()
            .1;
        assert_eq!(
            slot.calls,
            2_000 / 300,
            "a slot-cadence system runs once per 30-minute slot"
        );
        // Sorted slowest first.
        assert!(r.rows.windows(2).all(|w| w[0].1.total >= w[1].1.total));
        prof.reset();
        assert!(prof.report().rows.is_empty());
    }

    #[test]
    fn profiling_does_not_change_the_simulation() {
        let run = |profiled: bool| {
            let f = SimFactory::dev(None, 1);
            let f = if profiled {
                f.with_profiler(Some(Profiler::new()))
            } else {
                f
            };
            let mut sim = f.new_sim(WorldState::new("Prof", "p"));
            let cmd = |c| SimInput::Command {
                actor: None,
                cmd: c,
            };
            sim.submit(
                0,
                cmd(Command::DevCreateMap {
                    w: 20,
                    h: 20,
                    style: 1,
                }),
            )
            .unwrap();
            for i in 0..5 {
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
            sim.run_ticks(3_000).unwrap();
            sim.world().state_hash()
        };
        assert_eq!(run(false), run(true));
    }
}
