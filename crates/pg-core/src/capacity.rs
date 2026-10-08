//! Capacities (Blueprint §8.1, suggestion S-040): what a pawn can currently do, as a few derived integers.
//!
//! Movement, the scheduler, conversation and actions read these and never read needs or, later, body parts
//! directly. In Stage 1 the needs system fills them from needs (a collapsed pawn stops, a hungry one
//! fumbles); from Stage 2B the organism system takes over as their producer and the shortcut is removed, so
//! nothing that reads a capacity changes.

use crate::canon::{Canon, ToCanon};
use crate::read::{ReadError, Reader};

/// Capacities are permille: 1000 is fully able, 0 is not at all.
pub const FULL: i32 = 1000;

/// A pawn whose consciousness is below this cannot act (it neither walks nor starts tasks).
pub const ACT_THRESHOLD: i32 = 500;

/// The slowest a pawn walks, as a share of normal speed (permille), however low `moving` is.
pub const MIN_MOVING: i32 = 100;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Capacities {
    pub consciousness: i32,
    pub moving: i32,
    pub manipulation: i32,
    pub talking: i32,
    pub eating: i32,
    pub breathing: i32,
}

impl Default for Capacities {
    fn default() -> Self {
        Capacities::FULL
    }
}

impl Capacities {
    pub const FULL: Capacities = Capacities {
        consciousness: FULL,
        moving: FULL,
        manipulation: FULL,
        talking: FULL,
        eating: FULL,
        breathing: FULL,
    };

    pub fn get(&self, id: &str) -> Option<i32> {
        Some(match id {
            "consciousness" => self.consciousness,
            "moving" => self.moving,
            "manipulation" => self.manipulation,
            "talking" => self.talking,
            "eating" => self.eating,
            "breathing" => self.breathing,
            _ => return None,
        })
    }

    fn slot(&mut self, id: &str) -> Option<&mut i32> {
        Some(match id {
            "consciousness" => &mut self.consciousness,
            "moving" => &mut self.moving,
            "manipulation" => &mut self.manipulation,
            "talking" => &mut self.talking,
            "eating" => &mut self.eating,
            "breathing" => &mut self.breathing,
            _ => return None,
        })
    }

    /// Lowers `id` to at most `value` (never raises it). Unknown ids are ignored.
    pub fn cap(&mut self, id: &str, value: i32) {
        if let Some(slot) = self.slot(id) {
            *slot = (*slot).min(value.clamp(0, FULL));
        }
    }

    /// Whether the pawn is awake and able enough to walk and carry out tasks.
    pub fn can_act(&self) -> bool {
        self.consciousness >= ACT_THRESHOLD
    }

    /// Ticks for a step scaled by `moving`: slower as it drops, at most ten times slower.
    pub fn scale_step_ticks(&self, base: u32) -> u32 {
        let moving = u64::try_from(self.moving.clamp(MIN_MOVING, FULL)).unwrap_or(1000);
        let scaled = u64::from(base).saturating_mul(1000).div_ceil(moving);
        u32::try_from(scaled).unwrap_or(u32::MAX).max(base)
    }

    pub fn as_pairs(&self) -> [(&'static str, i32); 6] {
        [
            ("consciousness", self.consciousness),
            ("moving", self.moving),
            ("manipulation", self.manipulation),
            ("talking", self.talking),
            ("eating", self.eating),
            ("breathing", self.breathing),
        ]
    }
}

impl ToCanon for Capacities {
    fn to_canon(&self) -> Canon {
        Canon::map(self.as_pairs().map(|(k, v)| (k, v.to_canon())))
    }
}

impl Capacities {
    pub fn from_reader(r: Reader<'_>) -> Result<Capacities, ReadError> {
        r.only(&[
            "consciousness",
            "moving",
            "manipulation",
            "talking",
            "eating",
            "breathing",
        ])?;
        let get = |key: &str| -> Result<i32, ReadError> {
            let c = r.child(key)?;
            let v = c.reader().i32()?;
            if !(0..=FULL).contains(&v) {
                return Err(c.reader().err("a capacity is 0 to 1000"));
            }
            Ok(v)
        };
        Ok(Capacities {
            consciousness: get("consciousness")?,
            moving: get("moving")?,
            manipulation: get("manipulation")?,
            talking: get("talking")?,
            eating: get("eating")?,
            breathing: get("breathing")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::json;
    use pg_content::gamedata::CAPACITY_IDS;

    #[test]
    fn the_core_and_the_content_crate_agree_on_the_capacity_names() {
        let c = Capacities::FULL;
        let ours: Vec<&str> = c.as_pairs().iter().map(|(k, _)| *k).collect();
        assert_eq!(ours, CAPACITY_IDS);
        for id in CAPACITY_IDS {
            assert_eq!(c.get(id), Some(FULL));
        }
        assert_eq!(c.get("sight"), None);
    }

    #[test]
    fn capping_only_ever_lowers_and_unknown_names_are_ignored() {
        let mut c = Capacities::FULL;
        c.cap("moving", 700);
        c.cap("moving", 900);
        assert_eq!(c.moving, 700, "a later, milder cap does not raise it");
        c.cap("manipulation", -5);
        assert_eq!(c.manipulation, 0);
        c.cap("sight", 0);
        assert_eq!(
            c,
            Capacities {
                moving: 700,
                manipulation: 0,
                ..Capacities::FULL
            }
        );
    }

    #[test]
    fn a_pawn_acts_only_while_conscious_enough_and_walks_slower_as_moving_drops() {
        let mut c = Capacities::FULL;
        assert!(c.can_act());
        assert_eq!(c.scale_step_ticks(2), 2);
        c.moving = 500;
        assert_eq!(c.scale_step_ticks(2), 4);
        c.moving = 0;
        assert_eq!(
            c.scale_step_ticks(2),
            20,
            "never worse than ten times slower"
        );
        assert_eq!(c.scale_step_ticks(0), 0);
        c.cap("consciousness", ACT_THRESHOLD - 1);
        assert!(!c.can_act());
    }

    #[test]
    fn capacities_round_trip_and_out_of_range_values_are_refused() {
        let c = Capacities {
            moving: 850,
            ..Capacities::FULL
        };
        let text = c.to_canon().to_canonical_string();
        let back = Capacities::from_reader(Reader::new(&json::parse(&text).unwrap(), "c")).unwrap();
        assert_eq!(back, c);
        let bad = text.replace("850", "1500");
        assert!(Capacities::from_reader(Reader::new(&json::parse(&bad).unwrap(), "c")).is_err());
    }
}
