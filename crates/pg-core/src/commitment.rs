//! Commitments: the two-pawn agreement protocol (Blueprint §8.6).
//!
//! ```text
//! Proposed ──accept──▶ Accepted ──slot reached──▶ Active ──kept──▶ Completed
//!    │  │                  │                          │
//!    │  └─decline          └─────────failed───────────┴──▶ Failed
//!    └─expiry ▶ Expired    any live state ──cancel──▶ Cancelled
//! ```
//!
//! Reservations exist **only after acceptance**, and acceptance is atomic: both pawns' schedules take the
//! reservation or neither does. The row stored here is authoritative state; the systems in `activity`
//! move it through the states and record a [`ReasonCode`] for every decision.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::reason::ReasonCode;
use crate::schedule::CommitmentReq;
use pg_content::ActionId;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CommitState {
    Proposed,
    Accepted,
    Declined,
    Expired,
    Active,
    Completed,
    Failed,
    Cancelled,
}

impl CommitState {
    pub const fn name(self) -> &'static str {
        match self {
            CommitState::Proposed => "proposed",
            CommitState::Accepted => "accepted",
            CommitState::Declined => "declined",
            CommitState::Expired => "expired",
            CommitState::Active => "active",
            CommitState::Completed => "completed",
            CommitState::Failed => "failed",
            CommitState::Cancelled => "cancelled",
        }
    }

    pub fn from_name(name: &str) -> Option<CommitState> {
        [
            CommitState::Proposed,
            CommitState::Accepted,
            CommitState::Declined,
            CommitState::Expired,
            CommitState::Active,
            CommitState::Completed,
            CommitState::Failed,
            CommitState::Cancelled,
        ]
        .into_iter()
        .find(|s| s.name() == name)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            CommitState::Declined
                | CommitState::Expired
                | CommitState::Completed
                | CommitState::Failed
                | CommitState::Cancelled
        )
    }

    /// Whether the protocol allows `self -> next`.
    pub const fn can_become(self, next: CommitState) -> bool {
        use CommitState::*;
        matches!(
            (self, next),
            (Proposed, Accepted | Declined | Expired | Cancelled)
                | (Accepted, Active | Failed | Cancelled)
                | (Active, Completed | Failed | Cancelled)
        )
    }
}

/// An illegal state change was attempted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BadTransition {
    pub from: CommitState,
    pub to: CommitState,
}

impl std::fmt::Display for BadTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a {} commitment cannot become {}",
            self.from.name(),
            self.to.name()
        )
    }
}

impl std::error::Error for BadTransition {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commitment {
    pub id: EntityId,
    pub proposer: EntityId,
    pub invitee: EntityId,
    /// The day it is for.
    pub day: u64,
    pub start: u32,
    pub len: u32,
    pub action: ActionId,
    pub params: Canon,
    pub state: CommitState,
    pub created_tick: u64,
    /// The proposal lapses at this tick if nobody answered.
    pub expires_tick: u64,
    pub reschedulable: bool,
    /// The decision that put it in its current state.
    pub reason: Option<ReasonCode>,
}

impl Commitment {
    /// Moves to `next`, recording why. Illegal changes are refused and change nothing.
    pub fn transition(
        &mut self,
        next: CommitState,
        reason: ReasonCode,
    ) -> Result<(), BadTransition> {
        if !self.state.can_become(next) {
            return Err(BadTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        self.reason = Some(reason);
        Ok(())
    }

    /// The pawns involved, proposer first.
    pub fn parties(&self) -> [EntityId; 2] {
        [self.proposer, self.invitee]
    }

    /// The planner's view of this commitment.
    pub fn as_req(&self) -> CommitmentReq {
        CommitmentReq {
            id: self.id,
            action: self.action.clone(),
            params: self.params.clone(),
            start: self.start,
            len: self.len,
            reschedulable: self.reschedulable,
            created_tick: self.created_tick,
        }
    }

    /// Live commitments are the ones whose reservations the planner must keep.
    pub fn is_binding(&self) -> bool {
        matches!(self.state, CommitState::Accepted | CommitState::Active)
    }
}

impl ToCanon for Commitment {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("proposer", self.proposer.to_canon()),
            ("invitee", self.invitee.to_canon()),
            ("day", self.day.to_canon()),
            ("start", self.start.to_canon()),
            ("len", self.len.to_canon()),
            ("action", self.action.to_canon()),
            ("params", self.params.clone()),
            ("state", Canon::str(self.state.name())),
            ("created_tick", self.created_tick.to_canon()),
            ("expires_tick", self.expires_tick.to_canon()),
            ("reschedulable", self.reschedulable.to_canon()),
            (
                "reason",
                self.reason.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
        ])
    }
}

/// Cancels every live commitment (proposed, accepted or active) with `why`, returning their ids in id
/// order. Used when something the agreements were expressed in changes underneath them.
pub fn cancel_live(world: &mut crate::world::WorldState, why: &str) -> Vec<EntityId> {
    let mut cancelled = Vec::new();
    for (id, c) in world.commitments.iter_mut() {
        let reason = ReasonCode::builtin("commitment_cancelled", [("why", Canon::str(why))]);
        if c.transition(CommitState::Cancelled, reason).is_ok() {
            cancelled.push(id);
        }
    }
    cancelled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    const ALL: [CommitState; 8] = [
        CommitState::Proposed,
        CommitState::Accepted,
        CommitState::Declined,
        CommitState::Expired,
        CommitState::Active,
        CommitState::Completed,
        CommitState::Failed,
        CommitState::Cancelled,
    ];

    fn sample() -> Commitment {
        Commitment {
            id: EntityId::new(Kind::Commitment, 1),
            proposer: EntityId::new(Kind::Pawn, 1),
            invitee: EntityId::new(Kind::Pawn, 2),
            day: 0,
            start: 10,
            len: 2,
            action: ActionId::new("idle_at").unwrap(),
            params: Canon::Null,
            state: CommitState::Proposed,
            created_tick: 0,
            expires_tick: 100,
            reschedulable: false,
            reason: None,
        }
    }

    #[test]
    fn terminal_states_allow_no_further_change() {
        for from in ALL {
            for to in ALL {
                if from.is_terminal() {
                    assert!(!from.can_become(to), "{from:?} -> {to:?}");
                }
            }
        }
    }

    #[test]
    fn the_happy_path_and_the_failure_paths_are_the_documented_ones() {
        use CommitState::*;
        let mut c = sample();
        for next in [Accepted, Active, Completed] {
            c.transition(next, ReasonCode::simple("commitment_active"))
                .unwrap();
        }
        assert_eq!(c.state, Completed);
        let mut c = sample();
        c.transition(Declined, ReasonCode::simple("commitment_declined"))
            .unwrap();
        assert!(c.transition(Accepted, ReasonCode::simple("x")).is_err());
        assert_eq!(c.state, Declined, "a refused change leaves the state alone");
        // Proposed cannot skip to Active or Completed.
        let mut c = sample();
        assert!(c.transition(Active, ReasonCode::simple("x")).is_err());
        assert!(c.transition(Completed, ReasonCode::simple("x")).is_err());
        // Every live state can be cancelled.
        for live in [Proposed, Accepted, Active] {
            assert!(live.can_become(Cancelled));
        }
    }

    #[test]
    fn names_round_trip() {
        for s in ALL {
            assert_eq!(CommitState::from_name(s.name()), Some(s));
        }
        assert_eq!(CommitState::from_name("nope"), None);
    }

    #[test]
    fn only_accepted_and_active_commitments_are_binding() {
        for s in ALL {
            let mut c = sample();
            c.state = s;
            assert_eq!(
                c.is_binding(),
                matches!(s, CommitState::Accepted | CommitState::Active)
            );
        }
    }

    #[test]
    fn the_planner_view_carries_the_agreement() {
        let c = sample();
        let r = c.as_req();
        assert_eq!(
            (r.id, r.start, r.len, r.reschedulable),
            (c.id, 10, 2, false)
        );
        assert_eq!(c.parties(), [c.proposer, c.invitee]);
    }
}
