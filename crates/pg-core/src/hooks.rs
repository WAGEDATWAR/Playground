//! Hook points: where the engine asks packs for a bounded value (Blueprint §23.1 item 6, §23.9).
//!
//! The core only knows the [`HookHost`] trait; the script host implements it. Hook points, their
//! combiners and their clamps are described once, in `pg-api`. A hook can bias a decision but never
//! replace a hard rule: whatever the packs answer, [`resolve`] clamps the combined value into the range
//! the engine allows.

use crate::id::EntityId;
use crate::world::WorldState;
pub use pg_api::HookPoint;

/// Answers hook questions from packs. Implementations run on the simulation thread.
pub trait HookHost: Send {
    /// The answers of every pack that registered for `point`, in pack load order. `subject` is the entity
    /// the question is about; `world` is a read-only view. No answers means "no opinion".
    fn ask(&mut self, point: &'static HookPoint, world: &WorldState, subject: EntityId)
        -> Vec<i64>;

    /// Like [`HookHost::ask`], but says which pack gave each answer, and changes nothing: no errors are
    /// recorded, nothing is metered. For explaining to a person what packs are doing to a pawn (the
    /// inspector); the simulation never uses it. The default is no explanation.
    fn explain(
        &mut self,
        _point: &'static HookPoint,
        _world: &WorldState,
        _subject: EntityId,
    ) -> Vec<(String, i64)> {
        Vec::new()
    }
}

/// What packs are doing to one pawn through one hook point: each pack's answer and the combined, clamped value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookEffect {
    pub point: &'static str,
    pub resolved: i64,
    /// `(pack id, its answer)` in load order.
    pub packs: Vec<(String, i64)>,
}

/// The combined, clamped answer of the packs at `point` for `subject`, or `None` when nothing is installed
/// that answers (so callers keep their normal arithmetic when there are no packs).
pub fn resolved(
    hooks: &mut Option<Box<dyn HookHost>>,
    point: Option<&'static HookPoint>,
    world: &WorldState,
    subject: EntityId,
) -> Option<i64> {
    let (point, host) = (point?, hooks.as_mut()?);
    let answers = host.ask(point, world, subject);
    if answers.is_empty() {
        None
    } else {
        Some(resolve(point, &answers))
    }
}

/// `need.decay_modifier`: permille scaling how fast needs fall.
pub fn need_decay() -> Option<&'static HookPoint> {
    pg_api::hook_point("need.decay_modifier")
}

/// `mood.comfort_shift`: points added to the need levels the mood rules read.
pub fn mood_comfort() -> Option<&'static HookPoint> {
    pg_api::hook_point("mood.comfort_shift")
}

/// `conversation.chance_modifier`: permille scaling the chance a pair starts talking.
pub fn conversation_chance() -> Option<&'static HookPoint> {
    pg_api::hook_point("conversation.chance_modifier")
}

/// Combines the packs' answers with the point's combiner and clamps them to the engine's range.
pub fn resolve(point: &HookPoint, answers: &[i64]) -> i64 {
    point.resolve(answers)
}

/// `movement.speed_modifier`: permille of normal walking speed.
pub fn movement_speed() -> Option<&'static HookPoint> {
    pg_api::hook_point("movement.speed_modifier")
}

/// `memory.importance_modifier`: permille scaling a new memory's importance.
pub fn memory_importance() -> Option<&'static HookPoint> {
    pg_api::hook_point("memory.importance_modifier")
}

/// `relationship.delta_modifier`: permille scaling a conversation's change to a relationship.
pub fn relationship_delta() -> Option<&'static HookPoint> {
    pg_api::hook_point("relationship.delta_modifier")
}

/// The number of ticks a step takes when a pawn walks at `permille` of normal speed (never below one).
pub fn scaled_step_ticks(base: u32, permille: i64) -> u32 {
    if permille <= 0 {
        return base;
    }
    let scaled = (i64::from(base) * 1000 + permille / 2) / permille;
    u32::try_from(scaled.max(1)).unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_ticks_scale_inversely_with_speed() {
        assert_eq!(scaled_step_ticks(10, 1000), 10);
        assert_eq!(scaled_step_ticks(10, 500), 20);
        assert_eq!(scaled_step_ticks(10, 1500), 7);
        assert_eq!(scaled_step_ticks(2, 1500), 1);
        assert_eq!(scaled_step_ticks(1, 1500), 1);
        assert_eq!(scaled_step_ticks(2, 500), 4);
        assert_eq!(scaled_step_ticks(5, 0), 5);
    }

    #[test]
    fn the_movement_point_exists_and_clamps() {
        let p = movement_speed().expect("the movement hook is part of the API");
        assert_eq!(resolve(p, &[]), 1000);
        assert_eq!(resolve(p, &[10_000]), 1500);
        assert_eq!(resolve(p, &[1]), 500);
    }
}
