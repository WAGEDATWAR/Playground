//! **Dev scaffolding systems** for milestone 0.2.
//!
//! These give the tick pipeline something deterministic and RNG-dependent to do, so replay, hashing
//! and snapshot tests are meaningful before any real simulation system exists. They are removed (or
//! moved under `#[cfg(test)]`) when the Stage 1 systems land.

use crate::canon::{Canon, ToCanon};
use crate::pipeline::{Pipeline, System, SystemSlot, TickCtx};
use crate::rng::{Key, Rng};

/// Minute cadence (installed in the Needs slot): a seeded random walk on `probe.value`.
pub struct DevProbeSystem;

impl System for DevProbeSystem {
    fn id(&self) -> &str {
        "dev.probe"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let index = i64::try_from(ctx.world.probe.minutes).unwrap_or(i64::MAX);
        let rng = Rng::new(ctx.world.seed(), "dev.probe", &[Key::Int(index)]);
        let step = rng.int_in(0, 0, 999).unwrap_or(500) - 500;
        ctx.world.probe.value = ctx.world.probe.value.saturating_add(i64::from(step));
        ctx.world.probe.minutes = ctx.world.probe.minutes.saturating_add(1);
    }
}

/// Day cadence (installed in the DayPlanner slot): counts days and emits an event.
pub struct DevDaySystem;

impl System for DevDaySystem {
    fn id(&self) -> &str {
        "dev.day"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        ctx.world.probe.days = ctx.world.probe.days.saturating_add(1);
        let day = ctx.flags.day;
        ctx.emit("dev.day_started", Canon::map([("day", day.to_canon())]));
    }
}

/// Installs the dev systems into the Needs and DayPlanner slots.
pub fn install(pipeline: &mut Pipeline) {
    pipeline.set_builtin(SystemSlot::Needs, Box::new(DevProbeSystem));
    pipeline.set_builtin(SystemSlot::DayPlanner, Box::new(DevDaySystem));
}
