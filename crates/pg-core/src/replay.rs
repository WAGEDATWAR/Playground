//! Replay logs and deterministic replay (Blueprint §5.3, §18.3).
//!
//! A [`ReplayLog`] records the world's identity (name, seed text, system profile), the inputs that
//! were applied, and the per-day state hashes the original run produced. [`replay`] rebuilds the world,
//! feeds the inputs back and checks every hash. Logs are integer-only canonical JSON, so they are the
//! same bytes on every platform.

use crate::canon::{Canon, CanonError, ToCanon};
use crate::hash::StateHash;
use crate::input::StampedInput;
use crate::sim::{DayHash, Sim};
use crate::time::ClockOverflow;
use crate::world::WorldState;

pub const REPLAY_FORMAT: &str = "playground-replay";
pub const REPLAY_VERSION: u32 = 1;

/// Which set of systems the run used. Real content references replace this in later milestones.
pub const PROFILE_DEV: &str = "dev";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayHashRecord {
    pub day: u64,
    pub tick: u64,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayLog {
    pub profile: String,
    pub world_name: String,
    pub seed_text: String,
    /// How many ticks the original run simulated.
    pub ticks: u64,
    pub inputs: Vec<StampedInput>,
    pub day_hashes: Vec<DayHashRecord>,
    pub final_hash: String,
}

impl ReplayLog {
    /// Records a finished run. `sim` must have started from `WorldState::new(world_name, seed_text)`.
    pub fn record(sim: &Sim) -> ReplayLog {
        let w = sim.world();
        ReplayLog {
            profile: PROFILE_DEV.to_owned(),
            world_name: w.meta.name.clone(),
            seed_text: w.meta.seed_text.clone(),
            ticks: w.clock.tick(),
            inputs: sim.applied_inputs().to_vec(),
            day_hashes: sim
                .day_hashes()
                .iter()
                .map(|d| DayHashRecord {
                    day: d.day,
                    tick: d.tick,
                    hash: d.hash.to_hex(),
                })
                .collect(),
            final_hash: w.state_hash().to_hex(),
        }
    }
}

impl ToCanon for ReplayLog {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("format", Canon::str(REPLAY_FORMAT)),
            ("version", REPLAY_VERSION.to_canon()),
            ("profile", self.profile.to_canon()),
            (
                "world",
                Canon::map([
                    ("name", self.world_name.to_canon()),
                    ("seed_text", self.seed_text.to_canon()),
                ]),
            ),
            ("ticks", self.ticks.to_canon()),
            (
                "inputs",
                Canon::List(self.inputs.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "day_hashes",
                Canon::List(
                    self.day_hashes
                        .iter()
                        .map(|d| {
                            Canon::map([
                                ("day", d.day.to_canon()),
                                ("tick", d.tick.to_canon()),
                                ("hash", d.hash.to_canon()),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("final_hash", self.final_hash.to_canon()),
        ])
    }
}

impl ReplayLog {
    pub fn from_canon(c: &Canon) -> Result<ReplayLog, CanonError> {
        let text = |v: &Canon, what: &str| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| CanonError(format!("{what} must be text")))
        };
        let num = |v: &Canon, what: &str| {
            v.as_u64()
                .ok_or_else(|| CanonError(format!("{what} must be a non-negative integer")))
        };
        if c.field("format")?.as_str() != Some(REPLAY_FORMAT) {
            return Err(CanonError::new("not a playground replay log"));
        }
        let version = num(c.field("version")?, "version")?;
        if version != u64::from(REPLAY_VERSION) {
            return Err(CanonError(format!(
                "unsupported replay version {version} (this build reads {REPLAY_VERSION})"
            )));
        }
        let world = c.field("world")?;
        let inputs = c
            .field("inputs")?
            .as_list()
            .ok_or_else(|| CanonError::new("inputs must be a list"))?
            .iter()
            .map(StampedInput::from_canon)
            .collect::<Result<Vec<_>, _>>()?;
        let day_hashes = c
            .field("day_hashes")?
            .as_list()
            .ok_or_else(|| CanonError::new("day_hashes must be a list"))?
            .iter()
            .map(|d| {
                Ok(DayHashRecord {
                    day: num(d.field("day")?, "day")?,
                    tick: num(d.field("tick")?, "tick")?,
                    hash: text(d.field("hash")?, "hash")?,
                })
            })
            .collect::<Result<Vec<_>, CanonError>>()?;
        Ok(ReplayLog {
            profile: text(c.field("profile")?, "profile")?,
            world_name: text(world.field("name")?, "world name")?,
            seed_text: text(world.field("seed_text")?, "seed text")?,
            ticks: num(c.field("ticks")?, "ticks")?,
            inputs,
            day_hashes,
            final_hash: text(c.field("final_hash")?, "final_hash")?,
        })
    }
}

/// A hash that did not match the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// `None` for the final hash.
    pub day: Option<u64>,
    pub expected: String,
    pub actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub day_hashes: Vec<DayHash>,
    pub final_hash: StateHash,
    pub mismatches: Vec<Mismatch>,
}

impl ReplayOutcome {
    pub fn ok(&self) -> bool {
        self.mismatches.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    UnknownProfile(String),
    BadInput(String),
    Clock,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::UnknownProfile(p) => write!(f, "unknown system profile '{p}'"),
            ReplayError::BadInput(m) => write!(f, "log input rejected: {m}"),
            ReplayError::Clock => f.write_str("simulation clock overflowed"),
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<ClockOverflow> for ReplayError {
    fn from(_: ClockOverflow) -> Self {
        ReplayError::Clock
    }
}

/// Re-runs a log and compares every recorded hash.
pub fn replay(log: &ReplayLog) -> Result<ReplayOutcome, ReplayError> {
    if log.profile != PROFILE_DEV {
        return Err(ReplayError::UnknownProfile(log.profile.clone()));
    }
    let mut sim = Sim::with_dev_systems(WorldState::new(
        log.world_name.clone(),
        log.seed_text.clone(),
    ));
    // Inputs are logged in application order, so resubmitting in that order preserves tie-breaks.
    for stamped in &log.inputs {
        sim.submit(stamped.tick, stamped.input.clone())
            .map_err(|e| ReplayError::BadInput(e.to_string()))?;
    }
    sim.run_ticks(log.ticks)?;

    let final_hash = sim.world().state_hash();
    let mut mismatches = Vec::new();
    let produced = sim.day_hashes();
    for (i, expected) in log.day_hashes.iter().enumerate() {
        match produced.get(i) {
            Some(d)
                if d.day == expected.day
                    && d.tick == expected.tick
                    && d.hash.to_hex() == expected.hash => {}
            Some(d) => mismatches.push(Mismatch {
                day: Some(expected.day),
                expected: expected.hash.clone(),
                actual: d.hash.to_hex(),
            }),
            None => mismatches.push(Mismatch {
                day: Some(expected.day),
                expected: expected.hash.clone(),
                actual: "<missing>".into(),
            }),
        }
    }
    if produced.len() > log.day_hashes.len() {
        mismatches.push(Mismatch {
            day: produced.get(log.day_hashes.len()).map(|d| d.day),
            expected: "<no more day hashes>".into(),
            actual: "<extra day hash>".into(),
        });
    }
    if final_hash.to_hex() != log.final_hash {
        mismatches.push(Mismatch {
            day: None,
            expected: log.final_hash.clone(),
            actual: final_hash.to_hex(),
        });
    }
    Ok(ReplayOutcome {
        day_hashes: produced.to_vec(),
        final_hash,
        mismatches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Command, SettingChange, SimInput};
    use crate::time::TICKS_PER_DAY;

    fn recorded(days: u64) -> ReplayLog {
        let mut sim = Sim::with_dev_systems(WorldState::new("Replay Town", "replay-seed"));
        sim.submit(
            50,
            SimInput::Command {
                actor: None,
                cmd: Command::DevNudge { amount: 123 },
            },
        )
        .unwrap();
        sim.submit(
            9_000,
            SimInput::SettingChange(SettingChange::SlotMinutes(60)),
        )
        .unwrap();
        sim.submit(
            20_000,
            SimInput::Command {
                actor: None,
                cmd: Command::DevNudge { amount: -77 },
            },
        )
        .unwrap();
        sim.run_ticks(days * TICKS_PER_DAY).unwrap();
        ReplayLog::record(&sim)
    }

    #[test]
    fn a_recorded_run_replays_identically() {
        let log = recorded(3);
        assert_eq!(log.day_hashes.len(), 3);
        assert_eq!(log.inputs.len(), 3);
        let out = replay(&log).unwrap();
        assert!(out.ok(), "{:?}", out.mismatches);
    }

    #[test]
    fn the_log_survives_serialization() {
        let log = recorded(2);
        let text = log.to_canon().to_canonical_string();
        assert!(!text.contains('.'), "no floats can appear in a log");
        // Decode through the canonical tree again (the CLI does this via JSON text).
        let back = ReplayLog::from_canon(&log.to_canon()).unwrap();
        assert_eq!(back, log);
        assert!(replay(&back).unwrap().ok());
    }

    #[test]
    fn tampering_is_detected_and_localized() {
        let mut log = recorded(3);
        // Change an input: every later day hash must stop matching.
        if let Some(first) = log.inputs.first_mut() {
            first.input = SimInput::Command {
                actor: None,
                cmd: Command::DevNudge { amount: 124 },
            };
        }
        let out = replay(&log).unwrap();
        assert!(!out.ok());
        assert!(
            out.mismatches.iter().any(|m| m.day == Some(1)),
            "{:?}",
            out.mismatches
        );
        assert!(
            out.mismatches.iter().any(|m| m.day.is_none()),
            "final hash must mismatch too"
        );

        let mut log = recorded(3);
        if let Some(d) = log.day_hashes.get_mut(1) {
            d.hash = "00".repeat(32);
        }
        let out = replay(&log).unwrap();
        let days: Vec<_> = out.mismatches.iter().map(|m| m.day).collect();
        assert_eq!(days, vec![Some(2)], "only the corrupted day is reported");
    }

    #[test]
    fn a_wrong_seed_or_profile_is_not_silently_accepted() {
        let mut log = recorded(1);
        log.seed_text = "other".into();
        assert!(!replay(&log).unwrap().ok());
        let mut log = recorded(1);
        log.profile = "mystery".into();
        assert_eq!(
            replay(&log),
            Err(ReplayError::UnknownProfile("mystery".into()))
        );
    }

    #[test]
    fn decoding_rejects_foreign_or_future_logs() {
        let mut c = recorded(1).to_canon();
        if let Canon::Map(m) = &mut c {
            m.insert("version".into(), Canon::Int(99));
        }
        assert!(ReplayLog::from_canon(&c).is_err());
        assert!(ReplayLog::from_canon(&Canon::map([("format", Canon::str("other"))])).is_err());
    }
}
