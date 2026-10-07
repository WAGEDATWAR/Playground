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
}

/// Combines the packs' answers with the point's combiner and clamps them to the engine's range.
pub fn resolve(point: &HookPoint, answers: &[i64]) -> i64 {
    point.resolve(answers)
}

/// `movement.speed_modifier`: permille of normal walking speed.
pub fn movement_speed() -> Option<&'static HookPoint> {
    pg_api::hook_point("movement.speed_modifier")
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
