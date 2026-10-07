//! Developer commands for the simulation driver (milestone 0.2): `sim`, `replay`, `time`, `pipeline`.

use pg_core::canon::{Canon, ToCanon};
use pg_core::dev;
use pg_core::input::{Command, SettingChange, SimInput};
use pg_core::pipeline::{Cadence, Pipeline};
use pg_core::replay::{replay, ReplayLog};
use pg_core::sim::{Sim, SimSnapshot};
use pg_core::time::{flags_for, SlotMinutes, TICKS_PER_DAY};
use pg_core::world::WorldState;
use std::process::ExitCode;

fn value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Parses `<tick>:<number>`.
fn tick_pair(text: &str, flag: &str) -> Result<(u64, i64), String> {
    let (a, b) = text
        .split_once(':')
        .ok_or_else(|| format!("{flag} expects <tick>:<value>, got '{text}'"))?;
    let tick = a.parse().map_err(|e| format!("{flag} tick '{a}': {e}"))?;
    let val = b.parse().map_err(|e| format!("{flag} value '{b}': {e}"))?;
    Ok((tick, val))
}

fn cadence_name(c: Cadence) -> &'static str {
    match c {
        Cadence::Tick => "tick",
        Cadence::Minute => "minute",
        Cadence::Slot => "slot",
        Cadence::Day => "day",
    }
}

fn short(h: &pg_core::hash::StateHash) -> String {
    h.to_hex().chars().take(8).collect()
}

fn write_log(path: &str, log: &ReplayLog, pretty: bool) -> Result<(), String> {
    let canonical = log.to_canon().to_canonical_string();
    let text = if pretty {
        let v: serde_json::Value = serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
    } else {
        canonical
    };
    std::fs::write(path, text + "\n").map_err(|e| format!("cannot write {path}: {e}"))
}

pub fn sim_cmd(args: &[String]) -> Result<ExitCode, String> {
    let (mut seed, mut name) = (None::<String>, "Dev Town".to_owned());
    let (mut days, mut ticks) = (None::<u64>, None::<u64>);
    let (mut nudges, mut slots) = (Vec::new(), Vec::new());
    let (mut log_path, mut pretty, mut show_events) = (None::<String>, false, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => seed = Some(value(args, &mut i, "--seed")?.to_owned()),
            "--name" => name = value(args, &mut i, "--name")?.to_owned(),
            "--days" => {
                days = Some(
                    value(args, &mut i, "--days")?
                        .parse()
                        .map_err(|e| format!("--days: {e}"))?,
                )
            }
            "--ticks" => {
                ticks = Some(
                    value(args, &mut i, "--ticks")?
                        .parse()
                        .map_err(|e| format!("--ticks: {e}"))?,
                )
            }
            "--nudge" => nudges.push(tick_pair(value(args, &mut i, "--nudge")?, "--nudge")?),
            "--slot" => slots.push(tick_pair(value(args, &mut i, "--slot")?, "--slot")?),
            "--log" => log_path = Some(value(args, &mut i, "--log")?.to_owned()),
            "--pretty" => pretty = true,
            "--events" => show_events = true,
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    let seed = seed.ok_or("sim needs --seed <text>")?;
    let total = match (days, ticks) {
        (Some(d), None) => d.checked_mul(TICKS_PER_DAY).ok_or("--days is too large")?,
        (None, Some(t)) => t,
        (None, None) => 3 * TICKS_PER_DAY,
        (Some(_), Some(_)) => return Err("use either --days or --ticks, not both".into()),
    };

    let mut sim = Sim::with_dev_systems(WorldState::new(name.clone(), seed.clone()));
    for (tick, amount) in nudges {
        let amount = i32::try_from(amount)
            .map_err(|_| format!("--nudge amount {amount} does not fit i32"))?;
        sim.submit(
            tick,
            SimInput::Command {
                actor: None,
                cmd: Command::DevNudge { amount },
            },
        )
        .map_err(|e| e.to_string())?;
    }
    for (tick, minutes) in slots {
        let minutes =
            u32::try_from(minutes).map_err(|_| format!("--slot minutes {minutes} is invalid"))?;
        sim.submit(
            tick,
            SimInput::SettingChange(SettingChange::SlotMinutes(minutes)),
        )
        .map_err(|e| e.to_string())?;
    }

    println!(
        "world {name:?}  seed {seed:?}  ticks {total} ({:.2} days)",
        total as f64 / TICKS_PER_DAY as f64
    );
    let mut event_count = 0usize;
    for _ in 0..total {
        let report = sim.step().map_err(|e| e.to_string())?;
        event_count += report.events.len();
        if show_events {
            for e in &report.events {
                println!(
                    "  event @{:>7}  {:<16} {}",
                    e.tick,
                    e.kind,
                    e.detail.to_canonical_string()
                );
            }
        }
        if let Some(h) = report.day_hash {
            let day = sim.day_hashes().last();
            let tables = day
                .map(|d| {
                    d.tables
                        .iter()
                        .map(|(n, th)| format!("{n}:{}", &short(th)[..4]))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            println!(
                "day {:>3} @tick {:>8}  {}  [{tables}]",
                report.flags.day,
                report.tick,
                short(&h)
            );
        }
    }
    let w = sim.world();
    println!(
        "done: tick {}  day {}  probe(value={}, minutes={}, days={})  slot={}min  events={event_count}",
        w.clock.tick(),
        w.clock.day(),
        w.probe.value,
        w.probe.minutes,
        w.probe.days,
        w.settings.slot_minutes.get()
    );
    println!("final hash {}", w.state_hash());
    if let Some(path) = log_path {
        write_log(&path, &ReplayLog::record(&sim), pretty)?;
        println!("replay log written to {path}");
    }
    Ok(ExitCode::SUCCESS)
}

fn dev_pipeline() -> Pipeline {
    let mut p = Pipeline::new();
    dev::install(&mut p);
    p
}

/// Splits the run at `at`, snapshots, resumes, and compares with the uninterrupted run.
fn snapshot_equivalence(log: &ReplayLog, at: u64) -> Result<bool, String> {
    let build = || -> Result<Sim, String> {
        let mut s = Sim::with_dev_systems(WorldState::new(
            log.world_name.clone(),
            log.seed_text.clone(),
        ));
        for st in &log.inputs {
            s.submit(st.tick, st.input.clone())
                .map_err(|e| e.to_string())?;
        }
        Ok(s)
    };
    let mut whole = build()?;
    whole.run_ticks(log.ticks).map_err(|e| e.to_string())?;
    let mut first = build()?;
    first
        .run_ticks(at.min(log.ticks))
        .map_err(|e| e.to_string())?;
    let snap: SimSnapshot = first.snapshot();
    let pending = snap.pending.len();
    let mut resumed = Sim::restore(snap, dev_pipeline());
    resumed
        .run_ticks(log.ticks - at.min(log.ticks))
        .map_err(|e| e.to_string())?;
    let same = resumed.world().state_hash() == whole.world().state_hash();
    println!(
        "snapshot at tick {at} ({pending} input(s) pending): resumed {} uninterrupted ({} vs {})",
        if same { "==" } else { "!=" },
        short(&resumed.world().state_hash()),
        short(&whole.world().state_hash())
    );
    Ok(same)
}

pub fn replay_cmd(args: &[String]) -> Result<ExitCode, String> {
    let path = args.first().ok_or("replay needs <log.json>")?;
    let mut snapshot_at = None::<u64>;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--snapshot-at" => {
                snapshot_at = Some(
                    value(args, &mut i, "--snapshot-at")?
                        .parse()
                        .map_err(|e| format!("--snapshot-at: {e}"))?,
                )
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let canon: Canon = pg_core::canon::json::parse(&text).map_err(|e| format!("{path}: {e}"))?;
    let log = ReplayLog::from_canon(&canon).map_err(|e| format!("{path}: {e}"))?;
    println!(
        "{path}: world {:?} seed {:?}, {} ticks, {} input(s), {} day hash(es)",
        log.world_name,
        log.seed_text,
        log.ticks,
        log.inputs.len(),
        log.day_hashes.len()
    );
    let outcome = replay(&log).map_err(|e| e.to_string())?;
    for d in &outcome.day_hashes {
        let bad = outcome.mismatches.iter().any(|m| m.day == Some(d.day));
        println!(
            "  day {:>3}  {}  {}",
            d.day,
            short(&d.hash),
            if bad { "MISMATCH" } else { "ok" }
        );
    }
    for m in &outcome.mismatches {
        println!(
            "  MISMATCH {:?}: expected {}  actual {}",
            m.day, m.expected, m.actual
        );
    }
    let mut ok = outcome.ok();
    if let Some(at) = snapshot_at {
        ok &= snapshot_equivalence(&log, at)?;
    }
    println!("{}", if ok { "REPLAY OK" } else { "REPLAY FAILED" });
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub fn time_cmd(args: &[String]) -> Result<ExitCode, String> {
    let tick: u64 = args
        .first()
        .ok_or("time needs <tick>")?
        .parse()
        .map_err(|e| format!("tick: {e}"))?;
    let mut minutes = pg_core::time::DEFAULT_SLOT_MINUTES;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--slot-minutes" => {
                minutes = value(args, &mut i, "--slot-minutes")?
                    .parse()
                    .map_err(|e| format!("--slot-minutes: {e}"))?
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    let slot = SlotMinutes::new(minutes).map_err(|e| e.to_string())?;
    let f = flags_for(tick, slot);
    println!(
        "tick {tick}: day {}  {:02}:{:02}  slot {}/{}  boundaries: minute={} slot={} new_day={}",
        f.day,
        f.minute_of_day / 60,
        f.minute_of_day % 60,
        f.slot_of_day,
        slot.slots_per_day(),
        f.minute,
        f.slot,
        f.new_day
    );
    Ok(ExitCode::SUCCESS)
}

pub fn pipeline_cmd() -> ExitCode {
    println!("{:>2}  {:<24} {:<8} cadence", "#", "system", "kind");
    for (i, (id, kind, cadence)) in dev_pipeline().describe().iter().enumerate() {
        println!(
            "{:>2}  {:<24} {:<8} {}",
            i + 1,
            id,
            kind,
            cadence_name(*cadence)
        );
    }
    ExitCode::SUCCESS
}
