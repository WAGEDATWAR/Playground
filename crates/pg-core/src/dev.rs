//! **Dev scaffolding systems and generators** for milestones 0.2–0.4.
//!
//! These give the tick pipeline something deterministic and RNG-dependent to do, so replay, hashing,
//! snapshot and movement tests are meaningful before real simulation systems exist. The plan source
//! feeds the real scheduler (0.5); everything else here is throwaway. They are removed (or
//! moved under `#[cfg(test)]`) when the real systems land (D-009).

use crate::activity::{DayPlannerSystem, PlanSource, ReservationActivatorSystem};
use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::map::{Tile, ROAD, SAND, SIDEWALK, WATER};
use crate::num::Permille;
use crate::pipeline::{Cadence, Pipeline, Placement, System, SystemSlot, TickCtx};
use crate::rng::{Key, Rng, Stream};
use crate::schedule::{DutyTemplate, LeisureOption, PlanInputs};
use crate::world::WorldState;
use pg_content::ActionId;
use std::sync::Arc;

/// Minute cadence (installed in the Needs slot): a seeded random walk on `probe.value`.
pub struct DevProbeSystem;

impl System for DevProbeSystem {
    fn id(&self) -> &str {
        "dev.probe"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let index = i64::try_from(ctx.world.probe.minutes).unwrap_or(i64::MAX);
        let rng = Rng::new(ctx.world.seed(), Stream::DevProbe, &[Key::Int(index)]);
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

/// The scaffolding plan source (D-009): a work duty in the morning and afternoon at a seeded "workplace"
/// tile, and leisure that strolls to a random tile or lingers. Stands in for occupation templates and
/// needs until Stage 1; the schedule, tasks and movement it drives are the real systems.
pub struct DevPlanSource;

impl DevPlanSource {
    /// A seeded passable tile on the pawn's map; `day` of -1 gives the pawn's fixed workplace.
    fn tile(world: &WorldState, pawn: EntityId, day: i64, salt: i64) -> Option<Tile> {
        let p = world.pawns.get(pawn)?;
        let map = world.maps.get(p.position.map)?;
        let rng = Rng::new(
            world.seed(),
            Stream::DevPlan,
            &[Key::Id(pawn), Key::Int(day), Key::Int(salt)],
        );
        (0..200u32).find_map(|c| {
            let x = rng.int_in(c.wrapping_mul(2), 0, map.width() - 1)?;
            let y = rng.int_in(c.wrapping_mul(2).wrapping_add(1), 0, map.height() - 1)?;
            let t = Tile::new(x, y);
            world.is_passable(p.position.map, t).then_some(t)
        })
    }
}

fn at(tile: Tile) -> Canon {
    Canon::map([("at", tile.to_canon())])
}

impl PlanSource for DevPlanSource {
    fn id(&self) -> &str {
        "dev.plan"
    }

    fn inputs(&self, world: &WorldState, pawn: EntityId, day: u64) -> PlanInputs {
        let slots = world.settings.slot_minutes.slots_per_day();
        let hour = |h: u32| slots.saturating_mul(h) / 24;
        let day = i64::try_from(day).unwrap_or(i64::MAX);
        let (Ok(idle), Ok(walk)) = (ActionId::new("idle_at"), ActionId::new("move_to")) else {
            return PlanInputs::default();
        };
        let mut inputs = PlanInputs {
            open_weight: 15,
            ..PlanInputs::default()
        };
        if let Some(work) = DevPlanSource::tile(world, pawn, -1, 0) {
            for (start, end) in [(9, 12), (13, 17)] {
                let len = hour(end) - hour(start);
                inputs.duties.push(DutyTemplate {
                    action: idle.clone(),
                    params: at(work),
                    start: hour(start),
                    len,
                    min_len: (len * 2 / 3).max(1),
                    shift_earlier: hour(1) / 2,
                    shift_later: hour(1) / 2,
                });
            }
        }
        for salt in 1..=3 {
            if let Some(stroll) = DevPlanSource::tile(world, pawn, day, salt) {
                inputs.leisure.push(LeisureOption {
                    action: walk.clone(),
                    params: Canon::map([("to", stroll.to_canon())]),
                    len: 1,
                    weight: 4,
                });
            }
        }
        if let Some(spot) = DevPlanSource::tile(world, pawn, day, 4) {
            inputs.leisure.push(LeisureOption {
                action: idle,
                params: at(spot),
                len: 2,
                weight: 10,
            });
        }
        inputs
    }
}

/// Installs the dev systems: probe (Needs), the day counter (before the DayPlanner), and the dev plan
/// source behind the real planner and activator.
pub fn install(pipeline: &mut Pipeline) {
    let plans: Arc<dyn PlanSource> = Arc::new(DevPlanSource);
    pipeline.set_builtin(SystemSlot::Needs, Box::new(DevProbeSystem));
    pipeline.set_builtin(
        SystemSlot::DayPlanner,
        Box::new(DayPlannerSystem::new(Arc::clone(&plans))),
    );
    pipeline.set_builtin(
        SystemSlot::ReservationActivator,
        Box::new(ReservationActivatorSystem::new(plans)),
    );
    // The id is unique, so registration cannot fail.
    let _ = pipeline.add_extension(
        SystemSlot::DayPlanner,
        Placement::Before,
        Cadence::Day,
        Box::new(DevDaySystem),
    );
}

/// Fills a freshly created map. Style 0 leaves open grass. Style 1 makes a small town: a road grid,
/// sidewalks, seeded building footprints in most blocks, and a sandy-shored pond in the middle.
pub fn generate_dev_map(world: &mut WorldState, id: EntityId, style: u8) {
    if style == 0 {
        return;
    }
    const BLOCK: i32 = 12;
    let seed = world.seed();
    let Some(map) = world.maps.get_mut(id) else {
        return;
    };
    let (w, h) = (map.width(), map.height());
    let (cx, cy) = (w / 2, h / 2);
    let radius = (w.min(h) / 6).max(2);
    for y in 0..h {
        for x in 0..w {
            let t = Tile::new(x, y);
            let (mx, my) = (x % BLOCK, y % BLOCK);
            let on_road = mx == 0 || my == 0;
            if on_road {
                let _ = map.set_surface(t, ROAD);
            } else if mx == 1 || my == 1 || mx == BLOCK - 1 || my == BLOCK - 1 {
                let _ = map.set_surface(t, SIDEWALK);
            }
            let d = (x - cx).abs() + (y - cy).abs();
            if !on_road {
                if d <= radius {
                    let _ = map.set_terrain(t, WATER);
                } else if d == radius + 1 {
                    let _ = map.set_terrain(t, SAND);
                }
            }
        }
    }
    let chance = Permille::saturating(600);
    for by in 0..(h / BLOCK) {
        for bx in 0..(w / BLOCK) {
            let rng = Rng::new(
                seed,
                Stream::DevMap,
                &[
                    Key::Id(id),
                    Key::Int(i64::from(bx)),
                    Key::Int(i64::from(by)),
                ],
            );
            if !rng.chance(0, chance) {
                continue;
            }
            for y in (by * BLOCK + 3)..=(by * BLOCK + 8) {
                for x in (bx * BLOCK + 3)..=(bx * BLOCK + 8) {
                    let t = Tile::new(x, y);
                    if map
                        .terrain_at(t)
                        .is_some_and(|ter| ter != WATER && ter != SAND)
                    {
                        let _ = map.set_blocked(t, true);
                    }
                }
            }
        }
    }
}

/// A seeded random tile of `map` that a pawn could be placed on now, or `None` if none is found.
pub fn random_free_tile(world: &WorldState, map: EntityId) -> Option<Tile> {
    let m = world.maps.get(map)?;
    let rng = Rng::new(
        world.seed(),
        Stream::DevSpawn,
        &[
            Key::Id(map),
            Key::Int(i64::try_from(world.pawns.len()).unwrap_or(0)),
        ],
    );
    (0..400u32).find_map(|c| {
        let x = rng.int_in(c.wrapping_mul(2), 0, m.width() - 1)?;
        let y = rng.int_in(c.wrapping_mul(2).wrapping_add(1), 0, m.height() - 1)?;
        let t = Tile::new(x, y);
        (world.is_passable(map, t) && world.occupancy.occupant(map, t).is_none()).then_some(t)
    })
}
