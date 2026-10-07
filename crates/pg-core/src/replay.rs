//! Replay logs, deterministic replay, and divergence tools (Blueprint §5.3, §18.3, §20).
//!
//! A [`ReplayLog`] records the world's identity (name, seed text, system profile), the content it ran
//! with, the inputs that were applied, and the per-day state hashes (combined and per table) the original
//! run produced. [`replay`] rebuilds the world, feeds the inputs back and checks every hash. Logs are
//! integer-only canonical JSON, so they are the same bytes on every platform.
//!
//! When two runs disagree, [`diff_logs`] says where (first differing day and tables) from the recorded
//! data alone, and [`bisect`] re-runs both in lockstep to find the first differing *tick*.

use crate::canon::{Canon, CanonError, ToCanon};
use crate::hash::StateHash;
use crate::input::StampedInput;
use crate::sim::{DayHash, Sim};
use crate::time::ClockOverflow;
use crate::world::WorldState;
use pg_content::ContentSet;
use std::sync::Arc;

pub const REPLAY_FORMAT: &str = "playground-replay";
pub const REPLAY_VERSION: u32 = 2;

/// Which set of systems the run used.
pub const PROFILE_DEV: &str = "dev";

/// A pack a run used, as recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRefRecord {
    pub pack_id: String,
    pub version: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayHashRecord {
    pub day: u64,
    pub tick: u64,
    pub hash: String,
    /// Per-table hashes (hex), in table order.
    pub tables: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayLog {
    pub profile: String,
    pub world_name: String,
    pub seed_text: String,
    pub content: Vec<ContentRefRecord>,
    /// How many ticks the original run simulated.
    pub ticks: u64,
    pub inputs: Vec<StampedInput>,
    pub day_hashes: Vec<DayHashRecord>,
    pub final_hash: String,
    pub final_tables: Vec<(String, String)>,
}

fn table_records(tables: &[(&'static str, StateHash)]) -> Vec<(String, String)> {
    tables
        .iter()
        .map(|(n, h)| ((*n).to_owned(), h.to_hex()))
        .collect()
}

impl ReplayLog {
    /// Records a finished run. `sim` must have started from `WorldState::new(world_name, seed_text)`.
    pub fn record(sim: &Sim) -> ReplayLog {
        let w = sim.world();
        ReplayLog {
            profile: PROFILE_DEV.to_owned(),
            world_name: w.meta.name.clone(),
            seed_text: w.meta.seed_text.clone(),
            content: sim.content().map_or_else(Vec::new, |c| {
                c.refs()
                    .into_iter()
                    .map(|r| ContentRefRecord {
                        pack_id: r.pack_id.to_string(),
                        version: r.version.to_string(),
                        hash: r.hash,
                    })
                    .collect()
            }),
            ticks: w.clock.tick(),
            inputs: sim.applied_inputs().to_vec(),
            day_hashes: sim
                .day_hashes()
                .iter()
                .map(|d| DayHashRecord {
                    day: d.day,
                    tick: d.tick,
                    hash: d.hash.to_hex(),
                    tables: table_records(&d.tables),
                })
                .collect(),
            final_hash: w.state_hash().to_hex(),
            final_tables: table_records(&w.table_hashes()),
        }
    }
}

fn tables_canon(tables: &[(String, String)]) -> Canon {
    Canon::Map(
        tables
            .iter()
            .map(|(n, h)| (n.clone(), Canon::str(h.clone())))
            .collect(),
    )
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
            (
                "content",
                Canon::List(
                    self.content
                        .iter()
                        .map(|c| {
                            Canon::map([
                                ("pack_id", c.pack_id.to_canon()),
                                ("version", c.version.to_canon()),
                                ("hash", c.hash.to_canon()),
                            ])
                        })
                        .collect(),
                ),
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
                                ("tables", tables_canon(&d.tables)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("final_hash", self.final_hash.to_canon()),
            ("final_tables", tables_canon(&self.final_tables)),
        ])
    }
}

fn read_tables(c: &Canon, what: &str) -> Result<Vec<(String, String)>, CanonError> {
    match c {
        Canon::Map(m) => m
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.to_owned()))
                    .ok_or_else(|| CanonError(format!("{what}: table hashes must be text")))
            })
            .collect(),
        _ => Err(CanonError(format!("{what} must be an object"))),
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
        let content = c
            .field("content")?
            .as_list()
            .ok_or_else(|| CanonError::new("content must be a list"))?
            .iter()
            .map(|r| {
                Ok(ContentRefRecord {
                    pack_id: text(r.field("pack_id")?, "pack_id")?,
                    version: text(r.field("version")?, "version")?,
                    hash: text(r.field("hash")?, "hash")?,
                })
            })
            .collect::<Result<Vec<_>, CanonError>>()?;
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
                    tables: read_tables(d.field("tables")?, "tables")?,
                })
            })
            .collect::<Result<Vec<_>, CanonError>>()?;
        Ok(ReplayLog {
            profile: text(c.field("profile")?, "profile")?,
            world_name: text(world.field("name")?, "world name")?,
            seed_text: text(world.field("seed_text")?, "seed text")?,
            content,
            ticks: num(c.field("ticks")?, "ticks")?,
            inputs,
            day_hashes,
            final_hash: text(c.field("final_hash")?, "final_hash")?,
            final_tables: read_tables(c.field("final_tables")?, "final_tables")?,
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
    /// The tables whose hashes differ (when per-table hashes were recorded).
    pub tables: Vec<String>,
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
    /// The log used content but none was supplied.
    MissingContent,
    /// The supplied content is not the content the log was recorded with.
    ContentMismatch(String),
    Clock,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::UnknownProfile(p) => write!(f, "unknown system profile '{p}'"),
            ReplayError::BadInput(m) => write!(f, "log input rejected: {m}"),
            ReplayError::MissingContent => {
                f.write_str("the log was recorded with content packs, but none were supplied")
            }
            ReplayError::ContentMismatch(m) => write!(f, "content differs from the recording: {m}"),
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

fn check_content(log: &ReplayLog, content: Option<&ContentSet>) -> Result<(), ReplayError> {
    if log.content.is_empty() {
        return Ok(());
    }
    let content = content.ok_or(ReplayError::MissingContent)?;
    let have: Vec<ContentRefRecord> = content
        .refs()
        .into_iter()
        .map(|r| ContentRefRecord {
            pack_id: r.pack_id.to_string(),
            version: r.version.to_string(),
            hash: r.hash,
        })
        .collect();
    if have == log.content {
        return Ok(());
    }
    let mut diffs = Vec::new();
    for want in &log.content {
        match have.iter().find(|h| h.pack_id == want.pack_id) {
            None => diffs.push(format!("pack '{}' is missing", want.pack_id)),
            Some(h) if h.hash != want.hash => diffs.push(format!(
                "pack '{}' changed (recorded {} {}, have {} {})",
                want.pack_id,
                want.version,
                want.hash.chars().take(8).collect::<String>(),
                h.version,
                h.hash.chars().take(8).collect::<String>()
            )),
            Some(_) => {}
        }
    }
    for h in &have {
        if !log.content.iter().any(|w| w.pack_id == h.pack_id) {
            diffs.push(format!("pack '{}' was not in the recording", h.pack_id));
        }
    }
    Err(ReplayError::ContentMismatch(diffs.join("; ")))
}

/// Builds the sim a log was recorded from, with every input queued.
pub fn build_sim(log: &ReplayLog, content: Option<Arc<ContentSet>>) -> Result<Sim, ReplayError> {
    if log.profile != PROFILE_DEV {
        return Err(ReplayError::UnknownProfile(log.profile.clone()));
    }
    check_content(log, content.as_deref())?;
    let mut sim = Sim::with_dev_systems(WorldState::new(
        log.world_name.clone(),
        log.seed_text.clone(),
    ));
    if let Some(c) = content {
        sim = sim.with_content(c);
    }
    // Inputs are logged in application order, so resubmitting in that order preserves tie-breaks.
    for stamped in &log.inputs {
        sim.submit(stamped.tick, stamped.input.clone())
            .map_err(|e| ReplayError::BadInput(e.to_string()))?;
    }
    Ok(sim)
}

fn differing_tables(a: &[(String, String)], b: &[(&'static str, StateHash)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, hex) in a {
        match b.iter().find(|(n, _)| n == name) {
            Some((_, h)) if &h.to_hex() == hex => {}
            _ => out.push(name.clone()),
        }
    }
    out
}

/// Re-runs a log and compares every recorded hash.
pub fn replay(
    log: &ReplayLog,
    content: Option<Arc<ContentSet>>,
) -> Result<ReplayOutcome, ReplayError> {
    let mut sim = build_sim(log, content)?;
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
                tables: differing_tables(&expected.tables, &d.tables),
            }),
            None => mismatches.push(Mismatch {
                day: Some(expected.day),
                expected: expected.hash.clone(),
                actual: "<missing>".into(),
                tables: Vec::new(),
            }),
        }
    }
    if produced.len() > log.day_hashes.len() {
        mismatches.push(Mismatch {
            day: produced.get(log.day_hashes.len()).map(|d| d.day),
            expected: "<no more day hashes>".into(),
            actual: "<extra day hash>".into(),
            tables: Vec::new(),
        });
    }
    if final_hash.to_hex() != log.final_hash {
        mismatches.push(Mismatch {
            day: None,
            expected: log.final_hash.clone(),
            actual: final_hash.to_hex(),
            tables: differing_tables(&log.final_tables, &sim.world().table_hashes()),
        });
    }
    Ok(ReplayOutcome {
        day_hashes: produced.to_vec(),
        final_hash,
        mismatches,
    })
}

/// Where two logs first disagree, worked out from the recorded data alone (no re-running).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogDiff {
    pub header_differences: Vec<String>,
    /// Index of the first input that differs (or where one list ends early).
    pub first_input_difference: Option<usize>,
    /// The first recorded day whose hash differs, with the tables that differ.
    pub first_day_mismatch: Option<DayMismatch>,
    /// Tables whose final hashes differ.
    pub final_tables: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayMismatch {
    pub day: u64,
    pub tick: u64,
    pub tables: Vec<String>,
}

impl LogDiff {
    pub fn identical(&self) -> bool {
        self.header_differences.is_empty()
            && self.first_input_difference.is_none()
            && self.first_day_mismatch.is_none()
            && self.final_tables.is_empty()
    }
}

fn tables_that_differ(a: &[(String, String)], b: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, hex) in a {
        if b.iter().find(|(n, _)| n == name).map(|(_, h)| h) != Some(hex) {
            out.push(name.clone());
        }
    }
    for (name, _) in b {
        if !a.iter().any(|(n, _)| n == name) {
            out.push(name.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn diff_logs(a: &ReplayLog, b: &ReplayLog) -> LogDiff {
    let mut header = Vec::new();
    if a.profile != b.profile {
        header.push(format!("profile: {} vs {}", a.profile, b.profile));
    }
    if a.world_name != b.world_name {
        header.push(format!(
            "world name: {:?} vs {:?}",
            a.world_name, b.world_name
        ));
    }
    if a.seed_text != b.seed_text {
        header.push(format!("seed: {:?} vs {:?}", a.seed_text, b.seed_text));
    }
    if a.ticks != b.ticks {
        header.push(format!("ticks: {} vs {}", a.ticks, b.ticks));
    }
    if a.content != b.content {
        header.push("content packs or their hashes differ".to_owned());
    }
    let first_input_difference =
        (0..a.inputs.len().max(b.inputs.len())).find(|&i| a.inputs.get(i) != b.inputs.get(i));
    let first_day_mismatch = a
        .day_hashes
        .iter()
        .zip(&b.day_hashes)
        .find(|(x, y)| x.hash != y.hash)
        .map(|(x, y)| DayMismatch {
            day: x.day,
            tick: x.tick,
            tables: tables_that_differ(&x.tables, &y.tables),
        });
    LogDiff {
        header_differences: header,
        first_input_difference,
        first_day_mismatch,
        final_tables: tables_that_differ(&a.final_tables, &b.final_tables),
    }
}

/// The result of running two logs in lockstep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BisectResult {
    /// The tick whose step first made the two worlds differ (the clock value *before* that step), or
    /// `None` if they never differ within the shorter run.
    pub first_diff_tick: Option<u64>,
    pub tables: Vec<String>,
    /// The inputs each run applied on that tick.
    pub applied_a: Vec<StampedInput>,
    pub applied_b: Vec<StampedInput>,
}

/// Runs both logs tick by tick and reports the first tick at which their per-table hashes differ.
/// To avoid hashing every tick, both runs fast-forward to the last day where the recorded hashes still
/// agreed before comparing.
pub fn bisect(
    a: &ReplayLog,
    b: &ReplayLog,
    content: Option<Arc<ContentSet>>,
) -> Result<BisectResult, ReplayError> {
    let mut sa = build_sim(a, content.clone())?;
    let mut sb = build_sim(b, content)?;
    let limit = a.ticks.min(b.ticks);
    let start = diff_logs(a, b)
        .first_day_mismatch
        .map_or(0, |m| m.tick.saturating_sub(crate::time::TICKS_PER_DAY));
    let start = start.min(limit);
    sa.run_ticks(start)?;
    sb.run_ticks(start)?;
    for _ in start..limit {
        let (before_a, before_b) = (sa.applied_inputs().len(), sb.applied_inputs().len());
        let tick = sa.world().clock.tick();
        sa.step()?;
        sb.step()?;
        let (ha, hb) = (sa.world().table_hashes(), sb.world().table_hashes());
        if ha != hb {
            let tables = ha
                .iter()
                .zip(&hb)
                .filter(|(x, y)| x.1 != y.1)
                .map(|(x, _)| x.0.to_owned())
                .collect();
            return Ok(BisectResult {
                first_diff_tick: Some(tick),
                tables,
                applied_a: sa.applied_inputs().get(before_a..).unwrap_or(&[]).to_vec(),
                applied_b: sb.applied_inputs().get(before_b..).unwrap_or(&[]).to_vec(),
            });
        }
    }
    Ok(BisectResult {
        first_diff_tick: None,
        tables: Vec::new(),
        applied_a: Vec::new(),
        applied_b: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{EntityId, Kind};
    use crate::input::{Command, SettingChange, SimInput};
    use crate::map::Tile;
    use crate::time::TICKS_PER_DAY;
    use pg_content::{load_pack, ComponentRegistry, Limits, MemoryPack};

    fn cmd(c: Command) -> SimInput {
        SimInput::Command {
            actor: None,
            cmd: c,
        }
    }

    fn recorded_with(days: u64, first_nudge: i32) -> ReplayLog {
        let mut sim = Sim::with_dev_systems(WorldState::new("Replay Town", "replay-seed"));
        sim.submit(
            50,
            cmd(Command::DevNudge {
                amount: first_nudge,
            }),
        )
        .unwrap();
        sim.submit(
            9_000,
            SimInput::SettingChange(SettingChange::SlotMinutes(60)),
        )
        .unwrap();
        sim.submit(20_000, cmd(Command::DevNudge { amount: -77 }))
            .unwrap();
        sim.run_ticks(days * TICKS_PER_DAY).unwrap();
        ReplayLog::record(&sim)
    }

    fn recorded(days: u64) -> ReplayLog {
        recorded_with(days, 123)
    }

    fn content() -> Arc<ContentSet> {
        let pack = MemoryPack::new()
            .with("pack.json", r#"{"id":"base","name":"Base","version":"0.1.0"}"#)
            .with(
                "data/templates/t.json",
                r#"[{"id":"base.object","schema":1},{"id":"item.coin","schema":1,"extends":"base.object"}]"#,
            );
        Arc::new(
            ContentSet::build(
                vec![load_pack(&pack, &Limits::default()).unwrap()],
                ComponentRegistry::builtin(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_recorded_run_replays_identically() {
        let log = recorded(3);
        assert_eq!(log.day_hashes.len(), 3);
        assert_eq!(log.inputs.len(), 3);
        assert!(
            log.day_hashes.iter().all(|d| d.tables.len() == 10),
            "per-table hashes are recorded"
        );
        let out = replay(&log, None).unwrap();
        assert!(out.ok(), "{:?}", out.mismatches);
    }

    #[test]
    fn the_log_survives_serialization() {
        let log = recorded(2);
        let text = log.to_canon().to_canonical_string();
        assert!(!text.contains('.'), "no floats can appear in a log");
        let back = ReplayLog::from_canon(&crate::canon::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back, log);
        assert!(replay(&back, None).unwrap().ok());
    }

    #[test]
    fn tampering_is_detected_and_localized_to_a_table() {
        let mut log = recorded(3);
        if let Some(first) = log.inputs.first_mut() {
            first.input = cmd(Command::DevNudge { amount: 124 });
        }
        let out = replay(&log, None).unwrap();
        assert!(!out.ok());
        let day1 = out.mismatches.iter().find(|m| m.day == Some(1)).unwrap();
        assert_eq!(
            day1.tables,
            vec!["probe".to_owned()],
            "only the probe table changed"
        );
        assert!(
            out.mismatches.iter().any(|m| m.day.is_none()),
            "final hash must mismatch too"
        );

        let mut log = recorded(3);
        if let Some(d) = log.day_hashes.get_mut(1) {
            d.hash = "00".repeat(32);
        }
        let out = replay(&log, None).unwrap();
        let days: Vec<_> = out.mismatches.iter().map(|m| m.day).collect();
        assert_eq!(days, vec![Some(2)], "only the corrupted day is reported");
    }

    #[test]
    fn a_wrong_seed_or_profile_is_not_silently_accepted() {
        let mut log = recorded(1);
        log.seed_text = "other".into();
        assert!(!replay(&log, None).unwrap().ok());
        let mut log = recorded(1);
        log.profile = "mystery".into();
        assert_eq!(
            replay(&log, None),
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
        let mut v1 = recorded(1).to_canon();
        if let Canon::Map(m) = &mut v1 {
            m.insert("version".into(), Canon::Int(1));
        }
        assert!(
            ReplayLog::from_canon(&v1).is_err(),
            "old log formats are refused, not guessed at"
        );
    }

    #[test]
    fn logs_with_a_map_and_pawns_replay_identically() {
        let map = EntityId::new(Kind::Map, 1);
        let mut sim = Sim::with_dev_systems(WorldState::new("Town", "pawns"));
        sim.submit(
            0,
            cmd(Command::DevCreateMap {
                w: 30,
                h: 24,
                style: 1,
            }),
        )
        .unwrap();
        for i in 0..6 {
            sim.submit(
                0,
                cmd(Command::DevSpawnPawn {
                    map,
                    at: None,
                    name: format!("P{i}"),
                }),
            )
            .unwrap();
        }
        sim.submit(
            500,
            cmd(Command::DevSetBlocked {
                map,
                at: Tile::new(5, 5),
                blocked: true,
            }),
        )
        .unwrap();
        sim.run_ticks(TICKS_PER_DAY / 2).unwrap();
        let log = ReplayLog::record(&sim);
        assert_eq!(log.inputs.len(), 8);
        assert!(replay(&log, None).unwrap().ok());
    }

    #[test]
    fn content_is_recorded_and_checked() {
        let c = content();
        let mut sim =
            Sim::with_dev_systems(WorldState::new("Town", "stuff")).with_content(Arc::clone(&c));
        sim.submit(
            0,
            cmd(Command::DevCreateMap {
                w: 10,
                h: 10,
                style: 0,
            }),
        )
        .unwrap();
        sim.submit(
            1,
            cmd(Command::DevSpawnObject {
                map: EntityId::new(Kind::Map, 1),
                at: Tile::new(2, 2),
                template: "item.coin".into(),
            }),
        )
        .unwrap();
        sim.run_ticks(100).unwrap();
        let log = ReplayLog::record(&sim);
        assert_eq!(log.content.len(), 1);
        assert_eq!(sim.world().objects.len(), 1);

        assert!(replay(&log, Some(Arc::clone(&c))).unwrap().ok());
        assert_eq!(replay(&log, None), Err(ReplayError::MissingContent));

        // Different content (a changed template) must be refused with a reason, not mis-replayed.
        let other_pack = MemoryPack::new()
            .with("pack.json", r#"{"id":"base","name":"Base","version":"0.1.0"}"#)
            .with(
                "data/templates/t.json",
                r#"[{"id":"base.object","schema":1},{"id":"item.coin","schema":1,"extends":"base.object","tags":["shiny"]}]"#,
            );
        let other = Arc::new(
            ContentSet::build(
                vec![load_pack(&other_pack, &Limits::default()).unwrap()],
                ComponentRegistry::builtin(),
            )
            .unwrap(),
        );
        match replay(&log, Some(other)) {
            Err(ReplayError::ContentMismatch(msg)) => {
                assert!(msg.contains("pack 'base' changed"), "{msg}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn diff_finds_the_first_differing_input_day_and_table() {
        let a = recorded(3);
        assert!(diff_logs(&a, &a.clone()).identical());
        let b = recorded_with(3, 124);
        let d = diff_logs(&a, &b);
        assert!(!d.identical());
        assert_eq!(d.first_input_difference, Some(0));
        let m = d.first_day_mismatch.unwrap();
        assert_eq!((m.day, m.tick), (1, TICKS_PER_DAY));
        assert_eq!(m.tables, vec!["probe".to_owned()]);
        assert_eq!(d.final_tables, vec!["probe".to_owned()]);
        assert!(d.header_differences.is_empty());
    }

    #[test]
    fn diff_reports_header_differences() {
        let a = recorded(2);
        let mut b = a.clone();
        b.seed_text = "other".into();
        b.ticks += 1;
        let d = diff_logs(&a, &b);
        assert_eq!(d.header_differences.len(), 2, "{:?}", d.header_differences);
    }

    #[test]
    fn bisect_finds_the_exact_tick_where_two_runs_part() {
        // Identical until the nudge at tick 20_000 differs.
        let make = |last: i32| {
            let mut sim = Sim::with_dev_systems(WorldState::new("Town", "bisect"));
            sim.submit(50, cmd(Command::DevNudge { amount: 5 }))
                .unwrap();
            sim.submit(20_000, cmd(Command::DevNudge { amount: last }))
                .unwrap();
            sim.run_ticks(2 * TICKS_PER_DAY).unwrap();
            ReplayLog::record(&sim)
        };
        let (a, b) = (make(10), make(11));
        let r = bisect(&a, &b, None).unwrap();
        assert_eq!(r.first_diff_tick, Some(20_000));
        assert_eq!(r.tables, vec!["probe".to_owned()]);
        assert_eq!(r.applied_a.len(), 1);
        assert_eq!(r.applied_b.len(), 1);
        assert_ne!(r.applied_a, r.applied_b);
    }

    #[test]
    fn bisect_of_identical_logs_finds_nothing() {
        let a = recorded(2);
        let r = bisect(&a, &a.clone(), None).unwrap();
        assert_eq!(r.first_diff_tick, None);
    }

    #[test]
    fn bisect_catches_a_difference_on_the_very_first_tick() {
        let make = |seed: &str| {
            let mut sim = Sim::with_dev_systems(WorldState::new("Town", seed));
            sim.submit(
                0,
                cmd(Command::DevCreateMap {
                    w: 12,
                    h: 12,
                    style: 1,
                }),
            )
            .unwrap();
            sim.run_ticks(100).unwrap();
            ReplayLog::record(&sim)
        };
        let r = bisect(&make("one"), &make("two"), None).unwrap();
        assert_eq!(r.first_diff_tick, Some(0));
        assert!(
            r.tables.contains(&"meta".to_owned()),
            "the seed text is in the meta table: {:?}",
            r.tables
        );
    }
}
