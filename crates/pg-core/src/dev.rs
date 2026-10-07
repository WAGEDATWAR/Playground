//! **Dev scaffolding systems and generators** for milestones 0.2–0.4.
//!
//! These give the tick pipeline something deterministic and RNG-dependent to do, so replay, hashing,
//! snapshot and movement tests are meaningful before real simulation systems exist. They are removed (or
//! moved under `#[cfg(test)]`) when the real systems land (D-009).

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::map::{Tile, ROAD, SAND, SIDEWALK, WATER};
use crate::num::Permille;
use crate::pawn::Route;
use crate::pipeline::{Cadence, Pipeline, Placement, System, SystemSlot, TickCtx};
use crate::rng::{Key, Rng};
use crate::world::WorldState;

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

/// Minute cadence (after the TaskPlanner slot): each idle pawn has a small chance per minute to pick a
/// random reachable tile and walk there. Stands in for the task planner until the scheduler lands (0.5).
pub struct DevWanderSystem;

impl System for DevWanderSystem {
    fn id(&self) -> &str {
        "dev.wander"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let minute = i64::try_from(ctx.flags.tick / 10).unwrap_or(i64::MAX);
        let seed = ctx.world.seed();
        let chance = Permille::saturating(100);
        let idle: Vec<(EntityId, EntityId, Tile)> = ctx
            .world
            .pawns
            .iter()
            .filter(|(_, p)| p.route.is_none())
            .map(|(id, p)| (id, p.position.map, p.position.tile))
            .collect();
        for (pawn, map, here) in idle {
            let rng = Rng::new(seed, "dev.wander", &[Key::Id(pawn), Key::Int(minute)]);
            if !rng.chance(0, chance) {
                continue;
            }
            let Some((w, h)) = ctx.world.maps.get(map).map(|m| (m.width(), m.height())) else {
                continue;
            };
            let (Some(x), Some(y)) = (rng.int_in(1, 0, w - 1), rng.int_in(2, 0, h - 1)) else {
                continue;
            };
            let goal = Tile::new(x, y);
            if goal != here && ctx.world.is_passable(map, goal) {
                if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                    p.route = Some(Route::to(goal));
                }
            }
        }
    }
}

/// Installs the dev systems: probe (Needs), day counter (DayPlanner), wandering (after TaskPlanner).
pub fn install(pipeline: &mut Pipeline) {
    pipeline.set_builtin(SystemSlot::Needs, Box::new(DevProbeSystem));
    pipeline.set_builtin(SystemSlot::DayPlanner, Box::new(DevDaySystem));
    // The id is unique, so registration cannot fail.
    let _ = pipeline.add_extension(
        SystemSlot::TaskPlanner,
        Placement::After,
        Cadence::Minute,
        Box::new(DevWanderSystem),
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
                "dev.map",
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
        "dev.spawn",
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
