//! The systems that turn schedules into behaviour (Blueprint §8.5-§8.7).
//!
//! ```text
//! DayPlanner (day) ──▶ schedule per pawn ──▶ ReservationActivator (slot): replans, then
//!                                              intent = reservation | Free
//!                                           ──▶ TaskPlanner (tick): intent -> Task via the ActionRegistry
//!                                           ──▶ MovementSystem ──▶ ActivitySystem (tick): runs steps
//! CommitmentSystem (slot): proposals ──▶ both schedules reserve, or neither
//! ```
//!
//! Every failure ends the task with a [`ReasonCode`], emits `task.failed`, and asks the replanner to redo
//! the rest of the day. A [`PlanSource`] supplies the planner's inputs: until Stage 1 brings needs and
//! occupations, only the dev scaffolding provides one.

use crate::action::{ActionRegistry, MEET_RADIUS};
use crate::canon::{Canon, ToCanon};
use crate::commitment::{CommitState, Commitment};
use crate::id::EntityId;
use crate::map::Tile;
use crate::pawn::{Intent, Replan, Step, Task};
use crate::pipeline::{System, TickCtx};
use crate::reason::ReasonCode;
use crate::schedule::{plan_day_from, replan_from, reserve_commitment, PlanInputs};
use crate::time::TICKS_PER_DAY;
use crate::world::WorldState;
use std::sync::Arc;

/// Supplies the planner's inputs for a pawn's day. Pure: it reads the world and decides nothing random
/// except through seeded streams.
pub trait PlanSource: Send + Sync {
    fn id(&self) -> &str;
    fn inputs(&self, world: &WorldState, pawn: EntityId, day: u64) -> PlanInputs;
}

/// Plans nothing: every pawn's day is open. The default until a real source exists.
pub struct NoPlans;

impl PlanSource for NoPlans {
    fn id(&self) -> &str {
        "none"
    }

    fn inputs(&self, _world: &WorldState, _pawn: EntityId, _day: u64) -> PlanInputs {
        PlanInputs::default()
    }
}

/// The source's inputs plus the day's binding commitments that involve `pawn`.
pub fn gather_inputs(
    source: &dyn PlanSource,
    world: &WorldState,
    pawn: EntityId,
    day: u64,
) -> PlanInputs {
    let mut inputs = source.inputs(world, pawn, day);
    inputs.commitments.extend(
        world
            .commitments
            .iter()
            .filter(|(_, c)| c.is_binding() && c.day == day && c.parties().contains(&pawn))
            .map(|(_, c)| c.as_req()),
    );
    inputs
}

fn pawn_ids(world: &WorldState) -> Vec<EntityId> {
    world.pawns.iter().map(|(id, _)| id).collect()
}

fn slots_per_day(world: &WorldState) -> u32 {
    world.settings.slot_minutes.slots_per_day()
}

/// Plans every pawn's day at the day boundary.
pub struct DayPlannerSystem {
    source: Arc<dyn PlanSource>,
}

impl DayPlannerSystem {
    pub fn new(source: Arc<dyn PlanSource>) -> DayPlannerSystem {
        DayPlannerSystem { source }
    }
}

fn plan_pawn(ctx: &mut TickCtx<'_>, source: &dyn PlanSource, pawn: EntityId, from: u32) {
    let day = ctx.flags.day;
    let inputs = gather_inputs(source, ctx.world, pawn, day);
    let schedule = plan_day_from(
        ctx.world.seed(),
        pawn,
        day,
        slots_per_day(ctx.world),
        from,
        &inputs,
    );
    let (reserved, dropped) = (
        u32::try_from(schedule.reservations().count()).unwrap_or(u32::MAX),
        u32::try_from(schedule.dropped.len()).unwrap_or(u32::MAX),
    );
    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
        p.schedule = Some(schedule);
        p.replan = None;
    }
    ctx.emit(
        "schedule.planned",
        Canon::map([
            ("pawn", pawn.to_canon()),
            ("day", day.to_canon()),
            ("reservations", reserved.to_canon()),
            ("dropped", dropped.to_canon()),
        ]),
    );
    ctx.mark_changed(pawn);
}

impl System for DayPlannerSystem {
    fn id(&self) -> &str {
        "DayPlanner"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        for pawn in pawn_ids(ctx.world) {
            end_task(ctx, pawn);
            if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                p.intent = Intent::Free;
            }
            plan_pawn(ctx, self.source.as_ref(), pawn, 0);
        }
    }
}

/// At a slot boundary: plans pawns that have no schedule for today (spawned mid-day) and carries out
/// requested replans. Work always starts at the boundary, so a slot already under way is never rewritten.
fn replan_pass(ctx: &mut TickCtx<'_>, source: &dyn PlanSource) {
    let boundary = ctx.flags.slot_of_day;
    for pawn in pawn_ids(ctx.world) {
        let Some(p) = ctx.world.pawns.get(pawn) else {
            continue;
        };
        let slots = slots_per_day(ctx.world);
        let stale = p
            .schedule
            .as_ref()
            .is_none_or(|s| s.day != ctx.flags.day || s.slots_per_day() != slots);
        if stale {
            plan_pawn(ctx, source, pawn, boundary);
            continue;
        }
        let Some(request) = p.replan.clone() else {
            continue;
        };
        let from = request.from.max(boundary);
        let inputs = gather_inputs(source, ctx.world, pawn, ctx.flags.day);
        let seed = ctx.world.seed();
        if let Some(p) = ctx.world.pawns.get_mut(pawn) {
            if let Some(s) = p.schedule.as_mut() {
                replan_from(s, seed, pawn, from, &inputs, &request.why);
            }
            p.replan = None;
        }
        ctx.emit(
            "schedule.replanned",
            Canon::map([
                ("pawn", pawn.to_canon()),
                ("from", from.to_canon()),
                ("why", Canon::str(request.why)),
            ]),
        );
        ctx.mark_changed(pawn);
    }
}

/// Sets each pawn's intent to the reservation covering the slot that just began (after any replans).
pub struct ReservationActivatorSystem {
    source: Arc<dyn PlanSource>,
}

impl ReservationActivatorSystem {
    pub fn new(source: Arc<dyn PlanSource>) -> ReservationActivatorSystem {
        ReservationActivatorSystem { source }
    }
}

impl System for ReservationActivatorSystem {
    fn id(&self) -> &str {
        "ReservationActivator"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        replan_pass(ctx, self.source.as_ref());
        for pawn in pawn_ids(ctx.world) {
            let Some(p) = ctx.world.pawns.get(pawn) else {
                continue;
            };
            let wanted = p
                .schedule
                .as_ref()
                .filter(|s| s.day == ctx.flags.day)
                .and_then(|s| s.owner_id(ctx.flags.slot_of_day))
                .map_or(Intent::Free, Intent::Reservation);
            // A reservation lasts several slots; its task carries on across them.
            let carries_on = matches!(
                (&p.task, wanted),
                (Some(t), Intent::Reservation(id)) if t.reservation == id
            );
            if carries_on || p.intent == wanted {
                continue;
            }
            end_task(ctx, pawn);
            if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                p.intent = wanted;
            }
            ctx.mark_changed(pawn);
        }
    }
}

/// Drops a pawn's task (and its route, if the task was walking).
fn end_task(ctx: &mut TickCtx<'_>, pawn: EntityId) {
    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
        if let Some(t) = p.task.take() {
            if t.moving {
                p.route = None;
            }
        }
    }
}

/// Ends the pawn's task with a reason, records it, and (usually) asks for a replan of the rest of the day.
fn fail_task(ctx: &mut TickCtx<'_>, pawn: EntityId, reason: ReasonCode, replan: bool) {
    let slot = ctx.flags.slot_of_day;
    let Some(p) = ctx.world.pawns.get_mut(pawn) else {
        return;
    };
    let reservation = match (&p.task, p.intent) {
        (Some(t), _) => Some(t.reservation),
        (None, Intent::Reservation(id)) => Some(id),
        (None, Intent::Free) => None,
    };
    if p.task.take().is_some_and(|t| t.moving) {
        p.route = None;
    }
    p.intent = Intent::Free;
    p.last_failure = Some(reason.clone());
    if replan {
        p.replan = Some(Replan {
            from: slot + 1,
            why: reason.explain(),
        });
    }
    ctx.emit(
        "task.failed",
        Canon::map([
            ("pawn", pawn.to_canon()),
            (
                "reservation",
                reservation.map_or(Canon::Null, |r| r.to_canon()),
            ),
            ("reason", reason.to_canon()),
            ("explain", Canon::str(reason.explain())),
        ]),
    );
    ctx.mark_changed(pawn);
}

/// Turns a pawn's intent into a task using the action registry.
pub struct TaskPlannerSystem {
    registry: Arc<ActionRegistry>,
}

impl TaskPlannerSystem {
    pub fn new(registry: Arc<ActionRegistry>) -> TaskPlannerSystem {
        TaskPlannerSystem { registry }
    }
}

impl System for TaskPlannerSystem {
    fn id(&self) -> &str {
        "TaskPlanner"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let slot_ticks = ctx.world.settings.slot_minutes.ticks();
        let day_start = ctx.flags.day.saturating_mul(TICKS_PER_DAY);
        for pawn in pawn_ids(ctx.world) {
            let Some(p) = ctx.world.pawns.get(pawn) else {
                continue;
            };
            let Intent::Reservation(id) = p.intent else {
                continue;
            };
            if p.task.as_ref().is_some_and(|t| t.reservation == id) || !p.capacities.can_act() {
                continue;
            }
            let reservation = p
                .schedule
                .as_ref()
                .and_then(|s| s.reservation(id))
                .map(|r| (r.action.clone(), r.params.clone(), r.end()));
            let Some((action, params, end_slot)) = reservation else {
                // The reservation vanished (replanned away): nothing to do.
                if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                    p.intent = Intent::Free;
                }
                continue;
            };
            // Performing ends on the last tick of the reservation: at the next boundary the activator has
            // already moved the pawn on.
            let end_tick = day_start
                .saturating_add(u64::from(end_slot).saturating_mul(slot_ticks))
                .saturating_sub(1);
            match self.registry.instantiate(&action, &params, end_tick) {
                Ok(inst) => {
                    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                        p.task = Some(Task {
                            reservation: id,
                            action: action.clone(),
                            steps: inst.steps,
                            current: 0,
                            moving: false,
                            interruptible: inst.interruptible,
                        });
                    }
                    ctx.emit(
                        "task.started",
                        Canon::map([
                            ("pawn", pawn.to_canon()),
                            ("reservation", id.to_canon()),
                            ("action", action.to_canon()),
                        ]),
                    );
                    ctx.mark_changed(pawn);
                }
                Err(reason) => fail_task(ctx, pawn, reason, true),
            }
        }
    }
}

/// Runs the current step of each pawn's task.
pub struct ActivitySystem;

impl System for ActivitySystem {
    fn id(&self) -> &str {
        "ActivitySystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        for pawn in pawn_ids(ctx.world) {
            let Some(p) = ctx.world.pawns.get(pawn) else {
                continue;
            };
            let Some(task) = &p.task else { continue };
            let still_planned = p
                .schedule
                .as_ref()
                .is_some_and(|s| s.reservation(task.reservation).is_some());
            if !still_planned {
                fail_task(ctx, pawn, ReasonCode::simple("interrupted"), false);
                continue;
            }
            advance(ctx, pawn);
        }
    }
}

/// Moves a task along as far as it can go this tick.
fn advance(ctx: &mut TickCtx<'_>, pawn: EntityId) {
    // A step can finish and the next begin in the same tick; the step count bounds the loop.
    let limit = ctx
        .world
        .pawns
        .get(pawn)
        .and_then(|p| p.task.as_ref())
        .map_or(0, |t| t.steps.len() + 1);
    for _ in 0..limit {
        let Some(p) = ctx.world.pawns.get(pawn) else {
            return;
        };
        let Some(task) = &p.task else { return };
        let (here, map, route_idle) = (p.position.tile, p.position.map, p.route.is_none());
        let moving = task.moving;
        match task.step().copied() {
            None => return, // done; it stays on the pawn until the intent changes

            Some(Step::MoveTo { goal, within }) => {
                if here.manhattan(goal) <= within {
                    if moving {
                        if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                            p.route = None; // close enough: stop walking
                        }
                    }
                    step_done(ctx, pawn);
                } else if !moving {
                    let Some(target) = walk_target(ctx.world, pawn, map, goal, within) else {
                        fail_task(ctx, pawn, ReasonCode::simple("blocked_destination"), true);
                        return;
                    };
                    if let Some(p) = ctx.world.pawns.get_mut(pawn) {
                        p.route = Some(crate::pawn::Route::to(target));
                        p.move_failure = None;
                        if let Some(t) = p.task.as_mut() {
                            t.moving = true;
                        }
                    }
                    return;
                } else if route_idle {
                    // The movement system names why it gave up; anything unexpected reads as unreachable.
                    let code = p
                        .move_failure
                        .as_deref()
                        .and_then(|f| ReasonCode::known_codes().find(|k| *k == f))
                        .unwrap_or("unreachable_or_too_far");
                    fail_task(ctx, pawn, ReasonCode::simple(code), true);
                    return;
                } else {
                    return;
                }
            }
            Some(Step::PerformUntil(end)) if ctx.flags.tick >= end => step_done(ctx, pawn),
            Some(Step::PerformUntil(_)) => return,
        }
    }
}

/// The tile to walk to for a step: the goal itself when it can be stood on and is not another pawn's
/// tile; otherwise, for a gathering step (`within` > 0), the nearest free tile inside the radius, by
/// (distance, y, x). `None` when nothing qualifies.
fn walk_target(
    world: &WorldState,
    pawn: EntityId,
    map: EntityId,
    goal: Tile,
    within: u32,
) -> Option<Tile> {
    let free = |t: Tile| {
        world.is_passable(map, t)
            && world
                .occupancy
                .occupant(map, t)
                .is_none_or(|other| other == pawn)
    };
    if free(goal) {
        return Some(goal);
    }
    if within == 0 {
        // An exact destination that is merely occupied is the movement system's problem (it waits and
        // gives up with path_blocked); one that can never be stood on fails the task now.
        return world.is_passable(map, goal).then_some(goal);
    }
    let r = i32::try_from(within).unwrap_or(0);
    let mut best: Option<(u32, i32, i32)> = None;
    for dy in -r..=r {
        for dx in -r..=r {
            let t = Tile::new(goal.x.saturating_add(dx), goal.y.saturating_add(dy));
            let d = t.manhattan(goal);
            if d <= within && free(t) {
                let key = (d, t.y, t.x);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
        }
    }
    best.map(|(_, y, x)| Tile::new(x, y))
}

fn step_done(ctx: &mut TickCtx<'_>, pawn: EntityId) {
    let mut done = None;
    if let Some(t) = ctx.world.pawns.get_mut(pawn).and_then(|p| p.task.as_mut()) {
        t.current += 1;
        t.moving = false;
        if t.is_done() {
            done = Some((t.reservation, t.action.clone()));
        }
    }
    if let Some((reservation, action)) = done {
        ctx.emit(
            "task.done",
            Canon::map([
                ("pawn", pawn.to_canon()),
                ("reservation", reservation.to_canon()),
                ("action", action.to_canon()),
            ]),
        );
    }
    ctx.mark_changed(pawn);
}

/// Moves commitments through the protocol at each slot boundary.
pub struct CommitmentSystem;

impl System for CommitmentSystem {
    fn id(&self) -> &str {
        "CommitmentSystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let mut order: Vec<(u64, EntityId)> = ctx
            .world
            .commitments
            .iter()
            .filter(|(_, c)| !c.state.is_terminal())
            .map(|(id, c)| (c.created_tick, id))
            .collect();
        order.sort();
        for (_, id) in order {
            let Some(c) = ctx.world.commitments.get(id).cloned() else {
                continue;
            };
            let outcome = match c.state {
                CommitState::Proposed => evaluate_proposal(ctx, &c),
                CommitState::Accepted | CommitState::Active => progress(ctx, &c),
                _ => None,
            };
            if let Some((state, reason)) = outcome {
                set_state(ctx, id, state, reason);
            }
        }
    }
}

fn set_state(ctx: &mut TickCtx<'_>, id: EntityId, state: CommitState, reason: ReasonCode) {
    let Some(c) = ctx.world.commitments.get_mut(id) else {
        return;
    };
    let explain = reason.explain();
    if c.transition(state, reason).is_err() {
        return;
    }
    let parties = c.parties();
    ctx.emit(
        format!("commitment.{}", state.name()),
        Canon::map([
            ("commitment", id.to_canon()),
            ("explain", Canon::str(explain)),
        ]),
    );
    for p in parties {
        ctx.mark_changed(p);
    }
}

type Outcome = Option<(CommitState, ReasonCode)>;

/// The invitee's answer, evaluated at a slot boundary. Both schedules reserve, or neither does.
fn evaluate_proposal(ctx: &mut TickCtx<'_>, c: &Commitment) -> Outcome {
    if ctx.flags.tick >= c.expires_tick || c.day < ctx.flags.day {
        return Some((
            CommitState::Expired,
            ReasonCode::simple("commitment_expired"),
        ));
    }
    if c.day > ctx.flags.day {
        return None; // for a later day: wait (the expiry above still applies)
    }
    let decline = |why: String| {
        Some((
            CommitState::Declined,
            ReasonCode::builtin("commitment_declined", [("why", Canon::str(why))]),
        ))
    };
    let (Some(a), Some(b)) = (
        ctx.world
            .pawns
            .get(c.proposer)
            .and_then(|p| p.schedule.clone()),
        ctx.world
            .pawns
            .get(c.invitee)
            .and_then(|p| p.schedule.clone()),
    ) else {
        return decline("a participant has no plan for today".to_owned());
    };
    let slots = slots_per_day(ctx.world);
    if a.day != c.day || b.day != c.day || a.slots_per_day() != slots || b.slots_per_day() != slots
    {
        return decline("a participant has no plan for today".to_owned());
    }
    // Work on copies so a failure on either side leaves both untouched.
    let (mut a, mut b) = (a, b);
    let req = c.as_req();
    let from = ctx.flags.slot_of_day;
    for (who, schedule) in [(c.proposer, &mut a), (c.invitee, &mut b)] {
        if let Err(reason) = reserve_commitment(schedule, &req, from) {
            return decline(format!("{who}: {}", reason.explain()));
        }
    }
    for (who, schedule) in [(c.proposer, a), (c.invitee, b)] {
        if let Some(p) = ctx.world.pawns.get_mut(who) {
            p.schedule = Some(schedule);
        }
    }
    Some((
        CommitState::Accepted,
        ReasonCode::builtin(
            "commitment_accepted",
            [
                ("start", Canon::Int(i128::from(c.start))),
                ("end", Canon::Int(i128::from(c.start.saturating_add(c.len)))),
            ],
        ),
    ))
}

/// Accepted -> Active -> Completed/Failed.
fn progress(ctx: &mut TickCtx<'_>, c: &Commitment) -> Outcome {
    let fail = |why: String| {
        Some((
            CommitState::Failed,
            ReasonCode::builtin("commitment_no_show", [("why", Canon::str(why))]),
        ))
    };
    if c.day < ctx.flags.day {
        return fail("the day ended before it was kept".to_owned());
    }
    let held = |who: EntityId| {
        ctx.world
            .pawns
            .get(who)
            .and_then(|p| p.schedule.as_ref())
            .is_some_and(|s| s.reservations().any(|r| r.commitment == Some(c.id)))
    };
    if let Some(missing) = c.parties().into_iter().find(|w| !held(*w)) {
        return fail(format!("{missing} no longer has it in their schedule"));
    }
    let slot = ctx.flags.slot_of_day;
    let end = c.start.saturating_add(c.len);
    if c.state == CommitState::Accepted {
        return (slot >= c.start)
            .then(|| (CommitState::Active, ReasonCode::simple("commitment_active")));
    }
    if slot < end {
        return None;
    }
    let place = c.params.get("at").and_then(Tile::from_canon);
    let absent: Vec<String> = c
        .parties()
        .into_iter()
        .filter(|w| {
            place.is_some_and(|t| {
                ctx.world
                    .pawns
                    .get(*w)
                    .is_none_or(|p| p.position.tile.manhattan(t) > MEET_RADIUS)
            })
        })
        .map(|w| w.to_string())
        .collect();
    if absent.is_empty() {
        Some((
            CommitState::Completed,
            ReasonCode::simple("commitment_completed"),
        ))
    } else {
        fail(format!(
            "{} was not at the meeting place",
            absent.join(" and ")
        ))
    }
}

#[cfg(test)]
mod tests;
