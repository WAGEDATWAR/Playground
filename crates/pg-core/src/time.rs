//! Simulation time (Blueprint §6.1).
//!
//! The core knows only ticks. A game minute is [`TICKS_PER_GAME_MINUTE`] ticks and a game day is
//! [`TICKS_PER_DAY`] ticks. The runtime decides how fast ticks arrive in real time; nothing here
//! reads a clock.
//!
//! **Boundary convention.** A step processes the inputs stamped with the current tick `T`, then
//! advances the clock to `T + 1`, then computes [`TimeFlags`] *for the tick just entered*. So the first
//! minute boundary is seen when the clock reaches tick 10, and the first day boundary at tick 14,400.

use crate::canon::{Canon, ToCanon};
use std::fmt;

pub const TICKS_PER_GAME_MINUTE: u64 = 10;
pub const MINUTES_PER_DAY: u64 = 1_440;
pub const TICKS_PER_DAY: u64 = TICKS_PER_GAME_MINUTE * MINUTES_PER_DAY;
pub const DEFAULT_SLOT_MINUTES: u32 = 30;

/// A rejected slot length.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BadSlotMinutes(pub u32);

impl fmt::Display for BadSlotMinutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "slot length {} min must be 1..=1440 and divide 1440 evenly",
            self.0
        )
    }
}

impl std::error::Error for BadSlotMinutes {}

/// The schedule slot length in game minutes. Always divides a day evenly, so slot boundaries
/// always coincide with minute and day boundaries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotMinutes(u32);

impl SlotMinutes {
    pub const DEFAULT: SlotMinutes = SlotMinutes(DEFAULT_SLOT_MINUTES);

    pub const fn new(minutes: u32) -> Result<SlotMinutes, BadSlotMinutes> {
        if minutes >= 1
            && minutes <= MINUTES_PER_DAY as u32
            && (MINUTES_PER_DAY as u32).is_multiple_of(minutes)
        {
            Ok(SlotMinutes(minutes))
        } else {
            Err(BadSlotMinutes(minutes))
        }
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    /// Ticks per slot.
    pub const fn ticks(self) -> u64 {
        self.0 as u64 * TICKS_PER_GAME_MINUTE
    }

    pub const fn slots_per_day(self) -> u32 {
        (MINUTES_PER_DAY as u32) / self.0
    }
}

impl Default for SlotMinutes {
    fn default() -> Self {
        SlotMinutes::DEFAULT
    }
}

/// The tick counter overflowed `u64` (unreachable in practice; reported rather than wrapped).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClockOverflow;

impl fmt::Display for ClockOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("simulation clock overflowed")
    }
}

impl std::error::Error for ClockOverflow {}

/// The only time source in the core (`WorldState.clock`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    tick: u64,
}

impl Clock {
    pub const fn new() -> Clock {
        Clock { tick: 0 }
    }

    pub const fn from_tick(tick: u64) -> Clock {
        Clock { tick }
    }

    pub const fn tick(self) -> u64 {
        self.tick
    }

    pub fn advance(&mut self) -> Result<(), ClockOverflow> {
        self.tick = self.tick.checked_add(1).ok_or(ClockOverflow)?;
        Ok(())
    }

    /// Zero-based day index.
    pub const fn day(self) -> u64 {
        self.tick / TICKS_PER_DAY
    }

    pub const fn minute_of_day(self) -> u32 {
        ((self.tick % TICKS_PER_DAY) / TICKS_PER_GAME_MINUTE) as u32
    }

    pub const fn slot_of_day(self, slot: SlotMinutes) -> u32 {
        ((self.tick % TICKS_PER_DAY) / slot.ticks()) as u32
    }
}

impl ToCanon for Clock {
    fn to_canon(&self) -> Canon {
        Canon::map([("tick", self.tick.to_canon())])
    }
}

/// Which boundaries the clock just crossed. `new_day` implies `slot` implies `minute`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimeFlags {
    /// The tick just entered.
    pub tick: u64,
    pub day: u64,
    pub minute_of_day: u32,
    pub slot_of_day: u32,
    pub minute: bool,
    pub slot: bool,
    pub new_day: bool,
}

/// Flags for having just entered `tick`.
pub fn flags_for(tick: u64, slot: SlotMinutes) -> TimeFlags {
    let clock = Clock::from_tick(tick);
    TimeFlags {
        tick,
        day: clock.day(),
        minute_of_day: clock.minute_of_day(),
        slot_of_day: clock.slot_of_day(slot),
        minute: tick.is_multiple_of(TICKS_PER_GAME_MINUTE),
        slot: tick.is_multiple_of(slot.ticks()),
        new_day: tick.is_multiple_of(TICKS_PER_DAY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn constants_match_the_spec() {
        assert_eq!(TICKS_PER_DAY, 14_400);
        assert_eq!(SlotMinutes::DEFAULT.ticks(), 300);
        assert_eq!(SlotMinutes::DEFAULT.slots_per_day(), 48);
    }

    #[test]
    fn slot_minutes_must_divide_the_day() {
        for ok in [
            1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 24, 30, 45, 60, 90, 120, 180, 240, 360, 480, 720,
            1440,
        ] {
            assert!(SlotMinutes::new(ok).is_ok(), "{ok}");
        }
        for bad in [0, 7, 11, 25, 31, 100, 1439, 1441, 2880, u32::MAX] {
            assert_eq!(SlotMinutes::new(bad), Err(BadSlotMinutes(bad)), "{bad}");
        }
    }

    #[test]
    fn boundary_ticks_with_default_slots() {
        let s = SlotMinutes::DEFAULT;
        let f = |t| flags_for(t, s);
        assert!(!f(9).minute && f(10).minute);
        assert!(f(10).minute && !f(10).slot);
        assert!(!f(299).slot && f(300).slot && f(300).minute);
        assert!(!f(14_399).new_day && f(14_400).new_day);
        assert!(f(14_400).slot && f(14_400).minute);
        assert_eq!(f(14_400).day, 1);
        assert_eq!(f(14_399).day, 0);
    }

    #[test]
    fn time_wraps_at_midnight() {
        let c = Clock::from_tick(TICKS_PER_DAY - 1);
        assert_eq!((c.day(), c.minute_of_day()), (0, 1439));
        let c = Clock::from_tick(TICKS_PER_DAY);
        assert_eq!(
            (
                c.day(),
                c.minute_of_day(),
                c.slot_of_day(SlotMinutes::DEFAULT)
            ),
            (1, 0, 0)
        );
        let c = Clock::from_tick(TICKS_PER_DAY * 3 + 300);
        assert_eq!(
            (
                c.day(),
                c.minute_of_day(),
                c.slot_of_day(SlotMinutes::DEFAULT)
            ),
            (3, 30, 1)
        );
    }

    #[test]
    fn slot_of_day_runs_zero_to_slots_per_day_minus_one() {
        let s = SlotMinutes::DEFAULT;
        assert_eq!(
            Clock::from_tick(TICKS_PER_DAY - 1).slot_of_day(s),
            s.slots_per_day() - 1
        );
    }

    #[test]
    fn advance_counts_and_reports_overflow() {
        let mut c = Clock::new();
        c.advance().unwrap();
        c.advance().unwrap();
        assert_eq!(c.tick(), 2);
        let mut max = Clock::from_tick(u64::MAX);
        assert_eq!(max.advance(), Err(ClockOverflow));
        assert_eq!(
            max.tick(),
            u64::MAX,
            "failed advance must not change the clock"
        );
    }

    proptest! {
        #[test]
        fn boundaries_nest(tick in 1u64..10_000_000, slot_idx in 0usize..8) {
            let slots = [1u32, 5, 10, 30, 60, 120, 480, 1440];
            let s = SlotMinutes::new(slots[slot_idx]).unwrap();
            let f = flags_for(tick, s);
            prop_assert!(!f.new_day || f.slot);
            prop_assert!(!f.slot || f.minute);
            prop_assert!(f.minute_of_day < 1440);
            prop_assert!(f.slot_of_day < s.slots_per_day());
        }

        #[test]
        fn exactly_one_new_day_per_day_of_ticks(start in 1u64..1_000_000) {
            let s = SlotMinutes::DEFAULT;
            let days = (start..start + TICKS_PER_DAY).filter(|&t| flags_for(t, s).new_day).count();
            prop_assert_eq!(days, 1);
        }
    }
}
