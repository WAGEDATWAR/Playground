//! Friends seek each other out (Roadmap Stage 2; suggestion S-055, decision D-089).
//!
//! At each slot boundary a resident with an open stretch of the day may ask a friend to meet at a gathering
//! place. The ask is an ordinary commitment proposal (Blueprint §8.6): the invitee answers at the next
//! boundary, both schedules reserve the time only if both can, and the outcome (2.1) leaves memories and
//! changes the pair's affinity. Nothing here decides the answer or the outcome.
//!
//! Everything that could cause a flood is bounded and comes from data (`meetups` in `relationships.json`):
//! a resident has at most one live commitment, a boundary makes at most `max_per_boundary` proposals, a
//! pair is not asked again within `cooldown_minutes` whatever happened, and finished commitments are
//! removed after `keep_days` so the table stays small. The ask is drawn from the `social.proposal` stream,
//! more often for outgoing residents. Candidates are the friends the resident has not talked to for longest.

use crate::canon::{Canon, ToCanon};
use crate::commitment::{CommitState, Commitment};
use crate::id::{EntityId, Kind};
use crate::map::Tile;
use crate::pipeline::TickCtx;
use crate::rng::{Key, Rng, Stream};
use crate::time::TICKS_PER_DAY;
use pg_content::gamedata::{GameData, MeetupParams};
use pg_content::ActionId;
use std::collections::BTreeSet;

/// What a new proposal asks for.
#[derive(Copy, Clone, Debug)]
pub struct NewCommitment {
    pub proposer: EntityId,
    pub invitee: EntityId,
    pub start: u32,
    pub len: u32,
    pub at: Tile,
    pub expires_in: u32,
    pub reschedulable: bool,
}

/// Creates a proposed commitment between two pawns, or says why not. Shared with the developer command so
/// there is one place that builds the row.
pub fn new_commitment(
    world: &mut crate::world::WorldState,
    tick: u64,
    spec: &NewCommitment,
) -> Result<EntityId, String> {
    let NewCommitment {
        proposer,
        invitee,
        start,
        len,
        at,
        expires_in,
        reschedulable,
    } = *spec;
    if proposer == invitee {
        return Err("a pawn cannot meet itself".into());
    }
    let Some(map) = world.pawns.get(proposer).map(|p| p.position.map) else {
        return Err(format!("no pawn {proposer}"));
    };
    if !world.pawns.contains(invitee) {
        return Err(format!("no pawn {invitee}"));
    }
    let slots = world.settings.slot_minutes.slots_per_day();
    if len == 0 || start.checked_add(len).is_none_or(|end| end > slots) {
        return Err(format!(
            "slots {start}+{len} do not fit in a day of {slots}"
        ));
    }
    if !world.is_passable(map, at) {
        return Err(format!("tile {at} cannot be stood on"));
    }
    let action =
        ActionId::new("meet_at").map_err(|_| "the meet_at action id is invalid".to_owned())?;
    let id = world
        .id_counters
        .allocate(Kind::Commitment)
        .map_err(|e| e.to_string())?;
    let commitment = Commitment {
        id,
        proposer,
        invitee,
        day: world.clock.day(),
        start,
        len,
        action,
        params: Canon::map([("at", at.to_canon())]),
        state: CommitState::Proposed,
        created_tick: tick,
        expires_tick: tick.saturating_add(u64::from(expires_in)),
        reschedulable,
        reason: None,
    };
    world
        .commitments
        .insert(id, commitment)
        .map_err(|_| "duplicate commitment id".to_owned())?;
    Ok(id)
}

/// One slot boundary of asking. Called by the commitment system when game data with `meetups` is present.
pub fn run(ctx: &mut TickCtx<'_>, data: &GameData) {
    let Some(params) = data.relationships.as_ref().map(|r| &r.meetups) else {
        return;
    };
    if ctx.flags.new_day {
        prune(ctx, params);
    }
    let slots = ctx.world.settings.slot_minutes.slots_per_day();
    let slot_minutes = ctx.world.settings.slot_minutes.get().max(1);
    let now = ctx.flags.slot_of_day;
    let first = now.saturating_add(params.lead_slots);
    let last_end = (params.latest_end_minutes / slot_minutes).min(slots);
    if first.saturating_add(params.slots) > last_end {
        return;
    }
    let tick = ctx.flags.tick;
    let cooldown_ticks = u64::from(params.cooldown_minutes) * crate::time::TICKS_PER_GAME_MINUTE;
    // Who is already tied up, and which pairs asked recently (whatever the answer was).
    let mut busy: BTreeSet<EntityId> = BTreeSet::new();
    let mut recent: BTreeSet<(EntityId, EntityId)> = BTreeSet::new();
    for (_, c) in ctx.world.commitments.iter() {
        if !c.state.is_terminal() {
            busy.extend(c.parties());
        }
        if c.created_tick.saturating_add(cooldown_ticks) > tick {
            recent.insert(pair(c.proposer, c.invitee));
        }
    }
    let ids: Vec<EntityId> = ctx.world.pawns.iter().map(|(id, _)| id).collect();
    let mut made = 0u32;
    for pawn in ids {
        if made >= params.max_per_boundary {
            break;
        }
        if busy.contains(&pawn) {
            continue;
        }
        let Some(p) = ctx.world.pawns.get(pawn) else {
            continue;
        };
        if !p.capacities.can_act() || !has_plan_today(ctx, pawn, slots) {
            continue;
        }
        // The urge: more often for outgoing residents, less for reserved ones.
        let chance = (i64::from(params.chance_permille) * (1000 + i64::from(p.outgoing) / 2)
            / 1000)
            .clamp(0, 1000);
        let urge = Rng::new(
            ctx.world.seed(),
            Stream::SocialProposal,
            &[Key::Id(pawn), Key::Int(i64::try_from(tick).unwrap_or(0))],
        )
        .range(0, 1000)
        .unwrap_or(1000);
        if i64::from(urge) >= chance {
            continue;
        }
        // Friends the resident has not talked to for longest come first; ties go to the lower id.
        let mut friends: Vec<(u64, EntityId)> = ctx
            .world
            .relationships
            .of(pawn)
            .filter(|r| r.affinity >= params.min_affinity)
            .filter_map(|r| Some((r.last_interaction_tick, r.key.other(pawn)?)))
            .collect();
        friends.sort();
        for (_, other) in friends {
            if busy.contains(&other)
                || recent.contains(&pair(pawn, other))
                || !ctx
                    .world
                    .pawns
                    .get(other)
                    .is_some_and(|o| o.capacities.can_act())
                || !has_plan_today(ctx, other, slots)
            {
                continue;
            }
            let Some(start) = common_window(ctx, pawn, other, first, last_end, params.slots) else {
                continue;
            };
            let Some(at) = meeting_place(ctx, pawn, other) else {
                continue;
            };
            let expires = u32::try_from(
                u64::from(params.expires_slots)
                    * u64::from(slot_minutes)
                    * crate::time::TICKS_PER_GAME_MINUTE,
            )
            .unwrap_or(u32::MAX);
            let spec = NewCommitment {
                proposer: pawn,
                invitee: other,
                start,
                len: params.slots,
                at,
                expires_in: expires,
                reschedulable: false,
            };
            if let Ok(id) = new_commitment(ctx.world, tick, &spec) {
                ctx.emit(
                    "commitment.proposed",
                    Canon::map([
                        ("commitment", id.to_canon()),
                        ("proposer", pawn.to_canon()),
                        ("invitee", other.to_canon()),
                        ("start", start.to_canon()),
                        ("len", params.slots.to_canon()),
                    ]),
                );
                busy.insert(pawn);
                busy.insert(other);
                recent.insert(pair(pawn, other));
                ctx.mark_changed(pawn);
                ctx.mark_changed(other);
                made += 1;
                break;
            }
        }
    }
}

fn pair(a: EntityId, b: EntityId) -> (EntityId, EntityId) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn has_plan_today(ctx: &TickCtx<'_>, pawn: EntityId, slots: u32) -> bool {
    ctx.world
        .pawns
        .get(pawn)
        .and_then(|p| p.schedule.as_ref())
        .is_some_and(|s| s.day == ctx.flags.day && s.slots_per_day() == slots)
}

/// The earliest start in `first..=last_end - len` at which both pawns have `len` free slots.
fn common_window(
    ctx: &TickCtx<'_>,
    a: EntityId,
    b: EntityId,
    first: u32,
    last_end: u32,
    len: u32,
) -> Option<u32> {
    let sched = |id: EntityId| ctx.world.pawns.get(id).and_then(|p| p.schedule.as_ref());
    let (sa, sb) = (sched(a)?, sched(b)?);
    let latest_start = last_end.checked_sub(len)?;
    (first..=latest_start).find(|s| sa.is_free_run(*s, len) && sb.is_free_run(*s, len))
}

/// The gathering place that is shortest for the two of them together (ties to the lower y, then x).
fn meeting_place(ctx: &TickCtx<'_>, a: EntityId, b: EntityId) -> Option<Tile> {
    let (pa, pb) = (ctx.world.pawns.get(a)?, ctx.world.pawns.get(b)?);
    if pa.position.map != pb.position.map {
        return None;
    }
    let map = pa.position.map;
    ctx.world
        .town
        .gathering
        .iter()
        .copied()
        .filter(|t| ctx.world.is_passable(map, *t))
        .min_by_key(|t| {
            (
                t.manhattan(pa.position.tile) + t.manhattan(pb.position.tile),
                t.y,
                t.x,
            )
        })
        .or_else(|| crate::life::plaza(ctx.world, map))
}

/// Removes finished commitments older than `keep_days` so the table stays bounded.
fn prune(ctx: &mut TickCtx<'_>, params: &MeetupParams) {
    let keep = u64::from(params.keep_days).saturating_mul(TICKS_PER_DAY);
    let now = ctx.flags.tick;
    let old: Vec<EntityId> = ctx
        .world
        .commitments
        .iter()
        .filter(|(_, c)| c.state.is_terminal() && c.created_tick.saturating_add(keep) <= now)
        .map(|(id, _)| id)
        .collect();
    for id in old {
        ctx.world.commitments.remove(id);
    }
}

#[cfg(test)]
mod tests;
