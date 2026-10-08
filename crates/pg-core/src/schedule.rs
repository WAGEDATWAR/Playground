//! Day schedules, reservations and the deterministic day planner (Blueprint §8.5).
//!
//! A [`DaySchedule`] is an array of slots, each free or owned by one [`Reservation`]. [`plan_day`] fills it
//! in five priority passes (duties, urgent needs, commitments, chores, leisure); [`replan_from`] re-runs the
//! passes for the rest of the day without touching slots that have started; [`insert_urgent`] squeezes a
//! newly critical need in mid-day, displacing lower-priority reservations. Every placement and every drop
//! stores a [`ReasonCode`], so `pg schedule explain` can say *why* a pawn's day looks the way it does.
//!
//! The planner is pure: it reads a [`PlanInputs`] value and a seed, and writes a schedule. Needs,
//! occupations and chores arrive in Stage 1; until then a `PlanSource` (see `activity`) supplies inputs.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::read::{ReadError, Reader};
use crate::reason::ReasonCode;
use crate::rng::{Key, Rng, Seed, Stream};
use pg_content::ActionId;
use std::collections::BTreeMap;

/// Reservation priority: a lower number is stronger and never yields to a higher number.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Priority {
    Duty = 1,
    UrgentNeed = 2,
    Commitment = 3,
    Chore = 4,
    Leisure = 5,
}

impl Priority {
    pub const fn number(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            Priority::Duty => "duty",
            Priority::UrgentNeed => "urgent_need",
            Priority::Commitment => "commitment",
            Priority::Chore => "chore",
            Priority::Leisure => "leisure",
        }
    }

    /// Priorities 4 and 5 may be displaced by an urgent need.
    pub const fn displaceable(self) -> bool {
        matches!(self, Priority::Chore | Priority::Leisure)
    }
}

/// A block of consecutive slots committed to one action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    /// Unique within its schedule, ascending in creation order. It is the final tie-break everywhere.
    pub id: u32,
    pub priority: Priority,
    pub action: ActionId,
    pub params: Canon,
    pub start: u32,
    pub len: u32,
    pub urgency: u32,
    pub created_tick: u64,
    pub reschedulable: bool,
    /// The commitment this reservation realises, if any.
    pub commitment: Option<EntityId>,
    /// Why it is here.
    pub reason: ReasonCode,
}

impl Reservation {
    /// One past the last slot.
    pub fn end(&self) -> u32 {
        self.start.saturating_add(self.len)
    }
}

impl ToCanon for Reservation {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("priority", Canon::Int(i128::from(self.priority.number()))),
            ("action", self.action.to_canon()),
            ("params", self.params.clone()),
            ("start", self.start.to_canon()),
            ("len", self.len.to_canon()),
            ("urgency", self.urgency.to_canon()),
            ("created_tick", self.created_tick.to_canon()),
            ("reschedulable", self.reschedulable.to_canon()),
            (
                "commitment",
                self.commitment.map_or(Canon::Null, |c| c.to_canon()),
            ),
            ("reason", self.reason.to_canon()),
        ])
    }
}

/// Something the planner wanted to place and could not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    pub what: String,
    pub reason: ReasonCode,
}

impl ToCanon for Dropped {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("what", Canon::str(self.what.clone())),
            ("reason", self.reason.to_canon()),
        ])
    }
}

/// Why a placement was refused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlaceError {
    /// Zero length, or it runs past the end of the day.
    OutOfRange,
    /// Some slot in the range is already owned.
    Occupied,
}

/// One pawn's plan for one day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaySchedule {
    pub day: u64,
    slots: Vec<Option<u32>>,
    reservations: BTreeMap<u32, Reservation>,
    next_id: u32,
    /// Planning decisions that are not tied to one reservation (slots left open, replans).
    pub notes: Vec<ReasonCode>,
    pub dropped: Vec<Dropped>,
}

impl DaySchedule {
    pub fn new(day: u64, slots_per_day: u32) -> DaySchedule {
        DaySchedule {
            day,
            slots: vec![None; slots_per_day as usize],
            reservations: BTreeMap::new(),
            next_id: 1,
            notes: Vec::new(),
            dropped: Vec::new(),
        }
    }

    pub fn slots_per_day(&self) -> u32 {
        u32::try_from(self.slots.len()).unwrap_or(u32::MAX)
    }

    pub fn reservation(&self, id: u32) -> Option<&Reservation> {
        self.reservations.get(&id)
    }

    /// Reservations in id order.
    pub fn reservations(&self) -> impl Iterator<Item = &Reservation> {
        self.reservations.values()
    }

    /// Reservations in slot order.
    pub fn by_start(&self) -> Vec<&Reservation> {
        let mut v: Vec<&Reservation> = self.reservations.values().collect();
        v.sort_by_key(|r| (r.start, r.id));
        v
    }

    pub fn owner_id(&self, slot: u32) -> Option<u32> {
        self.slots.get(slot as usize).copied().flatten()
    }

    /// The reservation covering `slot`.
    pub fn owner(&self, slot: u32) -> Option<&Reservation> {
        self.owner_id(slot)
            .and_then(|id| self.reservations.get(&id))
    }

    pub fn is_free_run(&self, start: u32, len: u32) -> bool {
        let Some(end) = start.checked_add(len) else {
            return false;
        };
        len > 0
            && self
                .slots
                .get(start as usize..end as usize)
                .is_some_and(|s| s.iter().all(Option::is_none))
    }

    pub fn free_slots(&self) -> u32 {
        u32::try_from(self.slots.iter().filter(|s| s.is_none()).count()).unwrap_or(u32::MAX)
    }

    /// Places a reservation (its `id` is assigned here) and returns the id.
    pub fn place(&mut self, mut r: Reservation) -> Result<u32, PlaceError> {
        let end = r.start.checked_add(r.len).ok_or(PlaceError::OutOfRange)?;
        if r.len == 0 || end > self.slots_per_day() {
            return Err(PlaceError::OutOfRange);
        }
        if !self.is_free_run(r.start, r.len) {
            return Err(PlaceError::Occupied);
        }
        // A reservation that is being put back (id already issued by this schedule) keeps its id, because
        // tasks and commitments refer to it; anything else gets a fresh one.
        let id = if r.id != 0 && r.id < self.next_id && !self.reservations.contains_key(&r.id) {
            r.id
        } else {
            let fresh = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            fresh
        };
        r.id = id;
        if let Some(range) = self.slots.get_mut(r.start as usize..end as usize) {
            for s in range {
                *s = Some(id);
            }
        }
        self.reservations.insert(id, r);
        Ok(id)
    }

    pub fn remove(&mut self, id: u32) -> Option<Reservation> {
        let r = self.reservations.remove(&id)?;
        for s in &mut self.slots {
            if *s == Some(id) {
                *s = None;
            }
        }
        Some(r)
    }

    /// The first start in `from..` where `len` free slots fit, with `start < start_before` when given and
    /// the run ending no later than `end_by` when given.
    pub fn first_fit(
        &self,
        from: u32,
        len: u32,
        start_before: Option<u32>,
        end_by: Option<u32>,
    ) -> Option<u32> {
        let last_start = self.slots_per_day().checked_sub(len)?;
        (from..=last_start)
            .take_while(|s| start_before.is_none_or(|b| *s < b))
            .take_while(|s| end_by.is_none_or(|e| s.saturating_add(len) <= e))
            .find(|s| self.is_free_run(*s, len))
    }

    /// The free run of `len` slots starting nearest to `want` (ties go to the earlier start), at or after
    /// `from`.
    pub fn nearest_fit(&self, from: u32, want: u32, len: u32) -> Option<u32> {
        let last_start = self.slots_per_day().checked_sub(len)?;
        (from..=last_start)
            .filter(|s| self.is_free_run(*s, len))
            .min_by_key(|s| (s.abs_diff(want), *s))
    }

    /// Structural checks used by tests and debug builds: no overlap, every slot points at a reservation
    /// that covers it, every reservation's slots point back, ids ascend.
    pub fn check_invariants(&self) -> Result<(), String> {
        let n = self.slots_per_day();
        for (slot, owner) in self.slots.iter().enumerate() {
            if let Some(id) = owner {
                let r = self
                    .reservations
                    .get(id)
                    .ok_or_else(|| format!("slot {slot} points at missing reservation {id}"))?;
                let slot = u32::try_from(slot).unwrap_or(u32::MAX);
                if slot < r.start || slot >= r.end() {
                    return Err(format!("slot {slot} is outside reservation {id}"));
                }
            }
        }
        for r in self.reservations.values() {
            if r.len == 0 || r.end() > n {
                return Err(format!("reservation {} has a bad range", r.id));
            }
            if r.id >= self.next_id {
                return Err(format!("reservation {} is not below next_id", r.id));
            }
            for s in r.start..r.end() {
                if self.owner_id(s) != Some(r.id) {
                    return Err(format!("reservation {} does not own slot {s}", r.id));
                }
            }
        }
        Ok(())
    }
}

impl ToCanon for DaySchedule {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("day", self.day.to_canon()),
            ("slots", self.slots_per_day().to_canon()),
            (
                "reservations",
                Canon::List(self.reservations.values().map(ToCanon::to_canon).collect()),
            ),
            ("next_id", self.next_id.to_canon()),
            (
                "notes",
                Canon::List(self.notes.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "dropped",
                Canon::List(self.dropped.iter().map(ToCanon::to_canon).collect()),
            ),
        ])
    }
}

// ---- inputs ----------------------------------------------------------------------------------------

/// A duty from an occupation template, with the bounds the seeded variation may use.
#[derive(Clone, Debug, PartialEq)]
pub struct DutyTemplate {
    pub action: ActionId,
    pub params: Canon,
    pub start: u32,
    pub len: u32,
    pub min_len: u32,
    pub shift_earlier: u32,
    pub shift_later: u32,
}

/// A need predicted to become urgent, with a restoring activity.
#[derive(Clone, Debug, PartialEq)]
pub struct UrgentNeed {
    pub need: String,
    /// The slot at which the need is predicted to reach "urgent" (or, for a routine activity, the last slot
    /// it may start in).
    pub predicted_slot: u32,
    /// The first slot the activity may start in (0 for no limit).
    pub earliest: u32,
    pub action: ActionId,
    pub params: Canon,
    pub len: u32,
}

/// An accepted commitment the day must honour.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitmentReq {
    pub id: EntityId,
    pub action: ActionId,
    pub params: Canon,
    pub start: u32,
    pub len: u32,
    pub reschedulable: bool,
    pub created_tick: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chore {
    pub id: u32,
    pub action: ActionId,
    pub params: Canon,
    pub len: u32,
    pub urgency: u32,
    pub deadline_slot: Option<u32>,
    pub created_tick: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeisureOption {
    pub action: ActionId,
    pub params: Canon,
    pub len: u32,
    pub weight: u32,
}

/// Everything the planner needs to know about a pawn's day.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanInputs {
    pub duties: Vec<DutyTemplate>,
    pub urgent: Vec<UrgentNeed>,
    pub commitments: Vec<CommitmentReq>,
    pub chores: Vec<Chore>,
    pub leisure: Vec<LeisureOption>,
    /// Weight of "leave this slot open" against the leisure options' weights.
    pub open_weight: u32,
}

// ---- planning --------------------------------------------------------------------------------------

fn n(v: u32) -> Canon {
    Canon::Int(i128::from(v))
}

fn blank(
    priority: Priority,
    action: &ActionId,
    params: &Canon,
    start: u32,
    len: u32,
    reason: ReasonCode,
) -> Reservation {
    Reservation {
        id: 0,
        priority,
        action: action.clone(),
        params: params.clone(),
        start,
        len,
        urgency: 0,
        created_tick: 0,
        reschedulable: false,
        commitment: None,
        reason,
    }
}

fn describe(r: &Reservation) -> String {
    format!("{} '{}' at slot {}", r.priority.name(), r.action, r.start)
}

/// Plans a whole day from scratch.
pub fn plan_day(
    seed: Seed,
    pawn: EntityId,
    day: u64,
    slots_per_day: u32,
    inputs: &PlanInputs,
) -> DaySchedule {
    plan_day_from(seed, pawn, day, slots_per_day, 0, inputs)
}

/// Plans a day whose first `from` slots are already behind the pawn (a pawn that appears mid-day).
pub fn plan_day_from(
    seed: Seed,
    pawn: EntityId,
    day: u64,
    slots_per_day: u32,
    from: u32,
    inputs: &PlanInputs,
) -> DaySchedule {
    let mut s = DaySchedule::new(day, slots_per_day);
    fill(&mut s, seed, pawn, from, inputs);
    s
}

/// Re-plans slots `from..` after a trigger (need critical, commitment change, failure, map edit).
/// Reservations that started before `from` are kept as they are; everything else is placed again.
pub fn replan_from(
    s: &mut DaySchedule,
    seed: Seed,
    pawn: EntityId,
    from: u32,
    inputs: &PlanInputs,
    why: &str,
) {
    let stale: Vec<u32> = s
        .reservations
        .values()
        .filter(|r| r.start >= from)
        .map(|r| r.id)
        .collect();
    for id in stale {
        s.remove(id);
    }
    s.dropped.clear();
    s.notes.clear();
    s.notes.push(ReasonCode::builtin(
        "replanned",
        [("from", n(from)), ("why", Canon::str(why))],
    ));
    fill(s, seed, pawn, from, inputs);
}

fn fill(s: &mut DaySchedule, seed: Seed, pawn: EntityId, from: u32, inputs: &PlanInputs) {
    place_duties(s, seed, pawn, from, inputs);
    place_urgent(s, from, inputs);
    place_commitments(s, from, inputs);
    place_chores(s, from, inputs);
    place_leisure(s, seed, pawn, from, inputs);
    let open = s.free_slots();
    if open > 0 {
        s.notes
            .push(ReasonCode::builtin("slots_open", [("count", n(open))]));
    }
}

/// Step 1: duties, with seeded shift and shortening inside the template's bounds.
fn place_duties(s: &mut DaySchedule, seed: Seed, pawn: EntityId, from: u32, inputs: &PlanInputs) {
    let day = i64::try_from(s.day).unwrap_or(i64::MAX);
    let mut order: Vec<(usize, &DutyTemplate)> = inputs.duties.iter().enumerate().collect();
    order.sort_by_key(|(i, d)| (d.start, *i));
    for (index, d) in order {
        let rng = Rng::new(
            seed,
            Stream::SchedVariation,
            &[
                Key::Id(pawn),
                Key::Int(day),
                Key::Int(i64::try_from(index).unwrap_or(0)),
            ],
        );
        let lo = -i32::try_from(d.shift_earlier).unwrap_or(0);
        let hi = i32::try_from(d.shift_later).unwrap_or(0);
        let shift = rng.int_in(0, lo, hi).unwrap_or(0);
        let max_shrink = d.len.saturating_sub(d.min_len.max(1));
        let shrink = rng
            .int_in(1, 0, i32::try_from(max_shrink).unwrap_or(0))
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let start = u32::try_from((i64::from(d.start) + i64::from(shift)).max(0)).unwrap_or(0);
        let len = d.len.saturating_sub(shrink).max(1);
        if start < from {
            continue; // already started: frozen
        }
        let reason = if shift != 0 || shrink != 0 {
            ReasonCode::builtin(
                "duty_varied",
                [
                    ("shift", Canon::Int(i128::from(shift))),
                    ("shrink", n(shrink)),
                    ("start", n(start)),
                    ("end", n(start.saturating_add(len))),
                ],
            )
        } else {
            ReasonCode::builtin("duty_placed", [("slot", n(start)), ("slots", n(len))])
        };
        let res = blank(Priority::Duty, &d.action, &d.params, start, len, reason);
        if s.place(res).is_err() {
            let by = s
                .owner(start)
                .map_or_else(|| "the end of the day".to_owned(), describe);
            s.dropped.push(Dropped {
                what: format!("duty '{}' at slot {start}", d.action),
                reason: ReasonCode::builtin(
                    "duty_dropped",
                    [("slot", n(start)), ("by", Canon::str(by))],
                ),
            });
        }
    }
}

/// Step 2: urgent needs, earliest free run before the predicted slot.
fn place_urgent(s: &mut DaySchedule, from: u32, inputs: &PlanInputs) {
    let mut order: Vec<&UrgentNeed> = inputs.urgent.iter().collect();
    order.sort_by(|a, b| (a.predicted_slot, &a.need).cmp(&(b.predicted_slot, &b.need)));
    for u in order {
        let _ = place_one_urgent(s, from, u);
    }
}

fn place_one_urgent(s: &mut DaySchedule, from: u32, u: &UrgentNeed) -> Option<u32> {
    // A need that is already urgent has no deadline: take the earliest run that exists.
    let before = (u.predicted_slot > from).then_some(u.predicted_slot);
    let found = s.first_fit(from.max(u.earliest), u.len, before, None);
    let Some(start) = found else {
        s.dropped.push(Dropped {
            what: format!("urgent {} activity", u.need),
            reason: ReasonCode::builtin(
                "urgent_need_unplaced",
                [
                    ("need", Canon::str(u.need.clone())),
                    ("predicted", n(u.predicted_slot)),
                ],
            ),
        });
        return None;
    };
    let reason = ReasonCode::builtin(
        "urgent_need_placed",
        [
            ("need", Canon::str(u.need.clone())),
            ("predicted", n(u.predicted_slot)),
            ("slot", n(start)),
        ],
    );
    s.place(blank(
        Priority::UrgentNeed,
        &u.action,
        &u.params,
        start,
        u.len,
        reason,
    ))
    .ok()
}

/// Step 3: accepted commitments at their agreed slots; a conflict with a duty or urgent need is resolved by
/// the commitment's `reschedulable` flag.
fn place_commitments(s: &mut DaySchedule, from: u32, inputs: &PlanInputs) {
    let mut order: Vec<&CommitmentReq> = inputs.commitments.iter().collect();
    order.sort_by_key(|c| (c.created_tick, c.id));
    for c in order {
        // Already kept as a started reservation.
        if s.reservations().any(|r| r.commitment == Some(c.id)) {
            continue;
        }
        let mk = |start: u32, reason: ReasonCode| commitment_reservation(c, start, reason);
        if c.start >= from {
            let reason = ReasonCode::builtin(
                "commitment_placed",
                [("slot", n(c.start)), ("slots", n(c.len))],
            );
            if s.place(mk(c.start, reason)).is_ok() {
                continue;
            }
        }
        let moved = c
            .reschedulable
            .then(|| s.nearest_fit(from, c.start, c.len))
            .flatten();
        match moved {
            Some(to) => {
                let reason =
                    ReasonCode::builtin("commitment_moved", [("from", n(c.start)), ("to", n(to))]);
                let _ = s.place(mk(to, reason));
            }
            None => {
                let why = if c.reschedulable {
                    "no free slot today"
                } else {
                    "a duty or urgent need holds the slot and it cannot move"
                };
                s.dropped.push(Dropped {
                    what: format!("commitment {} at slot {}", c.id, c.start),
                    reason: ReasonCode::builtin(
                        "commitment_failed",
                        [("slot", n(c.start)), ("why", Canon::str(why))],
                    ),
                });
            }
        }
    }
}

/// Step 4: chores by (urgency desc, earliest deadline, id), first fit.
fn place_chores(s: &mut DaySchedule, from: u32, inputs: &PlanInputs) {
    let mut order: Vec<&Chore> = inputs.chores.iter().collect();
    order.sort_by_key(|c| {
        (
            std::cmp::Reverse(c.urgency),
            c.deadline_slot.unwrap_or(u32::MAX),
            c.id,
        )
    });
    for c in order {
        let deadline = c
            .deadline_slot
            .map_or(String::new(), |d| format!(" before slot {d}"));
        match s.first_fit(from, c.len, None, c.deadline_slot) {
            Some(start) => {
                let reason = ReasonCode::builtin(
                    "chore_placed",
                    [
                        ("slot", n(start)),
                        ("slots", n(c.len)),
                        ("urgency", n(c.urgency)),
                    ],
                );
                let mut r = blank(Priority::Chore, &c.action, &c.params, start, c.len, reason);
                r.urgency = c.urgency;
                r.created_tick = c.created_tick;
                let _ = s.place(r);
            }
            None => s.dropped.push(Dropped {
                what: format!("chore {} '{}'", c.id, c.action),
                reason: ReasonCode::builtin(
                    "chore_dropped",
                    [
                        ("urgency", n(c.urgency)),
                        ("deadline", Canon::str(deadline)),
                    ],
                ),
            }),
        }
    }
}

/// Step 5: leisure, one weighted draw per free stretch; "leave open" competes with the options.
fn place_leisure(s: &mut DaySchedule, seed: Seed, pawn: EntityId, from: u32, inputs: &PlanInputs) {
    if inputs.leisure.is_empty() {
        return;
    }
    let day = i64::try_from(s.day).unwrap_or(i64::MAX);
    let mut slot = from;
    while slot < s.slots_per_day() {
        if s.owner_id(slot).is_some() {
            slot += 1;
            continue;
        }
        let run = (slot..s.slots_per_day())
            .take_while(|x| s.owner_id(*x).is_none())
            .count();
        let run = u32::try_from(run).unwrap_or(u32::MAX);
        let fits: Vec<&LeisureOption> = inputs
            .leisure
            .iter()
            .filter(|o| o.len > 0 && o.len <= run && o.weight > 0)
            .collect();
        let mut weights: Vec<u32> = fits.iter().map(|o| o.weight).collect();
        weights.push(inputs.open_weight);
        let rng = Rng::new(
            seed,
            Stream::SchedTiebreak,
            &[Key::Id(pawn), Key::Int(day), Key::Int(i64::from(slot))],
        );
        let total: u32 = weights.iter().fold(0u32, |a, w| a.saturating_add(*w));
        match rng.weighted_pick(0, &weights).and_then(|i| fits.get(i)) {
            Some(option) => {
                let reason = ReasonCode::builtin(
                    "leisure_chosen",
                    [
                        ("slot", n(slot)),
                        ("slots", n(option.len)),
                        ("weight", n(option.weight)),
                        ("total", n(total)),
                    ],
                );
                let _ = s.place(blank(
                    Priority::Leisure,
                    &option.action,
                    &option.params,
                    slot,
                    option.len,
                    reason,
                ));
                slot = slot.saturating_add(option.len);
            }
            None => slot += 1, // left open
        }
    }
}

/// Puts reservations that were lifted out of the schedule back: those whose original slots are still free
/// return unchanged (first, so a relocated one can never take their place); the rest move to the first
/// later free run, or are dropped with `no_free_slot`. Priority then urgency decides who relocates first.
fn reflow(s: &mut DaySchedule, mut displaced: Vec<Reservation>) {
    displaced.sort_by_key(|r| (r.priority, std::cmp::Reverse(r.urgency), r.id));
    // Removed but not actually in the way: those go straight back first, so a relocated reservation can
    // never take their place.
    let (in_the_way, untouched): (Vec<Reservation>, Vec<Reservation>) = displaced
        .into_iter()
        .partition(|r| !s.is_free_run(r.start, r.len));
    for r in untouched {
        let _ = s.place(r);
    }
    for mut r in in_the_way {
        let was = r.start;
        match s.first_fit(was.saturating_add(1), r.len, None, None) {
            Some(to) => {
                r.start = to;
                r.reason = ReasonCode::builtin("moved_later", [("from", n(was)), ("to", n(to))]);
                let _ = s.place(r);
            }
            None => s.dropped.push(Dropped {
                what: describe(&r),
                reason: ReasonCode::builtin("no_free_slot", [("what", Canon::str(describe(&r)))]),
            }),
        }
    }
}

fn commitment_reservation(c: &CommitmentReq, start: u32, reason: ReasonCode) -> Reservation {
    let mut r = blank(
        Priority::Commitment,
        &c.action,
        &c.params,
        start,
        c.len,
        reason,
    );
    r.created_tick = c.created_tick;
    r.reschedulable = c.reschedulable;
    r.commitment = Some(c.id);
    r
}

/// Reserves an accepted commitment at its agreed slots. Free slots are simply taken; slots held only by
/// chores or leisure are cleared (those move later or are dropped). Duties, urgent needs and other
/// commitments are never displaced: the call fails with `commitment_failed` and changes nothing.
pub fn reserve_commitment(
    s: &mut DaySchedule,
    c: &CommitmentReq,
    from: u32,
) -> Result<u32, ReasonCode> {
    let fail = |why: String| {
        ReasonCode::builtin(
            "commitment_failed",
            [("slot", n(c.start)), ("why", Canon::str(why))],
        )
    };
    if c.start < from {
        return Err(fail("that slot has already started".to_owned()));
    }
    let placed = ReasonCode::builtin(
        "commitment_placed",
        [("slot", n(c.start)), ("slots", n(c.len))],
    );
    if s.is_free_run(c.start, c.len) {
        return s
            .place(commitment_reservation(c, c.start, placed))
            .map_err(|_| fail("it does not fit in the day".to_owned()));
    }
    let end = c.start.saturating_add(c.len);
    if c.len == 0 || end > s.slots_per_day() {
        return Err(fail("it does not fit in the day".to_owned()));
    }
    let mut blockers: Vec<u32> = (c.start..end).filter_map(|x| s.owner_id(x)).collect();
    blockers.sort_unstable();
    blockers.dedup();
    if let Some(strong) = blockers
        .iter()
        .filter_map(|id| s.reservation(*id))
        .find(|r| !r.priority.displaceable() || r.start < from)
    {
        return Err(fail(format!("it overlaps a {}", describe(strong))));
    }
    let lifted: Vec<Reservation> = blockers.into_iter().filter_map(|id| s.remove(id)).collect();
    let result = s
        .place(commitment_reservation(c, c.start, placed))
        .map_err(|_| fail("it does not fit in the day".to_owned()));
    reflow(s, lifted);
    result
}

/// A need became critical mid-day: place its restoring activity from `from`, displacing priority 4-5
/// reservations if there is no room. Displaced reservations move to a later free slot or are dropped
/// (`no_free_slot`). Priorities 1 and 3 are never touched.
pub fn insert_urgent(s: &mut DaySchedule, from: u32, u: &UrgentNeed) -> Option<u32> {
    let before = (u.predicted_slot > from).then_some(u.predicted_slot);
    if s.first_fit(from.max(u.earliest), u.len, before, None)
        .is_some()
    {
        return place_one_urgent(s, from, u);
    }
    // Try displacing, weakest first (priority 5 before 4, lower urgency first, later start first).
    let mut candidates: Vec<(u8, u32, u32, u32)> = s
        .reservations()
        .filter(|r| r.priority.displaceable() && r.start >= from)
        .map(|r| (r.priority.number(), r.urgency, r.start, r.id))
        .collect();
    candidates.sort_by_key(|(p, urg, start, id)| {
        (std::cmp::Reverse(*p), *urg, std::cmp::Reverse(*start), *id)
    });
    let mut displaced: Vec<Reservation> = Vec::new();
    let mut placed = None;
    for (_, _, _, id) in candidates {
        let Some(r) = s.remove(id) else { continue };
        displaced.push(r);
        if s.first_fit(from.max(u.earliest), u.len, before, None)
            .is_some()
        {
            placed = place_one_urgent(s, from, u);
            break;
        }
    }
    if placed.is_none() {
        // Nothing helped: put everything back exactly where it was.
        for r in displaced {
            let _ = s.place(r);
        }
        return None;
    }
    if let Some(id) = placed {
        let names: Vec<String> = displaced.iter().map(describe).collect();
        if let Some(r) = s.reservations.get_mut(&id) {
            r.reason = ReasonCode::builtin(
                "urgent_need_displaced",
                [
                    ("need", Canon::str(u.need.clone())),
                    ("slot", n(r.start)),
                    ("displaced", Canon::str(names.join("; "))),
                ],
            );
        }
    }
    reflow(s, displaced);
    placed
}

impl Priority {
    pub fn from_number(n: u8) -> Option<Priority> {
        [
            Priority::Duty,
            Priority::UrgentNeed,
            Priority::Commitment,
            Priority::Chore,
            Priority::Leisure,
        ]
        .into_iter()
        .find(|p| p.number() == n)
    }
}

impl Reservation {
    pub fn from_reader(r: Reader<'_>) -> Result<Reservation, ReadError> {
        r.only(&[
            "id",
            "priority",
            "action",
            "params",
            "start",
            "len",
            "urgency",
            "created_tick",
            "reschedulable",
            "commitment",
            "reason",
        ])?;
        let pr = r.child("priority")?;
        let priority = Priority::from_number(pr.reader().u8()?)
            .ok_or_else(|| pr.reader().err("priority must be 1..=5"))?;
        Ok(Reservation {
            id: r.child("id")?.reader().u32()?,
            priority,
            action: r.child("action")?.reader().parse()?,
            params: r.child("params")?.reader().value().clone(),
            start: r.child("start")?.reader().u32()?,
            len: r.child("len")?.reader().u32()?,
            urgency: r.child("urgency")?.reader().u32()?,
            created_tick: r.child("created_tick")?.reader().u64()?,
            reschedulable: r.child("reschedulable")?.reader().bool()?,
            commitment: match r.maybe("commitment")? {
                Some(c) => Some(c.reader().parse()?),
                None => None,
            },
            reason: ReasonCode::from_reader(r.child("reason")?.reader())?,
        })
    }
}

impl Dropped {
    pub fn from_reader(r: Reader<'_>) -> Result<Dropped, ReadError> {
        r.only(&["what", "reason"])?;
        Ok(Dropped {
            what: r.child("what")?.reader().str()?.to_owned(),
            reason: ReasonCode::from_reader(r.child("reason")?.reader())?,
        })
    }
}

/// The most slots a day can have (one-minute slots).
const MAX_SLOTS: u32 = 1_440;

impl DaySchedule {
    /// Decodes and re-validates a schedule: ranges, overlaps and ids are all checked, so a corrupt save is
    /// refused rather than loaded into an inconsistent state.
    pub fn from_reader(r: Reader<'_>) -> Result<DaySchedule, ReadError> {
        r.only(&[
            "day",
            "slots",
            "reservations",
            "next_id",
            "notes",
            "dropped",
        ])?;
        let day = r.child("day")?.reader().u64()?;
        let slots = r.child("slots")?.reader().u32()?;
        if slots == 0 || slots > MAX_SLOTS {
            return Err(r.err(format!("a day has 1..={MAX_SLOTS} slots, found {slots}")));
        }
        let mut s = DaySchedule::new(day, slots);
        s.next_id = r.child("next_id")?.reader().u32()?;
        for item in r.child("reservations")?.reader().list()? {
            let res = Reservation::from_reader(item.reader())?;
            let id = res.id;
            s.place(res).map_err(|e| {
                item.reader()
                    .err(format!("reservation {id} cannot be placed: {e:?}"))
            })?;
        }
        for item in r.child("notes")?.reader().list()? {
            s.notes.push(ReasonCode::from_reader(item.reader())?);
        }
        for item in r.child("dropped")?.reader().list()? {
            s.dropped.push(Dropped::from_reader(item.reader())?);
        }
        s.check_invariants().map_err(|e| r.err(e))?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests;
