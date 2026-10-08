//! Developer commands for the runtime (0.8): `profile`, `run`, `tools`.

use crate::args::{parse, parse_size, Spec};
use crate::shared::{load_content, short};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use pg_host::{RedactingLog, StderrLog};
use pg_host_os::{FsStorage, SystemClock};
use pg_persist::store::{LoadError, LoadOptions, SlotStore};
use pg_runtime::control::{RunEvent, RunState, Speed};
use pg_runtime::keyframes::SimFactory;
use pg_runtime::pool::WorkerPool;
use pg_runtime::profile::Profiler;
use pg_runtime::session::Session;
use pg_runtime::sim_loop::{Control, LoopConfig, LoopEvent, LoopServices, SimLoop};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "dev-map",
        "dev-pawns",
        "days",
        "ticks",
        "threads",
        "dir",
        "world",
        "seconds",
        "speed",
        "shadow",
        "autosave-minutes",
        "content",
        "scrub",
    ],
    switches: &[],
    optional: &[],
};

type MapSize = (i32, i32, u8);

fn map_size(text: Option<&str>, default: MapSize) -> Result<MapSize, String> {
    Ok(match text.map(parse_size).transpose()? {
        Some((w, h, style)) => (w, h, style.unwrap_or(default.2)),
        None => default,
    })
}

fn populated_sim(
    factory: &SimFactory,
    seed: &str,
    size: MapSize,
    pawns: u32,
) -> Result<Sim, String> {
    let mut sim = factory.new_sim(WorldState::new("Dev Town", seed));
    let cmd = |c| SimInput::Command {
        actor: None,
        cmd: c,
    };
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: size.0,
            h: size.1,
            style: size.2,
        }),
    )
    .map_err(|e| e.to_string())?;
    let map = EntityId::new(Kind::Map, 1);
    for i in 0..pawns {
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
    Ok(sim)
}

pub fn profile_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let seed = p.one("seed").unwrap_or("profile");
    let size = map_size(p.one("dev-map"), (96, 72, 1))?;
    let pawns = p.parse::<u32>("dev-pawns")?.unwrap_or(200);
    let threads = p.parse::<usize>("threads")?.unwrap_or(1);
    let ticks = match (p.parse::<u64>("days")?, p.parse::<u64>("ticks")?) {
        (Some(d), None) => d * pg_core::time::TICKS_PER_DAY,
        (None, Some(t)) => t,
        (None, None) => pg_core::time::TICKS_PER_DAY,
        _ => return Err("use either --days or --ticks".into()),
    };
    let prof = Profiler::new();
    let factory = SimFactory::dev(None, threads).with_profiler(Some(prof.clone()));
    let mut sim = populated_sim(&factory, seed, size, pawns)?;
    sim.run_ticks(1).map_err(|e| e.to_string())?; // build the world outside the measured run
    prof.reset();
    let start = Instant::now();
    sim.run_ticks(ticks).map_err(|e| e.to_string())?;
    let wall = start.elapsed();
    let report = prof.report();
    let per_tick_ms = wall.as_secs_f64() * 1000.0 / ticks.max(1) as f64;
    println!(
        "{pawns} pawn(s) on {}x{}, {ticks} ticks, {threads} path thread(s): {:.2} s wall, {:.0} ticks/s, {per_tick_ms:.3} ms per tick",
        size.0,
        size.1,
        wall.as_secs_f64(),
        ticks as f64 / wall.as_secs_f64().max(1e-9),
    );
    println!(
        "{:<26} {:>9} {:>10} {:>10} {:>10} {:>6}",
        "system", "calls", "total ms", "avg us", "max us", "share"
    );
    for (name, s) in &report.rows {
        println!(
            "{:<26} {:>9} {:>10.1} {:>10.2} {:>10.1} {:>5.1}%",
            name,
            s.calls,
            s.total.as_secs_f64() * 1000.0,
            s.total.as_secs_f64() * 1e6 / s.calls.max(1) as f64,
            s.max.as_secs_f64() * 1e6,
            s.total.as_secs_f64() * 100.0 / report.total.as_secs_f64().max(1e-12)
        );
    }
    let (hits, misses) = sim.path_cache_stats();
    println!(
        "path cache: {hits} hits, {misses} misses; final hash {}",
        short(&sim.world().state_hash())
    );
    println!(
        "budget: {per_tick_ms:.3} ms of the 100 ms available per tick at 1x ({:.2}%)",
        per_tick_ms
    );
    Ok(ExitCode::SUCCESS)
}

fn speed_arg(text: &str) -> Result<Speed, String> {
    Speed::ALL
        .into_iter()
        .find(|s| s.name() == text)
        .ok_or_else(|| format!("--speed must be one of 1x, 3x, 9x, 27x, not '{text}'"))
}

pub fn run_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let dir = p.one("dir").unwrap_or("pg-data");
    let world_id = p.one("world").unwrap_or("run");
    let seconds = p.parse::<u64>("seconds")?.unwrap_or(5);
    let speed = speed_arg(p.one("speed").unwrap_or("9x"))?;
    let content_dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    let content = if content_dirs.is_empty() {
        None
    } else {
        Some(load_content(&content_dirs)?)
    };
    let threads = p.parse::<usize>("threads")?.unwrap_or(2);
    let factory = SimFactory::dev(content.clone(), threads);
    let storage = Arc::new(FsStorage::new(dir).map_err(|e| e.to_string())?);

    let existing = SlotStore::new(storage.as_ref()).load(world_id, &LoadOptions::default());
    let world = match existing {
        Ok(l) => {
            println!(
                "loaded '{world_id}' at tick {} ({:?})",
                l.world.clock.tick(),
                l.recovery
            );
            l.world
        }
        Err(LoadError::NotFound) => {
            let seed = p.one("seed").unwrap_or("run");
            let size = map_size(p.one("dev-map"), (48, 36, 1))?;
            let pawns = p.parse::<u32>("dev-pawns")?.unwrap_or(10);
            let mut sim = populated_sim(&factory, seed, size, pawns)?;
            sim.run_ticks(1).map_err(|e| e.to_string())?;
            println!(
                "created '{world_id}': {pawns} pawn(s) on {}x{}",
                size.0, size.1
            );
            sim.world().clone()
        }
        Err(e) => return Err(e.to_string()),
    };

    let services = LoopServices {
        clock: Arc::new(SystemClock::new()),
        storage: storage.clone(),
        log: Arc::new(RedactingLog::new(StderrLog)),
        pool: Arc::new(WorkerPool::new(2)),
        thumbnailer: None,
        console: None,
        dialogue: None,
    };
    let mut cfg = LoopConfig::new(world_id);
    cfg.shadow_threads = p.parse::<usize>("shadow")?;
    cfg.autosave_minutes = p.parse::<u64>("autosave-minutes")?.unwrap_or(5);
    cfg.keyframe_interval_ticks = 600;
    let session = Session::start(
        SimLoop::new(factory, world, services, cfg),
        Duration::from_millis(4),
    );
    session.send(Control::Run(RunEvent::Resume(speed)));
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut next_report = Instant::now();
    let mut problems = 0;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
        for e in session.events() {
            if matches!(
                e,
                LoopEvent::SaveFailed(_) | LoopEvent::Diverged { .. } | LoopEvent::Crashed { .. }
            ) {
                problems += 1;
            }
            println!("  event: {e:?}");
        }
        if Instant::now() >= next_report {
            let s = session.snapshot();
            println!(
                "  tick {:>7}  day {} {:02}:{:02}  {} pawn(s)  {:?}",
                s.tick,
                s.day,
                s.minute_of_day / 60,
                s.minute_of_day % 60,
                s.pawns.len(),
                s.run
            );
            next_report += Duration::from_secs(1);
        }
        if session.snapshot().run == RunState::Crashed {
            break;
        }
    }
    if let Some(t) = p.parse::<u64>("scrub")? {
        session.send(Control::RewindTo(t));
        std::thread::sleep(Duration::from_millis(200));
        for e in session.events() {
            println!("  event: {e:?}");
        }
        println!("  scrubbed: now at tick {}", session.snapshot().tick);
    }
    if let Some(mut lp) = session.shutdown() {
        for e in lp.finish_saves().into_iter().chain(lp.finish_shadows()) {
            if matches!(e, LoopEvent::SaveFailed(_) | LoopEvent::Diverged { .. }) {
                problems += 1;
            }
            println!("  event: {e:?}");
        }
        println!(
            "stopped at tick {}, hash {}",
            lp.sim().world().clock.tick(),
            short(&lp.sim().world().state_hash())
        );
    }
    Ok(if problems == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub fn tools_cmd() -> ExitCode {
    for t in pg_runtime::devtools::registry() {
        println!(
            "{:<10} {:<22} {:<14} {:<18} {}",
            t.id,
            t.title,
            t.category,
            t.cli.unwrap_or("-"),
            if t.overlay { "overlay" } else { "" }
        );
    }
    ExitCode::SUCCESS
}
