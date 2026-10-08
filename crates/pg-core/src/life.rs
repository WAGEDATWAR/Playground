//! Needs, mood and what they ask of the day (Stage 1, milestone 1.1; Blueprint §8.1, §8.2).
//!
//! * [`NeedsSystem`] (every game minute): needs decay (faster while walking), are restored by the action a
//!   pawn is performing, and raise events and a replan when they become urgent or critical. From the needs it
//!   derives the pawn's [`Capacities`](crate::capacity::Capacities); a pawn whose consciousness falls below the
//!   action threshold collapses where it stands and recovers slowly.
//! * [`MoodSystem`] (every game minute): evaluates the ordered mood rule table from the game data.
//! * [`LifePlanSource`]: the plan source that asks for a restoring activity when a need is predicted to run low.
//!
//! All rates come from the game data (`data/game/needs.json`, `mood.json`). Everything is integer arithmetic
//! on the global minute counter, so a rate of 55 points an hour loses 55 points over any 60 minutes with no
//! remainder to carry or save.

use crate::action::{ActionRegistry, COMPANY_RADIUS};
use crate::activity::PlanSource;
use crate::canon::{Canon, ToCanon};
use crate::capacity::Capacities;
use crate::dev::DevPlanSource;
use crate::id::EntityId;
use crate::map::Tile;
use crate::pawn::{Intent, Replan, Step};
use crate::pipeline::{Pipeline, System, SystemSlot, TickCtx};
use crate::schedule::{DutyTemplate, LeisureOption, PlanInputs, UrgentNeed};
use crate::time::TICKS_PER_GAME_MINUTE;
use crate::world::WorldState;
use pg_content::gamedata::{GameData, MoodCond, PlaceKind, Tone};
use pg_content::ActionId;
use std::sync::Arc;

/// Points gained or lost during game minute `minute` at `per_hour` points an hour: the difference of two
/// whole-minute totals, so any sixty consecutive minutes add up to exactly `per_hour`.
pub fn per_minute(minute: u64, per_hour: u32) -> i32 {
    let rate = u64::from(per_hour);
    let upto = |m: u64| m.saturating_mul(rate) / 60;
    i32::try_from(upto(minute.saturating_add(1)).saturating_sub(upto(minute))).unwrap_or(i32::MAX)
}

/// The decay rate (points an hour) of a need for a pawn that is walking or standing.
pub fn decay_rate(decay_per_hour: u32, active_permille: u32, walking: bool) -> u32 {
    if walking {
        u32::try_from(u64::from(decay_per_hour) * u64::from(active_permille) / 1000)
            .unwrap_or(u32::MAX)
    } else {
        decay_per_hour
    }
}

fn pawn_ids(world: &WorldState) -> Vec<EntityId> {
    world.pawns.iter().map(|(id, _)| id).collect()
}

// ---- needs -----------------------------------------------------------------------------------------------

pub struct NeedsSystem {
    data: Arc<GameData>,
    actions: Arc<ActionRegistry>,
}

impl NeedsSystem {
    pub fn new(data: Arc<GameData>, actions: Arc<ActionRegistry>) -> NeedsSystem {
        NeedsSystem { data, actions }
    }
}

/// What happened to one pawn this minute, applied after the pawn's row is updated.
#[derive(Default)]
struct Outcome {
    urgent: Vec<(String, i32)>,
    critical: Vec<(String, i32)>,
    collapsed: bool,
    recovered: bool,
}

impl System for NeedsSystem {
    fn id(&self) -> &str {
        "NeedsSystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let minute = ctx.flags.tick / TICKS_PER_GAME_MINUTE;
        let slot = ctx.flags.slot_of_day;
        let others: Vec<(EntityId, EntityId, Tile)> = ctx
            .world
            .pawns
            .iter()
            .map(|(id, p)| (id, p.position.map, p.position.tile))
            .collect();
        for id in pawn_ids(ctx.world) {
            // Packs may make a pawn's needs fall slower or faster (bounded; 1000 is normal).
            let decay_permille = crate::hooks::resolved(
                &mut ctx.services.hooks,
                crate::hooks::need_decay(),
                &*ctx.world,
                id,
            )
            .unwrap_or(1000);
            let Some(p) = ctx.world.pawns.get_mut(id) else {
                continue;
            };
            let mut out = Outcome::default();
            for (need, def) in &self.data.needs {
                p.needs.entry(need.clone()).or_insert(def.start);
            }
            if p.home_tile.is_none() {
                p.home_tile = Some(p.position.tile);
            }
            let was_able = p.capacities.can_act();
            let walking = p.route.is_some();
            let restoring = p
                .task
                .as_ref()
                .filter(|t| matches!(t.step(), Some(Step::PerformUntil(_))))
                .and_then(|t| self.actions.get(&t.action))
                .map(|d| d.restores.clone())
                .unwrap_or_default();
            let company = restoring.iter().any(|r| r.company)
                && others.iter().any(|(other, map, tile)| {
                    *other != id
                        && *map == p.position.map
                        && tile.manhattan(p.position.tile) <= COMPANY_RADIUS
                });
            for (need, def) in &self.data.needs {
                let old = p.needs.get(need).copied().unwrap_or(def.start);
                let mut v = old;
                let rate = decay_rate(def.decay_per_hour, def.active_permille, walking);
                let rate = u32::try_from(
                    u64::from(rate) * u64::try_from(decay_permille).unwrap_or(1000) / 1000,
                )
                .unwrap_or(rate);
                v -= per_minute(minute, rate);
                if !was_able {
                    v += per_minute(minute, def.rest_restore_per_hour);
                }
                for r in restoring.iter().filter(|r| &r.need == need) {
                    if !r.company || company {
                        v += per_minute(minute, r.per_hour);
                    }
                }
                let v = v.clamp(0, 1000);
                p.needs.insert(need.clone(), v);
                if old >= def.critical_below && v < def.critical_below {
                    out.critical.push((need.clone(), v));
                } else if old >= def.urgent_below && v < def.urgent_below {
                    out.urgent.push((need.clone(), v));
                }
            }

            // Capacities from needs. A need that was critical stays critical until it climbs back to urgent,
            // so a pawn does not flicker between collapsed and walking at the threshold.
            let prev = p.capacities;
            let mut caps = Capacities::FULL;
            for (need, def) in &self.data.needs {
                let level = p.needs.get(need).copied().unwrap_or(def.start);
                for pen in &def.penalties {
                    let was_critical = prev.get(&pen.capacity).is_some_and(|c| c <= pen.critical)
                        && pen.critical < pen.urgent;
                    if level < def.critical_below || (was_critical && level < def.urgent_below) {
                        caps.cap(&pen.capacity, pen.critical);
                    } else if level < def.urgent_below {
                        caps.cap(&pen.capacity, pen.urgent);
                    }
                }
            }
            p.capacities = caps;
            if was_able && !caps.can_act() {
                out.collapsed = true;
                p.task = None;
                p.route = None;
                p.intent = Intent::Free;
                p.replan = None;
            } else if !was_able && caps.can_act() {
                out.recovered = true;
            }
            let wants_replan = !out.urgent.is_empty() || !out.critical.is_empty() || out.recovered;
            if wants_replan && p.replan.is_none() {
                let why = match (out.critical.first(), out.urgent.first(), out.recovered) {
                    (Some((n, _)), _, _) => format!("{n} became critical"),
                    (None, Some((n, _)), _) => format!("{n} became urgent"),
                    _ => "the pawn recovered".to_owned(),
                };
                p.replan = Some(Replan {
                    from: slot.saturating_add(1),
                    why,
                });
            }
            let changed = wants_replan || out.collapsed;
            for (need, value) in out.urgent {
                ctx.emit(
                    "need.urgent",
                    Canon::map([
                        ("pawn", id.to_canon()),
                        ("need", Canon::str(need)),
                        ("value", value.to_canon()),
                    ]),
                );
            }
            for (need, value) in out.critical {
                ctx.emit(
                    "need.critical",
                    Canon::map([
                        ("pawn", id.to_canon()),
                        ("need", Canon::str(need)),
                        ("value", value.to_canon()),
                    ]),
                );
            }
            if out.collapsed {
                ctx.emit("pawn.collapsed", Canon::map([("pawn", id.to_canon())]));
            }
            if out.recovered {
                ctx.emit("pawn.recovered", Canon::map([("pawn", id.to_canon())]));
            }
            if changed {
                ctx.mark_changed(id);
            }
        }
    }
}

// ---- mood ------------------------------------------------------------------------------------------------

pub struct MoodSystem {
    data: Arc<GameData>,
}

impl MoodSystem {
    pub fn new(data: Arc<GameData>) -> MoodSystem {
        MoodSystem { data }
    }
}

/// Whether `cond` holds for the pawn at `tick`.
fn holds(cond: &MoodCond, p: &crate::pawn::Pawn, data: &GameData, tick: u64, shift: i32) -> bool {
    match cond {
        MoodCond::NeedCritical { need } => match (data.needs.get(need), p.needs.get(need)) {
            (Some(def), Some(v)) => v.saturating_add(shift) < def.critical_below,
            _ => false,
        },
        MoodCond::AnyNeedUrgent => data.needs.iter().any(|(n, def)| {
            p.needs
                .get(n)
                .is_some_and(|v| v.saturating_add(shift) < def.urgent_below)
        }),
        MoodCond::AllNeedsAbove { value } => {
            !data.needs.is_empty()
                && data.needs.keys().all(|n| {
                    p.needs
                        .get(n)
                        .is_some_and(|v| v.saturating_add(shift) > *value)
                })
        }
        MoodCond::Memory {
            tone,
            within_hours,
            min_importance,
        } => {
            let window = u64::from(*within_hours) * 60 * TICKS_PER_GAME_MINUTE;
            p.memories.iter().any(|m| {
                // Memories from before the world began (tick 0) do not move anyone's mood.
                m.tick > 0
                    && tick.saturating_sub(m.tick) <= window
                    && m.importance >= *min_importance
                    && match tone {
                        Tone::Positive => m.is_positive(),
                        Tone::Negative => m.is_negative(),
                    }
            })
        }
        MoodCond::IdleHours { hours } => p.idle_since.is_some_and(|since| {
            tick.saturating_sub(since) >= u64::from(*hours) * 60 * TICKS_PER_GAME_MINUTE
        }),
    }
}

impl System for MoodSystem {
    fn id(&self) -> &str {
        "MoodSystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        let tick = ctx.flags.tick;
        let rules = self.data.ordered_rules();
        for id in pawn_ids(ctx.world) {
            // Packs may nudge how well off a pawn feels (bounded).
            let shift = crate::hooks::resolved(
                &mut ctx.services.hooks,
                crate::hooks::mood_comfort(),
                &*ctx.world,
                id,
            )
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(0);
            let Some(p) = ctx.world.pawns.get_mut(id) else {
                continue;
            };
            // Idle means nothing is planned and nothing is being done.
            let idle = p.task.is_none() && p.intent == Intent::Free;
            match (idle, p.idle_since) {
                (true, None) => p.idle_since = Some(tick),
                (false, Some(_)) => p.idle_since = None,
                _ => {}
            }
            let decided = rules
                .iter()
                .find(|r| r.when.iter().all(|c| holds(c, p, &self.data, tick, shift)));
            let Some(rule) = decided else { continue };
            if p.mood != rule.mood {
                let from = std::mem::replace(&mut p.mood, rule.mood.clone());
                ctx.emit(
                    "mood.changed",
                    Canon::map([
                        ("pawn", id.to_canon()),
                        ("from", Canon::str(from)),
                        ("to", Canon::str(rule.mood.clone())),
                        ("rule", Canon::str(rule.id.clone())),
                    ]),
                );
                ctx.mark_changed(id);
            }
        }
    }
}

// ---- places and plans ------------------------------------------------------------------------------------

/// The tile a kind of place resolves to for a pawn today. Until town generation provides real homes,
/// workplaces and gathering places (milestone 1.2), homes are where the pawn started, workplaces are the
/// scaffolding's seeded tile and the gathering place is the middle of the map.
pub fn resolve_place(
    world: &WorldState,
    pawn: EntityId,
    kind: PlaceKind,
    day: u64,
) -> Option<Tile> {
    let p = world.pawns.get(pawn)?;
    match kind {
        PlaceKind::Home => Some(p.home_tile.unwrap_or(p.position.tile)),
        PlaceKind::Workplace => p
            .workplace
            .or_else(|| DevPlanSource::tile(world, pawn, -1, 0)),
        PlaceKind::Gathering => world
            .town
            .gathering
            .iter()
            .copied()
            .min_by_key(|t| (t.manhattan(p.position.tile), t.y, t.x))
            .or_else(|| plaza(world, p.position.map)),
        PlaceKind::Anywhere => {
            DevPlanSource::tile(world, pawn, i64::try_from(day).unwrap_or(i64::MAX), 5)
        }
    }
}

/// The tile nearest the middle of the map that has open ground around it (at least twelve of the thirteen
/// tiles within two steps are passable), so a crowd can gather there; failing that, the nearest passable
/// tile. Ties go to the lower y, then the lower x.
pub fn plaza(world: &WorldState, map: EntityId) -> Option<Tile> {
    let m = world.maps.get(map)?;
    let (cx, cy) = (m.width() / 2, m.height() / 2);
    let reach = m.width().max(m.height());
    let open = |t: Tile| -> usize {
        let mut n = 0;
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx.abs() + dy.abs() <= 2 && world.is_passable(map, Tile::new(t.x + dx, t.y + dy))
                {
                    n += 1;
                }
            }
        }
        n
    };
    let mut nearest_passable: Option<Tile> = None;
    for r in 0..=reach {
        let mut best: Option<(i32, i32)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() + dy.abs() != r {
                    continue;
                }
                let t = Tile::new(cx + dx, cy + dy);
                if !world.is_passable(map, t) {
                    continue;
                }
                if nearest_passable.is_none() {
                    nearest_passable = Some(t);
                }
                if open(t) >= 12 {
                    let key = (t.y, t.x);
                    if best.is_none_or(|b| key < b) {
                        best = Some(key);
                    }
                }
            }
        }
        if let Some((y, x)) = best {
            return Some(Tile::new(x, y));
        }
    }
    nearest_passable
}

/// The day an occupation template asks for (milestone 1.3): its duties at the pawn's workplace, shifted by
/// the pawn's own habit, and its leisure options at places that resolve for this pawn.
///
/// Each duty's start is moved by a **habit** drawn once from the pawn's occupation `variation` (an early
/// bird starts at the early end of the template's allowed shift, a late riser at the other end), and the
/// planner adds its usual small seeded variation per day on top, so two baristas keep different routines
/// that still drift a little from day to day.
pub fn occupation_inputs(
    world: &WorldState,
    pawn: EntityId,
    day: u64,
    variation: i32,
    def: &pg_content::gamedata::OccupationDef,
) -> PlanInputs {
    let slot_minutes = world.settings.slot_minutes.get().max(1);
    let slots = world.settings.slot_minutes.slots_per_day();
    let to_slots = |minutes: u32| minutes.div_ceil(slot_minutes).max(1);
    let mut inputs = PlanInputs {
        open_weight: def.open_weight,
        ..PlanInputs::default()
    };
    let tile_params = |at: Tile| Canon::map([("at", at.to_canon())]);
    for duty in &def.duties {
        let (Ok(action), Some(at)) = (
            ActionId::new(&duty.activity),
            resolve_place(world, pawn, duty.place, day),
        ) else {
            continue;
        };
        let (earlier, later) = (
            duty.shift_earlier / slot_minutes,
            duty.shift_later / slot_minutes,
        );
        // The habit: somewhere in -earlier..=later, the same every day.
        let span = earlier + later + 1;
        let habit = i64::from(u32::try_from(variation.clamp(0, 999)).unwrap_or(0) * span / 1000)
            - i64::from(earlier);
        let start = i64::from(duty.start / slot_minutes) + habit;
        let len = to_slots(duty.minutes);
        inputs.duties.push(DutyTemplate {
            action,
            params: tile_params(at),
            start: u32::try_from(start.clamp(0, i64::from(slots.saturating_sub(len)))).unwrap_or(0),
            len,
            min_len: to_slots(duty.min_minutes).min(len),
            shift_earlier: earlier,
            shift_later: later,
        });
    }
    for (i, l) in def.leisure.iter().enumerate() {
        let salt = i64::try_from(i).unwrap_or(0) + 11;
        let at = match l.place {
            PlaceKind::Anywhere => {
                DevPlanSource::tile(world, pawn, i64::try_from(day).unwrap_or(i64::MAX), salt)
            }
            other => resolve_place(world, pawn, other, day),
        };
        let (Ok(action), Some(at)) = (ActionId::new(&l.activity), at) else {
            continue;
        };
        // `move_to` takes a destination named `to`; the others stay where they are named `at`.
        let params = if l.activity == "move_to" {
            Canon::map([("to", at.to_canon())])
        } else {
            tile_params(at)
        };
        inputs.leisure.push(LeisureOption {
            action,
            params,
            len: to_slots(l.minutes),
            weight: l.weight,
        });
    }
    inputs
}

/// The scaffolding plan (duties and leisure) plus a restoring activity for each need that is running low.
pub struct LifePlanSource {
    data: Arc<GameData>,
    inner: DevPlanSource,
}

impl LifePlanSource {
    pub fn new(data: Arc<GameData>) -> LifePlanSource {
        LifePlanSource {
            data,
            inner: DevPlanSource,
        }
    }
}

impl PlanSource for LifePlanSource {
    fn id(&self) -> &str {
        "life.plan"
    }

    fn inputs(&self, world: &WorldState, pawn: EntityId, day: u64) -> PlanInputs {
        let Some(p) = world.pawns.get(pawn) else {
            return PlanInputs::default();
        };
        // A resident with an occupation follows its template; a pawn without one (the developer tools'
        // anonymous pawns) keeps the scaffolding plan.
        let mut inputs = match p
            .occupation
            .as_ref()
            .and_then(|o| self.data.occupations.get(&o.template).map(|def| (o, def)))
        {
            Some((occ, def)) => occupation_inputs(world, pawn, day, occ.variation, def),
            None => self.inner.inputs(world, pawn, day),
        };
        let slot_minutes = world.settings.slot_minutes.get();
        let slots = world.settings.slot_minutes.slots_per_day();
        // Planning for today starts from where the clock is; any other day starts at midnight.
        let now_minute = if day == world.clock.day() {
            world.clock.minute_of_day()
        } else {
            0
        };
        let now_slot = now_minute / slot_minutes;
        for (need, def) in &self.data.needs {
            let (Some(restore), Some(level)) = (&def.restore, p.needs.get(need)) else {
                continue;
            };
            let (Ok(action), Some(at)) = (
                ActionId::new(&restore.action),
                resolve_place(world, pawn, restore.place, day),
            ) else {
                continue;
            };
            let params = Canon::map([("at", at.to_canon())]);
            let want = |earliest: u32, last_start: u32, minutes: u32| UrgentNeed {
                need: need.clone(),
                predicted_slot: last_start,
                earliest,
                rank: def.priority,
                action: action.clone(),
                params: params.clone(),
                len: minutes.div_ceil(slot_minutes).max(1),
            };
            // A need that is already urgent is looked after as soon as there is room.
            if *level < def.urgent_below {
                inputs
                    .urgent
                    .push(want(now_slot, now_slot, restore.minutes));
            }
            // The day's regular meals, bedtimes and the like: each may start anywhere in its window.
            for r in &restore.routine {
                let (first, last) = (r.start / slot_minutes, r.end / slot_minutes);
                if last < now_slot || last >= slots {
                    continue;
                }
                inputs.urgent.push(want(first, last + 1, r.minutes));
            }
        }
        inputs
    }
}

/// Installs the needs and mood systems and the plan source that knows about needs, replacing the
/// scaffolding probe. Called when content with game data is loaded.
pub fn install(pipeline: &mut Pipeline, data: Arc<GameData>) {
    let actions = Arc::new(ActionRegistry::builtin());
    let plans: Arc<dyn PlanSource> = Arc::new(LifePlanSource::new(Arc::clone(&data)));
    pipeline.set_builtin(
        SystemSlot::Needs,
        Box::new(NeedsSystem::new(Arc::clone(&data), actions)),
    );
    pipeline.set_builtin(
        SystemSlot::Mood,
        Box::new(MoodSystem::new(Arc::clone(&data))),
    );
    crate::conversation::install(pipeline, data);
    pipeline.set_builtin(
        SystemSlot::DayPlanner,
        Box::new(crate::activity::DayPlannerSystem::new(Arc::clone(&plans))),
    );
    pipeline.set_builtin(
        SystemSlot::ReservationActivator,
        Box::new(crate::activity::ReservationActivatorSystem::new(plans)),
    );
}

#[cfg(test)]
mod tests;
