//! The movement system (Blueprint §7.4), run every tick in the `MovementSystem` slot.
//!
//! 1. **Repaths.** Pawns whose route wants a (re)solve are solved as one batch (through the executor and
//!    cache). A found path replaces the route's path; a failure ends the route with a reason.
//! 2. **Steps.** Pawns with a path, in ascending id order, count ticks toward the next step. When
//!    `move_ticks_per_tile` have elapsed:
//!    * if the next tile became impassable (the map was edited), ask for a repath;
//!    * if another pawn stands on it, wait; the lower id is processed first, so it claims a contested
//!      tile. After `max_wait_ticks` try a one-tile sidestep (sideways in preference to backwards, then
//!      toward the goal, then N, E, S, W order) and ask for a repath;
//!    * otherwise move, update occupancy and facing.
//!
//!    After `max_repaths` re-solves the route fails with `path_blocked`. A pawn with nowhere to step
//!    aside gives up after the whole wait-and-repath budget, so a head-on deadlock cannot hang.
//!
//! Failures never panic and never leave a half-updated pawn: the route is cleared and an event is emitted.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::map::{Dir4, Tile};
use crate::path::{solve_cached, PathJob, PathOutcome};
use crate::pipeline::{System, TickCtx};
use crate::world::MovementSettings;

pub struct MovementSystem;

impl System for MovementSystem {
    fn id(&self) -> &str {
        "MovementSystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let settings = ctx.world.settings.movement;
        resolve_repaths(ctx, &settings);
        step_pawns(ctx, &settings);
    }
}

fn event(ctx: &mut TickCtx<'_>, kind: &str, pawn: EntityId, extra: (&str, Canon)) {
    ctx.emit(
        kind,
        Canon::map([("pawn", pawn.to_canon()), (extra.0, extra.1)]),
    );
    ctx.mark_changed(pawn);
}

fn fail(ctx: &mut TickCtx<'_>, pawn: EntityId, reason: &str) {
    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
        p.route = None;
    }
    event(ctx, "move.failed", pawn, ("reason", Canon::str(reason)));
}

fn arrive(ctx: &mut TickCtx<'_>, pawn: EntityId) {
    let tile = ctx.world.pawns.get(pawn).map(|p| p.position.tile);
    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
        p.route = None;
    }
    if let Some(t) = tile {
        event(ctx, "move.arrived", pawn, ("tile", t.to_canon()));
    }
}

fn resolve_repaths(ctx: &mut TickCtx<'_>, settings: &MovementSettings) {
    let wanting: Vec<(EntityId, EntityId, Tile, Tile)> = ctx
        .world
        .pawns
        .iter()
        .filter_map(|(id, p)| {
            let r = p.route.as_ref().filter(|r| r.needs_repath)?;
            Some((id, p.position.map, p.position.tile, r.goal))
        })
        .collect();
    if wanting.is_empty() {
        return;
    }

    // Solve the whole batch at once. Jobs borrow the maps and the cost table; the executor and cache are
    // separate fields of the services, so all three can be borrowed together.
    let outcomes: Vec<Option<PathOutcome>> = {
        let world = &*ctx.world;
        let crate::pipeline::Services { exec, paths, costs } = &mut *ctx.services;
        let mut jobs = Vec::new();
        let mut slot_of_job = Vec::new();
        for (i, (_, map_id, from, goal)) in wanting.iter().enumerate() {
            if let Some(map) = world.maps.get(*map_id) {
                jobs.push(PathJob {
                    map,
                    costs,
                    from: *from,
                    to: *goal,
                    cap: settings.path_expansion_cap,
                });
                slot_of_job.push(i);
            }
        }
        let solved = solve_cached(&**exec, paths, &jobs);
        let mut out: Vec<Option<PathOutcome>> = vec![None; wanting.len()];
        for (job_index, outcome) in solved.into_iter().enumerate() {
            if let Some(slot) = slot_of_job.get(job_index).and_then(|&s| out.get_mut(s)) {
                *slot = Some(outcome);
            }
        }
        out
    };

    for ((pawn, ..), outcome) in wanting.into_iter().zip(outcomes) {
        match outcome {
            None => fail(ctx, pawn, "unknown_map"),
            Some(PathOutcome::Found { path, .. }) => {
                let (exceeded, arrived) = {
                    let Some(route) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.route.as_mut())
                    else {
                        continue;
                    };
                    if !route.path.is_empty() {
                        route.repaths = route.repaths.saturating_add(1); // a re-solve, not the first solve
                    }
                    route.path = path;
                    route.next = 0;
                    route.needs_repath = false;
                    route.waited = 0;
                    (route.repaths > settings.max_repaths, route.path.is_empty())
                };
                if exceeded {
                    fail(ctx, pawn, "path_blocked");
                } else if arrived {
                    arrive(ctx, pawn);
                }
            }
            Some(other) => fail(
                ctx,
                pawn,
                other.failure_reason().unwrap_or("unreachable_or_too_far"),
            ),
        }
    }
}

fn step_pawns(ctx: &mut TickCtx<'_>, settings: &MovementSettings) {
    let walkers: Vec<EntityId> = ctx
        .world
        .pawns
        .iter()
        .filter(|(_, p)| p.route.as_ref().is_some_and(|r| !r.needs_repath))
        .map(|(id, _)| id)
        .collect();
    for pawn in walkers {
        step_one(ctx, pawn, settings);
    }
}

/// Moves `pawn` to `to`, keeping occupancy, position and facing in step.
fn relocate(ctx: &mut TickCtx<'_>, pawn: EntityId, map: EntityId, from: Tile, to: Tile) -> bool {
    if ctx.world.occupancy.place(map, to, pawn).is_err() {
        return false;
    }
    ctx.world.occupancy.vacate(map, from, pawn);
    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
        p.position.tile = to;
        p.facing = Dir4::between(from, to).unwrap_or(p.facing);
    }
    ctx.mark_changed(pawn);
    true
}

fn can_enter(ctx: &TickCtx<'_>, map: EntityId, tile: Tile) -> bool {
    ctx.world
        .maps
        .get(map)
        .is_some_and(|m| ctx.services.costs.step_cost(m, tile).is_some())
}

fn step_one(ctx: &mut TickCtx<'_>, pawn: EntityId, settings: &MovementSettings) {
    // Count the tick and see whether a step is due.
    let (map, cur, next) = {
        let Some(p) = ctx.world.pawns.get_mut(pawn) else {
            return;
        };
        let (map, cur) = (p.position.map, p.position.tile);
        let Some(route) = p.route.as_mut() else {
            return;
        };
        if route.needs_repath {
            return;
        }
        route.since_step = route.since_step.saturating_add(1);
        if route.since_step < settings.move_ticks_per_tile {
            return;
        }
        match route.path.get(route.next).copied() {
            Some(next) => (map, cur, next),
            None => {
                arrive(ctx, pawn);
                return;
            }
        }
    };

    // The map may have been edited under the route.
    if !can_enter(ctx, map, next) {
        if let Some(r) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.route.as_mut()) {
            r.needs_repath = true;
        }
        return;
    }

    match ctx.world.occupancy.occupant(map, next) {
        None => {
            if relocate(ctx, pawn, map, cur, next) {
                let done = {
                    let Some(route) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.route.as_mut())
                    else {
                        return;
                    };
                    route.next += 1;
                    route.since_step = 0;
                    route.waited = 0;
                    route.next >= route.path.len()
                };
                if done {
                    arrive(ctx, pawn);
                }
            }
        }
        Some(_) => {
            let waited = {
                let Some(route) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.route.as_mut())
                else {
                    return;
                };
                route.waited = route.waited.saturating_add(1);
                route.waited
            };
            if waited >= settings.max_wait_ticks {
                try_sidestep(ctx, pawn, map, cur, next);
            }
        }
    }
}

/// After waiting too long, step to a free neighbor and ask for a fresh path.
///
/// Free neighbors are ranked by `(is it straight backwards, distance to the goal, N-E-S-W order)`,
/// so a pawn steps *aside* rather than backwards, and among sideways options prefers the one closest to
/// where it is going. Backing up is a last resort, because it only swaps the two pawns' problem.
fn try_sidestep(
    ctx: &mut TickCtx<'_>,
    pawn: EntityId,
    map: EntityId,
    cur: Tile,
    blocked_next: Tile,
) {
    let goal = ctx
        .world
        .pawns
        .get(pawn)
        .and_then(|p| p.route.as_ref())
        .map_or(cur, |r| r.goal);
    // "Backwards" is the tile directly opposite the one we are blocked on.
    let behind = Tile::new(
        cur.x.saturating_mul(2).saturating_sub(blocked_next.x),
        cur.y.saturating_mul(2).saturating_sub(blocked_next.y),
    );
    let mut options: Vec<(bool, u32, usize, Tile)> = Dir4::ALL
        .into_iter()
        .enumerate()
        .map(|(order, d)| {
            let n = d.step(cur);
            (n == behind, n.manhattan(goal), order, n)
        })
        .filter(|(_, _, _, n)| {
            *n != blocked_next
                && can_enter(ctx, map, *n)
                && ctx.world.occupancy.occupant(map, *n).is_none()
        })
        .collect();
    options.sort();
    let free = options.first().map(|(_, _, _, n)| *n);
    let Some(side) = free else {
        // Nowhere to step aside. Keep waiting, but not forever: a head-on deadlock in a one-tile corridor
        // would otherwise never resolve, so give up after the whole wait-and-repath budget is spent.
        let settings = ctx.world.settings.movement;
        let budget = settings
            .max_wait_ticks
            .saturating_mul(settings.max_repaths.saturating_add(1));
        let waited = ctx
            .world
            .pawns
            .get(pawn)
            .and_then(|p| p.route.as_ref())
            .map_or(0, |r| r.waited);
        if waited >= budget {
            fail(ctx, pawn, "path_blocked");
        }
        return;
    };
    if relocate(ctx, pawn, map, cur, side) {
        if let Some(r) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.route.as_mut()) {
            r.needs_repath = true;
            r.waited = 0;
            r.since_step = 0;
        }
        event(ctx, "move.sidestep", pawn, ("tile", side.to_canon()));
    }
}

#[cfg(test)]
mod tests;
