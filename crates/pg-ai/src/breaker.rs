//! Circuit breaker (Blueprint §10.3): after N consecutive failures the provider is left alone for a while,
//! then a single probe request decides whether to resume. A broken provider never stalls play.
//!
//! The breaker is a plain state machine over a monotonic time value passed in by the caller, so tests
//! drive it with a fake clock.

use std::time::Duration;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BreakerConfig {
    /// Consecutive failures that open the breaker.
    pub failures_to_open: u32,
    /// How long it stays open before a probe is allowed.
    pub open_for: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        BreakerConfig {
            failures_to_open: 3,
            open_for: Duration::from_secs(30),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BreakerState {
    Closed {
        failures: u32,
    },
    Open {
        until: Duration,
    },
    /// One probe request is allowed; `probing` is true while it is in flight.
    HalfOpen {
        probing: bool,
    },
}

/// What the breaker says about sending a request now.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Permit {
    Yes,
    /// Send it, as the single probe; report the outcome.
    Probe,
    No {
        retry_in: Duration,
    },
}

#[derive(Clone, Debug)]
pub struct CircuitBreaker {
    cfg: BreakerConfig,
    state: BreakerState,
}

impl CircuitBreaker {
    pub fn new(cfg: BreakerConfig) -> CircuitBreaker {
        CircuitBreaker {
            cfg,
            state: BreakerState::Closed { failures: 0 },
        }
    }

    pub fn state(&self) -> BreakerState {
        self.state
    }

    /// May a request go out at monotonic time `now`?
    pub fn permit(&mut self, now: Duration) -> Permit {
        match self.state {
            BreakerState::Closed { .. } => Permit::Yes,
            BreakerState::Open { until } if now >= until => {
                self.state = BreakerState::HalfOpen { probing: true };
                Permit::Probe
            }
            BreakerState::Open { until } => Permit::No {
                retry_in: until - now,
            },
            BreakerState::HalfOpen { probing: false } => {
                self.state = BreakerState::HalfOpen { probing: true };
                Permit::Probe
            }
            // Someone else's probe is in flight: wait a moment rather than pile on.
            BreakerState::HalfOpen { probing: true } => Permit::No {
                retry_in: Duration::from_secs(1),
            },
        }
    }

    pub fn record_success(&mut self) {
        self.state = BreakerState::Closed { failures: 0 };
    }

    /// A failure that counts against the provider's health.
    pub fn record_failure(&mut self, now: Duration) {
        self.state = match self.state {
            BreakerState::Closed { failures } => {
                let failures = failures + 1;
                if failures >= self.cfg.failures_to_open {
                    BreakerState::Open {
                        until: now + self.cfg.open_for,
                    }
                } else {
                    BreakerState::Closed { failures }
                }
            }
            // The probe failed (or a stale failure arrived): open again for a full period.
            BreakerState::HalfOpen { .. } | BreakerState::Open { .. } => BreakerState::Open {
                until: now + self.cfg.open_for,
            },
        };
    }

    /// A request ended without telling us anything about provider health (cancelled, configuration
    /// error). If it was the probe, let another one try.
    pub fn record_neutral(&mut self) {
        if let BreakerState::HalfOpen { probing: true } = self.state {
            self.state = BreakerState::HalfOpen { probing: false };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn breaker() -> CircuitBreaker {
        CircuitBreaker::new(BreakerConfig {
            failures_to_open: 3,
            open_for: secs(30),
        })
    }

    #[test]
    fn it_opens_after_n_consecutive_failures_only() {
        let mut b = breaker();
        b.record_failure(secs(0));
        b.record_failure(secs(1));
        assert_eq!(b.permit(secs(2)), Permit::Yes);
        b.record_success(); // resets the streak
        b.record_failure(secs(3));
        b.record_failure(secs(4));
        assert_eq!(b.permit(secs(5)), Permit::Yes);
        b.record_failure(secs(6));
        assert_eq!(b.state(), BreakerState::Open { until: secs(36) });
        assert_eq!(b.permit(secs(10)), Permit::No { retry_in: secs(26) });
    }

    #[test]
    fn after_the_delay_one_probe_decides() {
        let mut b = breaker();
        for t in 0..3 {
            b.record_failure(secs(t));
        }
        assert_eq!(b.permit(secs(31)), Permit::No { retry_in: secs(1) });
        assert_eq!(b.permit(secs(32)), Permit::Probe);
        // While the probe is in flight, others wait.
        assert!(matches!(b.permit(secs(33)), Permit::No { .. }));
        b.record_success();
        assert_eq!(b.state(), BreakerState::Closed { failures: 0 });
        assert_eq!(b.permit(secs(34)), Permit::Yes);
    }

    #[test]
    fn a_failed_probe_reopens_for_a_full_period() {
        let mut b = breaker();
        for t in 0..3 {
            b.record_failure(secs(t));
        }
        assert_eq!(b.permit(secs(40)), Permit::Probe);
        b.record_failure(secs(41));
        assert_eq!(b.state(), BreakerState::Open { until: secs(71) });
        assert_eq!(b.permit(secs(50)), Permit::No { retry_in: secs(21) });
    }

    #[test]
    fn a_neutral_probe_outcome_lets_another_probe_go() {
        let mut b = breaker();
        for t in 0..3 {
            b.record_failure(secs(t));
        }
        assert_eq!(b.permit(secs(40)), Permit::Probe);
        b.record_neutral();
        assert_eq!(b.permit(secs(41)), Permit::Probe);
        // Neutral outcomes while closed change nothing.
        let mut c = breaker();
        c.record_failure(secs(0));
        c.record_neutral();
        assert_eq!(c.state(), BreakerState::Closed { failures: 1 });
    }

    #[test]
    fn every_state_transition_keeps_the_machine_valid() {
        // Drive random-ish sequences and check the invariants the client relies on.
        let mut b = breaker();
        let mut now = 0u64;
        for step in 0..500u64 {
            now += (step * 7) % 13;
            match step % 5 {
                0 => {
                    let _ = b.permit(secs(now));
                }
                1 => b.record_failure(secs(now)),
                2 => b.record_success(),
                3 => b.record_neutral(),
                _ => {
                    let p = b.permit(secs(now));
                    if let Permit::No { retry_in } = p {
                        assert!(retry_in <= secs(30), "{retry_in:?}");
                    }
                }
            }
            if let BreakerState::Closed { failures } = b.state() {
                assert!(failures < 3);
            }
        }
    }
}
