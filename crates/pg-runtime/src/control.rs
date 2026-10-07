//! Run states, the fixed-step accumulator and the autosave timer (Blueprint §6.1, §6.4).
//!
//! These are small pure state machines over a monotonic time value, so every rule is tested without
//! threads or real clocks. Real time only decides *how many* ticks run between frames; the ticks
//! themselves are discrete and identical however the frames fall.

use std::time::Duration;

/// How fast the world runs. One simulated day is 14,400 ticks; at `Normal` that is a 24-minute day.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Speed {
    Normal,
    Fast,
    Faster,
    Fastest,
}

impl Speed {
    pub const ALL: [Speed; 4] = [Speed::Normal, Speed::Fast, Speed::Faster, Speed::Fastest];

    /// Ticks per real second. A day is 14,400 ticks, so `Normal` (10 per second) makes a day last 24 real
    /// minutes, inside the Roadmap's 20-30 minute target.
    pub const fn ticks_per_second(self) -> u64 {
        match self {
            Speed::Normal => 10,
            Speed::Fast => 30,
            Speed::Faster => 90,
            Speed::Fastest => 270,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Speed::Normal => "1x",
            Speed::Fast => "3x",
            Speed::Faster => "9x",
            Speed::Fastest => "27x",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    /// Time advances at the given speed.
    Running(Speed),
    /// The player paused.
    Paused,
    /// Focus was lost (or the window minimised): the world paused and saved, and waits for the player to
    /// resume rather than carrying on unseen.
    Suspended,
    /// A close was requested: a final save is due, then the loop ends.
    Closing,
    /// The loop has ended.
    Stopped,
    /// The last-resort guard caught a panic; the world is frozen and a report was written.
    Crashed,
}

/// Things that change the run state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Pause,
    Resume(Speed),
    SetSpeed(Speed),
    FocusLost,
    /// Focus returned. The world stays suspended until the player resumes.
    FocusGained,
    CloseRequested,
    Finished,
    Panicked,
}

/// What the runtime should do as a result of a transition.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Save now (focus lost, close requested).
    SaveNow,
}

impl RunState {
    pub fn is_running(self) -> bool {
        matches!(self, RunState::Running(_))
    }

    pub fn speed(self) -> Option<Speed> {
        match self {
            RunState::Running(s) => Some(s),
            _ => None,
        }
    }

    /// The next state and what to do. Invalid events (resuming a crashed world, anything after `Stopped`)
    /// change nothing.
    pub fn on(self, e: RunEvent) -> (RunState, Effect) {
        use RunEvent::*;
        use RunState::*;
        match (self, e) {
            (Stopped, _) => (Stopped, Effect::None),
            (Crashed, Finished | CloseRequested) => (Stopped, Effect::None),
            (Crashed, _) => (Crashed, Effect::None),
            (_, Panicked) => (Crashed, Effect::None),
            (_, Finished) => (Stopped, Effect::None),
            (Closing, _) => (Closing, Effect::None),
            (_, CloseRequested) => (Closing, Effect::SaveNow),
            (Running(_), Pause) => (Paused, Effect::None),
            (Running(_), FocusLost) => (Suspended, Effect::SaveNow),
            (Running(_), SetSpeed(s)) => (Running(s), Effect::None),
            (Running(_), Resume(s)) => (Running(s), Effect::None),
            (Paused, Resume(s)) => (Running(s), Effect::None),
            (Paused, FocusLost) => (Suspended, Effect::SaveNow),
            (Suspended, Resume(s)) => (Running(s), Effect::None),
            (Suspended, FocusGained) => (Suspended, Effect::None),
            (s, _) => (s, Effect::None),
        }
    }
}

/// Turns real elapsed time into a whole number of ticks, carrying the remainder so no time is lost or
/// invented. A per-frame cap stops a long stall (a dragged window, a debugger) from running a burst of
/// ticks that freezes the UI: excess time is dropped, not queued.
#[derive(Clone, Debug)]
pub struct Accumulator {
    /// Nanoseconds-times-ticks not yet converted.
    carry: u128,
    max_ticks_per_frame: u64,
}

const NANOS: u128 = 1_000_000_000;

impl Accumulator {
    pub fn new(max_ticks_per_frame: u64) -> Accumulator {
        Accumulator {
            carry: 0,
            max_ticks_per_frame: max_ticks_per_frame.max(1),
        }
    }

    /// Ticks to run for `elapsed` real time at `speed`.
    pub fn advance(&mut self, elapsed: Duration, speed: Speed) -> u64 {
        // elapsed_ns * ticks_per_second / 1e9, kept exact by carrying the numerator remainder.
        self.carry += elapsed.as_nanos() * u128::from(speed.ticks_per_second());
        let whole = self.carry / NANOS;
        self.carry %= NANOS;
        let whole = u64::try_from(whole).unwrap_or(u64::MAX);
        if whole > self.max_ticks_per_frame {
            self.carry = 0; // drop the backlog
            self.max_ticks_per_frame
        } else {
            whole
        }
    }

    /// Forget any partial tick (when pausing, so resuming starts clean).
    pub fn reset(&mut self) {
        self.carry = 0;
    }
}

/// Autosave every N real minutes **of running time** (paused or suspended time does not count).
#[derive(Clone, Debug)]
pub struct AutosaveTimer {
    interval: Duration,
    running_for: Duration,
    last: Option<Duration>,
}

impl AutosaveTimer {
    pub fn new(interval_minutes: u64) -> AutosaveTimer {
        AutosaveTimer {
            interval: Duration::from_secs(interval_minutes.max(1) * 60),
            running_for: Duration::ZERO,
            last: None,
        }
    }

    pub fn set_interval_minutes(&mut self, minutes: u64) {
        self.interval = Duration::from_secs(minutes.max(1) * 60);
    }

    /// Call once per frame with the monotonic time and whether the world is running. Returns true when a
    /// save is due (and restarts the interval).
    pub fn poll(&mut self, now: Duration, running: bool) -> bool {
        let prev = self.last.replace(now).unwrap_or(now);
        if running {
            self.running_for += now.saturating_sub(prev);
        }
        if self.running_for >= self.interval {
            self.running_for = Duration::ZERO;
            return true;
        }
        false
    }

    /// A save happened for another reason (manual, focus loss): restart the interval.
    pub fn saved(&mut self) {
        self.running_for = Duration::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_day_lasts_about_24_real_minutes_at_normal_speed() {
        // 14,400 ticks at 10 ticks per second.
        assert_eq!(14_400 / Speed::Normal.ticks_per_second(), 1_440);
        let mut prev = 0;
        for s in Speed::ALL {
            assert!(s.ticks_per_second() > prev);
            prev = s.ticks_per_second();
            assert!(!s.name().is_empty());
        }
    }

    #[test]
    fn the_state_machine_follows_the_documented_rules() {
        use RunEvent::*;
        use RunState::*;
        let n = Speed::Normal;
        assert_eq!(Paused.on(Resume(n)), (Running(n), Effect::None));
        assert_eq!(Running(n).on(Pause), (Paused, Effect::None));
        assert_eq!(
            Running(n).on(SetSpeed(Speed::Fast)),
            (Running(Speed::Fast), Effect::None)
        );
        assert_eq!(
            Paused.on(SetSpeed(Speed::Fast)),
            (Paused, Effect::None),
            "speed changes do not unpause"
        );
        // Focus loss pauses and saves; regaining focus does NOT resume by itself.
        assert_eq!(Running(n).on(FocusLost), (Suspended, Effect::SaveNow));
        assert_eq!(Paused.on(FocusLost), (Suspended, Effect::SaveNow));
        assert_eq!(Suspended.on(FocusGained), (Suspended, Effect::None));
        assert_eq!(Suspended.on(Resume(n)), (Running(n), Effect::None));
        // Close saves, then finishes; nothing resumes a closing world.
        assert_eq!(Running(n).on(CloseRequested), (Closing, Effect::SaveNow));
        assert_eq!(Suspended.on(CloseRequested), (Closing, Effect::SaveNow));
        assert_eq!(Closing.on(Resume(n)), (Closing, Effect::None));
        assert_eq!(Closing.on(Finished), (Stopped, Effect::None));
        // A panic freezes the world for good.
        assert_eq!(Running(n).on(Panicked), (Crashed, Effect::None));
        assert_eq!(Crashed.on(Resume(n)), (Crashed, Effect::None));
        assert_eq!(Crashed.on(Finished), (Stopped, Effect::None));
        assert_eq!(Stopped.on(Resume(n)), (Stopped, Effect::None));
        assert!(Running(n).is_running() && !Paused.is_running());
        assert_eq!(Running(Speed::Fast).speed(), Some(Speed::Fast));
    }

    #[test]
    fn every_state_and_event_pair_is_total_and_stopped_is_final() {
        use RunEvent::*;
        let states = [
            RunState::Running(Speed::Normal),
            RunState::Paused,
            RunState::Suspended,
            RunState::Closing,
            RunState::Stopped,
            RunState::Crashed,
        ];
        let events = [
            Pause,
            Resume(Speed::Fast),
            SetSpeed(Speed::Faster),
            FocusLost,
            FocusGained,
            CloseRequested,
            Finished,
            Panicked,
        ];
        for s in states {
            for e in events {
                let (next, _) = s.on(e);
                if s == RunState::Stopped {
                    assert_eq!(next, RunState::Stopped, "{e:?}");
                }
                if s == RunState::Crashed {
                    assert!(
                        matches!(next, RunState::Crashed | RunState::Stopped),
                        "{e:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_accumulator_converts_time_exactly_and_carries_the_remainder() {
        let mut a = Accumulator::new(1_000);
        // 10 ticks per second: 150 ms is 1 tick with 50 ms carried.
        assert_eq!(a.advance(Duration::from_millis(150), Speed::Normal), 1);
        assert_eq!(a.advance(Duration::from_millis(50), Speed::Normal), 1);
        assert_eq!(a.advance(Duration::from_millis(99), Speed::Normal), 0);
        assert_eq!(a.advance(Duration::from_millis(1), Speed::Normal), 1);
        assert_eq!(a.advance(Duration::from_secs(2), Speed::Fast), 60);
    }

    #[test]
    fn a_long_stall_is_capped_and_the_backlog_dropped() {
        let mut a = Accumulator::new(50);
        assert_eq!(a.advance(Duration::from_secs(60), Speed::Normal), 50);
        assert_eq!(
            a.advance(Duration::from_millis(100), Speed::Normal),
            1,
            "no backlog remains"
        );
        a.advance(Duration::from_millis(70), Speed::Normal);
        a.reset();
        assert_eq!(a.advance(Duration::from_millis(40), Speed::Normal), 0);
    }

    proptest! {
        /// However the same total time is cut into frames, the same number of ticks run.
        #[test]
        fn frame_boundaries_do_not_change_the_tick_count(frames in prop::collection::vec(1u64..400, 1..60)) {
            let total: u64 = frames.iter().sum();
            let mut split = Accumulator::new(u64::MAX);
            let by_frames: u64 = frames.iter().map(|ms| split.advance(Duration::from_millis(*ms), Speed::Faster)).sum();
            let mut whole = Accumulator::new(u64::MAX);
            let at_once = whole.advance(Duration::from_millis(total), Speed::Faster);
            prop_assert_eq!(by_frames, at_once);
        }
    }

    #[test]
    fn autosave_counts_only_running_time() {
        let mut t = AutosaveTimer::new(5);
        let s = |n| Duration::from_secs(n);
        assert!(!t.poll(s(0), true));
        assert!(!t.poll(s(200), true));
        // Ten paused minutes do not count.
        assert!(!t.poll(s(800), false));
        assert!(!t.poll(s(850), true), "only 250 s of running time so far");
        assert!(t.poll(s(900), true), "300 s of running time: due");
        // The interval restarts after each save, including saves made for other reasons.
        assert!(!t.poll(s(1000), true));
        t.saved();
        assert!(!t.poll(s(1250), true));
        assert!(t.poll(s(1300), true));
        t.set_interval_minutes(1);
        assert!(t.poll(s(1400), true));
    }
}
