//! `SimInput` and the ordered input queue (Blueprint §5.3, §6.2 step 1).
//!
//! Every external influence on the world is a `SimInput` stamped with the tick at which it applies.
//! `(initial state | seed + content) + input log` always reproduces the same state, which is what
//! makes replay, save/reload equivalence and bug reports possible.
//!
//! Inputs due on the same tick apply in the order `(kind order, actor, sequence)`, so the result never
//! depends on how the runtime happened to deliver them.

use crate::canon::{Canon, CanonError, ToCanon};
use crate::id::EntityId;
use std::collections::BTreeMap;
use std::fmt;

/// Gameplay commands (UI → core). Grows with the milestones; non-exhaustive on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// **Dev scaffolding:** adds `amount` to the dev probe value. Developer tool only.
    DevNudge { amount: i32 },
}

/// A change to a world setting. Validated when applied; an invalid value is rejected, never clamped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingChange {
    SlotMinutes(u32),
}

/// Everything that can enter the simulation from outside.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimInput {
    SettingChange(SettingChange),
    Command {
        actor: Option<EntityId>,
        cmd: Command,
    },
}

impl SimInput {
    /// Application order between kinds on the same tick: settings first, then commands.
    pub fn kind_order(&self) -> u8 {
        match self {
            SimInput::SettingChange(_) => 0,
            SimInput::Command { .. } => 1,
        }
    }

    pub fn actor(&self) -> Option<EntityId> {
        match self {
            SimInput::SettingChange(_) => None,
            SimInput::Command { actor, .. } => *actor,
        }
    }
}

/// An input with the tick it applies on and its submission sequence number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampedInput {
    pub tick: u64,
    pub seq: u64,
    pub input: SimInput,
}

/// Why a submission was refused.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The input's tick has already been simulated.
    InThePast { input_tick: u64, now: u64 },
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubmitError::InThePast { input_tick, now } => {
                write!(f, "input for tick {input_tick} submitted at tick {now}: that tick is already simulated")
            }
        }
    }
}

impl std::error::Error for SubmitError {}

type QueueKey = (u64, u8, Option<EntityId>, u64);

/// Pending inputs, ordered for application.
#[derive(Clone, Debug, Default)]
pub struct InputQueue {
    pending: BTreeMap<QueueKey, StampedInput>,
    next_seq: u64,
}

impl InputQueue {
    pub fn new() -> InputQueue {
        InputQueue::default()
    }

    /// Queues `input` to apply at `tick`. `now` is the current clock tick.
    pub fn submit(
        &mut self,
        now: u64,
        tick: u64,
        input: SimInput,
    ) -> Result<StampedInput, SubmitError> {
        if tick < now {
            return Err(SubmitError::InThePast {
                input_tick: tick,
                now,
            });
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let stamped = StampedInput { tick, seq, input };
        let key = (tick, stamped.input.kind_order(), stamped.input.actor(), seq);
        self.pending.insert(key, stamped.clone());
        Ok(stamped)
    }

    /// Removes and returns every input due at `tick`, in application order.
    pub fn drain_for(&mut self, tick: u64) -> Vec<StampedInput> {
        let later = self
            .pending
            .split_off(&(tick.saturating_add(1), 0, None, 0));
        let due = std::mem::replace(&mut self.pending, later);
        due.into_values().collect()
    }

    /// The pending inputs in application order, plus the next sequence number (for snapshots).
    pub fn snapshot(&self) -> (Vec<StampedInput>, u64) {
        (self.pending.values().cloned().collect(), self.next_seq)
    }

    /// Rebuilds a queue from a snapshot. Keys are recomputed from the inputs, so order is preserved.
    pub fn restore(next_seq: u64, inputs: Vec<StampedInput>) -> InputQueue {
        let pending = inputs
            .into_iter()
            .map(|s| ((s.tick, s.input.kind_order(), s.input.actor(), s.seq), s))
            .collect();
        InputQueue { pending, next_seq }
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

impl ToCanon for Command {
    fn to_canon(&self) -> Canon {
        match self {
            Command::DevNudge { amount } => Canon::map([
                ("type", Canon::str("dev_nudge")),
                ("amount", amount.to_canon()),
            ]),
        }
    }
}

impl Command {
    pub fn from_canon(c: &Canon) -> Result<Command, CanonError> {
        let ty = c
            .field("type")?
            .as_str()
            .ok_or_else(|| CanonError::new("command type must be text"))?;
        match ty {
            "dev_nudge" => {
                let amount = c
                    .field("amount")?
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok());
                Ok(Command::DevNudge {
                    amount: amount.ok_or_else(|| CanonError::new("bad dev_nudge amount"))?,
                })
            }
            other => Err(CanonError(format!("unknown command type '{other}'"))),
        }
    }
}

impl ToCanon for SettingChange {
    fn to_canon(&self) -> Canon {
        match self {
            SettingChange::SlotMinutes(m) => {
                Canon::map([("key", Canon::str("slot_minutes")), ("value", m.to_canon())])
            }
        }
    }
}

impl SettingChange {
    pub fn from_canon(c: &Canon) -> Result<SettingChange, CanonError> {
        let key = c
            .field("key")?
            .as_str()
            .ok_or_else(|| CanonError::new("setting key must be text"))?;
        match key {
            "slot_minutes" => {
                let v = c
                    .field("value")?
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok());
                Ok(SettingChange::SlotMinutes(v.ok_or_else(|| {
                    CanonError::new("bad slot_minutes value")
                })?))
            }
            other => Err(CanonError(format!("unknown setting '{other}'"))),
        }
    }
}

impl ToCanon for SimInput {
    fn to_canon(&self) -> Canon {
        match self {
            SimInput::SettingChange(s) => {
                Canon::map([("kind", Canon::str("setting")), ("change", s.to_canon())])
            }
            SimInput::Command { actor, cmd } => Canon::map([
                ("kind", Canon::str("command")),
                ("actor", actor.to_canon()),
                ("cmd", cmd.to_canon()),
            ]),
        }
    }
}

impl SimInput {
    pub fn from_canon(c: &Canon) -> Result<SimInput, CanonError> {
        let kind = c
            .field("kind")?
            .as_str()
            .ok_or_else(|| CanonError::new("input kind must be text"))?;
        match kind {
            "setting" => Ok(SimInput::SettingChange(SettingChange::from_canon(
                c.field("change")?,
            )?)),
            "command" => {
                let actor = match c.field("actor")? {
                    Canon::Null => None,
                    other => Some(
                        other
                            .as_str()
                            .and_then(|s| s.parse::<EntityId>().ok())
                            .ok_or_else(|| CanonError::new("bad actor id"))?,
                    ),
                };
                Ok(SimInput::Command {
                    actor,
                    cmd: Command::from_canon(c.field("cmd")?)?,
                })
            }
            other => Err(CanonError(format!("unknown input kind '{other}'"))),
        }
    }
}

impl ToCanon for StampedInput {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("tick", self.tick.to_canon()),
            ("seq", self.seq.to_canon()),
            ("input", self.input.to_canon()),
        ])
    }
}

impl StampedInput {
    pub fn from_canon(c: &Canon) -> Result<StampedInput, CanonError> {
        Ok(StampedInput {
            tick: c
                .field("tick")?
                .as_u64()
                .ok_or_else(|| CanonError::new("bad tick"))?,
            seq: c
                .field("seq")?
                .as_u64()
                .ok_or_else(|| CanonError::new("bad seq"))?,
            input: SimInput::from_canon(c.field("input")?)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    fn nudge(actor: Option<EntityId>, amount: i32) -> SimInput {
        SimInput::Command {
            actor,
            cmd: Command::DevNudge { amount },
        }
    }

    fn pawn(n: u32) -> EntityId {
        EntityId::new(Kind::Pawn, n)
    }

    #[test]
    fn past_ticks_are_rejected_and_present_or_future_accepted() {
        let mut q = InputQueue::new();
        assert!(matches!(
            q.submit(10, 9, nudge(None, 1)),
            Err(SubmitError::InThePast {
                input_tick: 9,
                now: 10
            })
        ));
        assert!(q.submit(10, 10, nudge(None, 1)).is_ok());
        assert!(q.submit(10, 99, nudge(None, 1)).is_ok());
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn drain_returns_only_due_inputs() {
        let mut q = InputQueue::new();
        q.submit(0, 5, nudge(None, 1)).unwrap();
        q.submit(0, 6, nudge(None, 2)).unwrap();
        q.submit(0, 5, nudge(None, 3)).unwrap();
        assert!(q.drain_for(4).is_empty());
        let due: Vec<_> = q.drain_for(5).into_iter().map(|s| s.seq).collect();
        assert_eq!(due, vec![0, 2]);
        assert_eq!(q.len(), 1);
        assert_eq!(q.drain_for(5).len(), 0, "drained inputs do not come back");
        assert_eq!(q.drain_for(6).len(), 1);
    }

    #[test]
    fn order_is_kind_then_actor_then_sequence_regardless_of_submission_order() {
        let mut q = InputQueue::new();
        q.submit(0, 7, nudge(Some(pawn(3)), 30)).unwrap(); // seq 0
        q.submit(0, 7, nudge(Some(pawn(1)), 10)).unwrap(); // seq 1
        q.submit(0, 7, nudge(None, 0)).unwrap(); // seq 2
        q.submit(
            0,
            7,
            SimInput::SettingChange(SettingChange::SlotMinutes(60)),
        )
        .unwrap(); // seq 3
        q.submit(0, 7, nudge(Some(pawn(1)), 11)).unwrap(); // seq 4
        let order: Vec<_> = q.drain_for(7).into_iter().map(|s| s.seq).collect();
        // setting first; then commands by actor (None < pawn_1 < pawn_3), ties by sequence.
        assert_eq!(order, vec![3, 2, 1, 4, 0]);
    }

    #[test]
    fn canon_round_trip() {
        let inputs = [
            nudge(None, -5),
            nudge(Some(pawn(46)), i32::MAX),
            SimInput::SettingChange(SettingChange::SlotMinutes(15)),
        ];
        for (i, input) in inputs.into_iter().enumerate() {
            let stamped = StampedInput {
                tick: 100 + i as u64,
                seq: i as u64,
                input,
            };
            let back = StampedInput::from_canon(&stamped.to_canon()).unwrap();
            assert_eq!(back, stamped);
        }
    }

    #[test]
    fn decoding_rejects_malformed_input() {
        let bad = Canon::map([("kind", Canon::str("teleport"))]);
        assert!(SimInput::from_canon(&bad).is_err());
        let no_amount = Canon::map([("type", Canon::str("dev_nudge"))]);
        assert!(Command::from_canon(&no_amount).is_err());
        let huge = Canon::map([
            ("type", Canon::str("dev_nudge")),
            ("amount", Canon::Int(i128::from(i64::MAX))),
        ]);
        assert!(Command::from_canon(&huge).is_err(), "amount must fit i32");
        assert!(SettingChange::from_canon(&Canon::map([
            ("key", Canon::str("nope")),
            ("value", Canon::Int(1))
        ]))
        .is_err());
    }
}
