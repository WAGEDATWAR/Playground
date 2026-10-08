//! Developer commands for the simulation driver: `sim`, `replay`, `time`, `pipeline`.

use crate::args::{parse, parse_size, Parsed, Spec};
use crate::shared::{load_content, short, EventFilter, DEFAULT_CONTENT_DIR};
use pg_content::ContentSet;
use pg_core::canon::ToCanon;
use pg_core::containment::validate_containment;
use pg_core::id::{EntityId, Kind};
use pg_core::input::{Command, SettingChange, SimInput};
use pg_core::map::Tile;
use pg_core::pipeline::{Cadence, Pipeline};
use pg_core::replay::{bisect, build_sim, diff_logs, replay, ReplayLog};
use pg_core::sim::Sim;
use pg_core::time::{flags_for, SlotMinutes, TICKS_PER_DAY};
use pg_core::world::WorldState;
use pg_runtime::exec::ScopedThreads;
use std::process::ExitCode;
use std::sync::Arc;

pub const SIM_SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "name",
        "days",
        "ticks",
        "nudge",
        "slot",
        "log",
        "dev-map",
        "dev-pawns",
        "content",
        "threads",
        "since",
        "until",
        "object",
        "put",
        "propose",
        "day",
        "at",
    ],
    switches: &["pretty", "needs"],
    optional: &["events"],
};

/// `<tick>:<number>`.
fn tick_pair(text: &str, flag: &str) -> Result<(u64, i64), String> {
    let (a, b) = text
        .split_once(':')
        .ok_or_else(|| format!("--{flag} expects <tick>:<value>, got '{text}'"))?;
    let tick = a.parse().map_err(|e| format!("--{flag} tick '{a}': {e}"))?;
    let val = b
        .parse()
        .map_err(|e| format!("--{flag} value '{b}': {e}"))?;
    Ok((tick, val))
}

/// `template@x,y`.
fn parse_object(text: &str) -> Result<(String, Tile), String> {
    let (template, at) = text
        .split_once('@')
        .ok_or_else(|| format!("--object expects template@x,y, got '{text}'"))?;
    let (x, y) = crate::args::parse_tile(at)?;
    Ok((template.to_owned(), Tile::new(x, y)))
}

/// `child:owner:container`, for example `obj_2:obj_1:contents`.
fn parse_put(text: &str) -> Result<(EntityId, EntityId, String), String> {
    let parts: Vec<&str> = text.split(':').collect();
    match parts.as_slice() {
        [child, owner, container] => Ok((
            child
                .parse()
                .map_err(|e| format!("--put child '{child}': {e}"))?,
            owner
                .parse()
                .map_err(|e| format!("--put owner '{owner}': {e}"))?,
            (*container).to_owned(),
        )),
        _ => Err(format!("--put expects child:owner:container, got '{text}'")),
    }
}

fn cadence_name(c: Cadence) -> &'static str {
    match c {
        Cadence::Tick => "tick",
        Cadence::Minute => "minute",
        Cadence::Slot => "slot",
        Cadence::Day => "day",
    }
}

fn write_log(path: &str, log: &ReplayLog, pretty: bool) -> Result<(), String> {
    if path.ends_with(".pglog") {
        return std::fs::write(path, pg_persist::logfile::encode_log(log))
            .map_err(|e| format!("cannot write {path}: {e}"));
    }
    let canonical = log.to_canon().to_canonical_string();
    let text = if pretty {
        let v: serde_json::Value = serde_json::from_str(&canonical).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
    } else {
        canonical
    };
    std::fs::write(path, text + "\n").map_err(|e| format!("cannot write {path}: {e}"))
}

/// Reads a replay log in any form: plain JSON, compressed `.pglog`, or a `.pgbundle` (its trimmed log).
fn read_log(path: &str) -> Result<ReplayLog, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    pg_persist::logfile::read_any(&bytes).map_err(|e| format!("{path}: {e}"))
}

/// Everything `sim` and `map show` need to describe a demo world, as inputs applied at tick 0.
pub struct DemoSetup {
    pub seed: String,
    pub name: String,
    pub map: Option<(i32, i32, u8)>,
    pub pawns: u32,
    pub nudges: Vec<(u64, i64)>,
    pub slots: Vec<(u64, i64)>,
    /// `(template, tile)`: objects spawned on map_1 at tick 1.
    pub objects: Vec<(String, Tile)>,
    /// `(child, owner, container)`: moves applied at tick 2.
    pub puts: Vec<(EntityId, EntityId, String)>,
    /// Commitment proposals: `(tick, proposer, invitee, start slot, slots, place)`.
    pub proposals: Vec<Proposal>,
    pub content: Option<Arc<ContentSet>>,
    pub threads: Option<usize>,
}

/// `--propose <tick>:<proposer>:<invitee>:<start>:<len>:<x>,<y>`.
#[derive(Clone, Debug)]
pub struct Proposal {
    pub tick: u64,
    pub proposer: EntityId,
    pub invitee: EntityId,
    pub start: u32,
    pub len: u32,
    pub at: Tile,
}

fn parse_proposal(text: &str) -> Result<Proposal, String> {
    let bad = || format!("--propose expects tick:proposer:invitee:start:len:x,y, got '{text}'");
    let parts: Vec<&str> = text.split(':').collect();
    let [tick, a, b, start, len, at] = parts.as_slice() else {
        return Err(bad());
    };
    let (x, y) = crate::args::parse_tile(at)?;
    Ok(Proposal {
        tick: tick.parse().map_err(|_| bad())?,
        proposer: a
            .parse()
            .map_err(|e| format!("--propose proposer '{a}': {e}"))?,
        invitee: b
            .parse()
            .map_err(|e| format!("--propose invitee '{b}': {e}"))?,
        start: start.parse().map_err(|_| bad())?,
        len: len.parse().map_err(|_| bad())?,
        at: Tile::new(x, y),
    })
}

pub fn build_demo(setup: &DemoSetup) -> Result<Sim, String> {
    let mut sim = Sim::with_dev_systems(WorldState::new(setup.name.clone(), setup.seed.clone()));
    if let Some(c) = &setup.content {
        sim = sim.with_content(Arc::clone(c));
    }
    if let Some(n) = setup.threads {
        sim.set_executor(Box::new(ScopedThreads::new(n)));
    }
    let cmd = |c: Command| SimInput::Command {
        actor: None,
        cmd: c,
    };
    if let Some((w, h, style)) = setup.map {
        sim.submit(0, cmd(Command::DevCreateMap { w, h, style }))
            .map_err(|e| e.to_string())?;
        let map = EntityId::new(Kind::Map, 1);
        for i in 0..setup.pawns {
            sim.submit(
                0,
                cmd(Command::DevSpawnPawn {
                    map,
                    at: None,
                    name: format!("P{}", i + 1),
                }),
            )
            .map_err(|e| e.to_string())?;
        }
    } else if setup.pawns > 0 {
        return Err("--dev-pawns needs --dev-map (pawns live on a map)".into());
    }
    if setup.map.is_some() {
        let map = EntityId::new(Kind::Map, 1);
        for (template, at) in &setup.objects {
            sim.submit(
                1,
                cmd(Command::DevSpawnObject {
                    map,
                    at: *at,
                    template: template.clone(),
                }),
            )
            .map_err(|e| e.to_string())?;
        }
        for (child, owner, container) in &setup.puts {
            sim.submit(
                2,
                cmd(Command::DevPutInContainer {
                    child: *child,
                    owner: *owner,
                    container: container.clone(),
                }),
            )
            .map_err(|e| e.to_string())?;
        }
    } else if !setup.objects.is_empty() || !setup.puts.is_empty() {
        return Err("--object and --put need --dev-map".into());
    }
    for pr in &setup.proposals {
        sim.submit(
            pr.tick,
            cmd(Command::DevPropose {
                proposer: pr.proposer,
                invitee: pr.invitee,
                start: pr.start,
                len: pr.len,
                at: pr.at,
                expires_in: u32::try_from(TICKS_PER_DAY).unwrap_or(u32::MAX),
                reschedulable: false,
            }),
        )
        .map_err(|e| e.to_string())?;
    }
    for (tick, amount) in &setup.nudges {
        let amount = i32::try_from(*amount)
            .map_err(|_| format!("--nudge amount {amount} does not fit i32"))?;
        sim.submit(*tick, cmd(Command::DevNudge { amount }))
            .map_err(|e| e.to_string())?;
    }
    for (tick, minutes) in &setup.slots {
        let minutes =
            u32::try_from(*minutes).map_err(|_| format!("--slot minutes {minutes} is invalid"))?;
        sim.submit(
            *tick,
            SimInput::SettingChange(SettingChange::SlotMinutes(minutes)),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(sim)
}

/// Reads the demo-world flags shared by `sim` and `map show`.
pub fn demo_from_flags(p: &Parsed) -> Result<DemoSetup, String> {
    let seed = p.one("seed").ok_or("needs --seed <text>")?.to_owned();
    let map = p
        .one("dev-map")
        .map(|t| parse_size(t).map(|(w, h, s)| (w, h, s.unwrap_or(1))))
        .transpose()?;
    let content_dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    Ok(DemoSetup {
        seed,
        name: p.one("name").unwrap_or("Dev Town").to_owned(),
        map,
        pawns: p.parse::<u32>("dev-pawns")?.unwrap_or(0),
        objects: p
            .all("object")
            .into_iter()
            .map(parse_object)
            .collect::<Result<_, _>>()?,
        puts: p
            .all("put")
            .into_iter()
            .map(parse_put)
            .collect::<Result<_, _>>()?,
        proposals: p
            .all("propose")
            .into_iter()
            .map(parse_proposal)
            .collect::<Result<_, _>>()?,
        nudges: p
            .all("nudge")
            .into_iter()
            .map(|t| tick_pair(t, "nudge"))
            .collect::<Result<_, _>>()?,
        slots: p
            .all("slot")
            .into_iter()
            .map(|t| tick_pair(t, "slot"))
            .collect::<Result<_, _>>()?,
        content: if content_dirs.is_empty() {
            None
        } else {
            Some(load_content(&content_dirs)?)
        },
        threads: p.parse::<usize>("threads")?,
    })
}

pub fn sim_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SIM_SPEC)?;
    let setup = demo_from_flags(&p)?;
    let (days, ticks) = (p.parse::<u64>("days")?, p.parse::<u64>("ticks")?);
    let total = match (days, ticks) {
        (Some(d), None) => d.checked_mul(TICKS_PER_DAY).ok_or("--days is too large")?,
        (None, Some(t)) => t,
        (None, None) => 3 * TICKS_PER_DAY,
        (Some(_), Some(_)) => return Err("use either --days or --ticks, not both".into()),
    };
    let filter = EventFilter {
        enabled: p.has("events"),
        prefix: p.one("events").map(str::to_owned),
        since: p.parse::<u64>("since")?.unwrap_or(0),
        until: p.parse::<u64>("until")?,
    };

    let mut sim = build_demo(&setup)?;
    println!(
        "world {:?}  seed {:?}  ticks {total} ({} days){}{}",
        setup.name,
        setup.seed,
        total / TICKS_PER_DAY,
        setup.map.map_or(String::new(), |(w, h, s)| format!(
            "  map {w}x{h} style {s}, {} pawn(s)",
            setup.pawns
        )),
        setup
            .threads
            .map_or(String::new(), |n| format!("  path threads {n}")),
    );
    let mut event_count = 0usize;
    for _ in 0..total {
        let report = sim.step().map_err(|e| e.to_string())?;
        event_count += report.events.len();
        for e in report
            .events
            .iter()
            .filter(|e| filter.matches(&e.kind, e.tick))
        {
            println!(
                "  event @{:>7}  {:<16} {}",
                e.tick,
                e.kind,
                e.detail.to_canonical_string()
            );
        }
        if let Some(h) = report.day_hash {
            let tables = sim
                .day_hashes()
                .last()
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
    let walking = w.pawns.iter().filter(|(_, p)| p.route.is_some()).count();
    println!(
        "done: tick {}  day {}  maps {}  pawns {} ({walking} walking)  objects {}  probe(value={}, minutes={}, days={})  slot={}min  events={event_count}",
        w.clock.tick(),
        w.clock.day(),
        w.maps.len(),
        w.pawns.len(),
        w.objects.len(),
        w.probe.value,
        w.probe.minutes,
        w.probe.days,
        w.settings.slot_minutes.get()
    );
    if p.has("needs") {
        print_needs(w);
    }
    let (hits, misses) = sim.path_cache_stats();
    if hits + misses > 0 {
        println!("path cache: {hits} hits, {misses} misses");
    }
    if let Some(c) = sim.content() {
        let report = validate_containment(w, c);
        println!(
            "containment: {}",
            if report.is_ok() {
                "OK".to_owned()
            } else {
                format!("{} violation(s)\n{report}", report.error_count())
            }
        );
    }
    println!("final hash {}", w.state_hash());
    if let Some(path) = p.one("log") {
        write_log(path, &ReplayLog::record(&sim), p.has("pretty"))?;
        println!("replay log written to {path}");
    }
    Ok(ExitCode::SUCCESS)
}

/// `pg sim --needs`: each pawn's needs, mood and capacities at the end of the run (Stage 1).
fn print_needs(w: &pg_core::world::WorldState) {
    let Some((_, first)) = w.pawns.iter().next() else {
        return;
    };
    let ids: Vec<&String> = first.needs.keys().collect();
    print!(
        "
{:<10} {:<14}",
        "pawn", "mood"
    );
    for n in &ids {
        print!(" {n:>8}");
    }
    println!("  capacities below full");
    for (id, p) in w.pawns.iter() {
        print!("{:<10} {:<14}", id.to_string(), p.mood);
        for n in &ids {
            print!(" {:>8}", p.needs.get(*n).copied().unwrap_or(0));
        }
        let low: Vec<String> = p
            .capacities
            .as_pairs()
            .iter()
            .filter(|(_, v)| *v < 1000)
            .map(|(k, v)| format!("{k} {v}"))
            .collect();
        println!(
            "  {}",
            if low.is_empty() {
                "-".to_owned()
            } else {
                low.join(", ")
            }
        );
    }
}

fn dev_pipeline() -> Pipeline {
    let mut p = Pipeline::new();
    pg_core::dev::install(&mut p);
    p
}

/// Content for a log: from `--content`, else the base pack if the log used content.
fn content_for(log: &ReplayLog, p: &Parsed) -> Result<Option<Arc<ContentSet>>, String> {
    let dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if !dirs.is_empty() {
        return load_content(&dirs).map(Some);
    }
    if log.content.is_empty() {
        return Ok(None);
    }
    load_content(&[DEFAULT_CONTENT_DIR.to_owned()]).map(Some)
}

/// Splits the run at `at`, snapshots, resumes, and compares with the uninterrupted run.
fn snapshot_equivalence(
    log: &ReplayLog,
    at: u64,
    content: Option<Arc<ContentSet>>,
) -> Result<bool, String> {
    let mut whole = build_sim(log, content.clone()).map_err(|e| e.to_string())?;
    whole.run_ticks(log.ticks).map_err(|e| e.to_string())?;
    let at = at.min(log.ticks);
    let mut first = build_sim(log, content.clone()).map_err(|e| e.to_string())?;
    first.run_ticks(at).map_err(|e| e.to_string())?;
    let snap = first.snapshot();
    let pending = snap.pending.len();
    let mut resumed = Sim::restore(snap, dev_pipeline());
    if let Some(c) = content {
        resumed = resumed.with_content(c);
    }
    resumed
        .run_ticks(log.ticks - at)
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

const REPLAY_SPEC: Spec<'static> = Spec {
    values: &["snapshot-at", "content", "out"],
    switches: &["diff", "bisect", "rerecord"],
    optional: &[],
};

pub fn replay_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &REPLAY_SPEC)?;
    if p.has("diff") {
        return diff_cmd(&p);
    }
    if p.has("bisect") {
        return bisect_cmd(&p);
    }
    let path = p.positional.first().ok_or("replay needs <log.json>")?;
    let log = read_log(path)?;
    if p.has("rerecord") {
        return rerecord_cmd(path, log, &p);
    }
    println!(
        "{path}: world {:?} seed {:?}, {} ticks, {} input(s), {} day hash(es), {} content pack(s)",
        log.world_name,
        log.seed_text,
        log.ticks,
        log.inputs.len(),
        log.day_hashes.len(),
        log.content.len()
    );
    let content = content_for(&log, &p)?;
    let outcome = replay(&log, content.clone()).map_err(|e| e.to_string())?;
    for d in &outcome.day_hashes {
        let bad = outcome.mismatches.iter().find(|m| m.day == Some(d.day));
        match bad {
            None => println!("  day {:>3}  {}  ok", d.day, short(&d.hash)),
            Some(m) => println!(
                "  day {:>3}  {}  MISMATCH (tables: {})",
                d.day,
                short(&d.hash),
                m.tables.join(", ")
            ),
        }
    }
    for m in &outcome.mismatches {
        println!(
            "  MISMATCH {:?}: expected {}  actual {}",
            m.day, m.expected, m.actual
        );
    }
    let mut ok = outcome.ok();
    if let Some(at) = p.parse::<u64>("snapshot-at")? {
        ok &= snapshot_equivalence(&log, at, content)?;
    }
    println!("{}", if ok { "REPLAY OK" } else { "REPLAY FAILED" });
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `pg replay <log> --rerecord [--out file]`: runs the log's inputs again against the current build and
/// content and writes the result with its new hashes (the golden logs are re-pinned this way after a
/// deliberate change to the state shape or the base content; say why in `docs/DECISIONS.md`).
fn rerecord_cmd(path: &str, mut log: ReplayLog, p: &Parsed) -> Result<ExitCode, String> {
    if log.start.is_some() {
        return Err("a trimmed log cannot be re-recorded (it has no start of its own)".into());
    }
    let content = content_for(&log, p)?;
    if let Some(c) = &content {
        log.content = c
            .refs()
            .into_iter()
            .map(|r| pg_core::replay::ContentRefRecord {
                pack_id: r.pack_id.to_string(),
                version: r.version.to_string(),
                hash: r.hash,
            })
            .collect();
    }
    let mut sim = build_sim(&log, content).map_err(|e| e.to_string())?;
    sim.run_ticks(log.ticks).map_err(|e| e.to_string())?;
    let out = p.one("out").unwrap_or(path);
    write_log(out, &ReplayLog::record(&sim), false)?;
    println!(
        "re-recorded {} tick(s), {} input(s): final hash {} written to {out}",
        log.ticks,
        log.inputs.len(),
        short(&sim.world().state_hash())
    );
    Ok(ExitCode::SUCCESS)
}

fn two_logs(p: &Parsed, what: &str) -> Result<(ReplayLog, ReplayLog), String> {
    match p.positional.as_slice() {
        [a, b] => Ok((read_log(a)?, read_log(b)?)),
        _ => Err(format!("replay --{what} needs exactly two log files")),
    }
}

fn diff_cmd(p: &Parsed) -> Result<ExitCode, String> {
    let (a, b) = two_logs(p, "diff")?;
    let d = diff_logs(&a, &b);
    if d.identical() {
        println!("the two logs are identical (same header, inputs and hashes)");
        return Ok(ExitCode::SUCCESS);
    }
    for h in &d.header_differences {
        println!("header differs: {h}");
    }
    if let Some(i) = d.first_input_difference {
        println!("first differing input: #{i}");
        println!(
            "  a: {}",
            a.inputs.get(i).map_or("<none>".into(), |s| format!(
                "tick {} {}",
                s.tick,
                s.input.to_canon().to_canonical_string()
            ))
        );
        println!(
            "  b: {}",
            b.inputs.get(i).map_or("<none>".into(), |s| format!(
                "tick {} {}",
                s.tick,
                s.input.to_canon().to_canonical_string()
            ))
        );
    }
    match &d.first_day_mismatch {
        Some(m) => println!(
            "first differing day hash: day {} (tick {}) in table(s): {}",
            m.day,
            m.tick,
            m.tables.join(", ")
        ),
        None => println!("all recorded day hashes agree"),
    }
    if !d.final_tables.is_empty() {
        println!(
            "final state differs in table(s): {}",
            d.final_tables.join(", ")
        );
    }
    println!("tip: `pg replay --bisect a.json b.json` finds the exact tick");
    Ok(ExitCode::FAILURE)
}

fn bisect_cmd(p: &Parsed) -> Result<ExitCode, String> {
    let (a, b) = two_logs(p, "bisect")?;
    let content = content_for(&a, p)?.or(content_for(&b, p)?);
    let r = bisect(&a, &b, content).map_err(|e| e.to_string())?;
    match r.first_diff_tick {
        None => {
            println!(
                "no difference within the first {} ticks",
                a.ticks.min(b.ticks)
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(t) => {
            let flags = flags_for(t, SlotMinutes::DEFAULT);
            println!(
                "first difference while processing tick {t} (day {}, {:02}:{:02}) in table(s): {}",
                flags.day,
                flags.minute_of_day / 60,
                flags.minute_of_day % 60,
                r.tables.join(", ")
            );
            for (table, ids) in &r.rows {
                println!("  {table}: differing row(s): {}", ids.join(", "));
            }
            for (label, applied) in [("a", &r.applied_a), ("b", &r.applied_b)] {
                if applied.is_empty() {
                    println!("  {label}: no inputs applied on that tick (the difference comes from earlier state or a system)");
                }
                for s in applied {
                    println!(
                        "  {label}: input {}",
                        s.input.to_canon().to_canonical_string()
                    );
                }
            }
            Ok(ExitCode::FAILURE)
        }
    }
}

pub fn time_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(
        args,
        &Spec {
            values: &["slot-minutes"],
            switches: &[],
            optional: &[],
        },
    )?;
    let tick: u64 = p
        .positional
        .first()
        .ok_or("time needs <tick>")?
        .parse()
        .map_err(|e| format!("tick: {e}"))?;
    let minutes = p
        .parse::<u32>("slot-minutes")?
        .unwrap_or(pg_core::time::DEFAULT_SLOT_MINUTES);
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
