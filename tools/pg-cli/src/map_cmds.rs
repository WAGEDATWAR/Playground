//! Developer commands for maps and pathfinding (milestone 0.4): `map show | path | bench-paths`.

use crate::args::{parse, parse_size, parse_tile, Spec};
use crate::sim_cmds::{build_demo, demo_from_flags, DemoSetup};
use pg_core::id::{EntityId, Kind};
use pg_core::map::{
    MapData, MapKind, MoveCosts, Tile, FLOOR, GRASS, ROAD, SAND, SIDEWALK, SURFACE_NONE, WATER,
};
use pg_core::path::{
    find_path, BatchExecutor, PathJob, PathOutcome, SerialExecutor, DEFAULT_EXPANSION_CAP,
};
use pg_core::rng::{Key, Rng, Seed};
use pg_core::world::WorldState;
use pg_runtime::exec::ScopedThreads;
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

pub fn map_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("show") => show(&args[1..]),
        Some("path") => path(&args[1..]),
        Some("bench-paths") => bench(&args[1..]),
        Some(other) => Err(format!(
            "unknown map command '{other}' (try show, path, bench-paths)"
        )),
        None => Err("map needs a command: show, path, bench-paths".into()),
    }
}

fn glyph(map: &MapData, t: Tile) -> char {
    if map.is_blocked(t) {
        return '#';
    }
    match map.surface_at(t).unwrap_or(SURFACE_NONE) {
        s if s == ROAD => '=',
        s if s == SIDEWALK => ':',
        s if s == FLOOR => '_',
        _ => match map.terrain_at(t).unwrap_or(GRASS) {
            x if x == WATER => '~',
            x if x == SAND => ',',
            _ => '.',
        },
    }
}

fn render(map: &MapData, overlay: &BTreeMap<Tile, char>) -> String {
    let mut out = String::new();
    for y in 0..map.height() {
        for x in 0..map.width() {
            let t = Tile::new(x, y);
            out.push(overlay.get(&t).copied().unwrap_or_else(|| glyph(map, t)));
        }
        out.push('\n');
    }
    out
}

const LEGEND: &str = ". grass   , sand   ~ water   = road   : sidewalk   _ floor   # blocked   * object   + route   A-Z pawns";

fn pawn_letter(index: usize) -> char {
    char::from(b'A' + u8::try_from(index % 26).unwrap_or(0))
}

const SHOW_SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "name",
        "dev-map",
        "dev-pawns",
        "content",
        "threads",
        "ticks",
        "nudge",
        "slot",
        "object",
        "put",
    ],
    switches: &["no-routes"],
    optional: &[],
};

/// `pg map show`: builds a demo world, runs it, and draws it.
fn show(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SHOW_SPEC)?;
    let mut setup: DemoSetup = demo_from_flags(&p)?;
    if setup.map.is_none() {
        setup.map = Some((48, 32, 1));
    }
    if setup.pawns == 0 && p.one("dev-pawns").is_none() {
        setup.pawns = 8;
    }
    let ticks = p.parse::<u64>("ticks")?.unwrap_or(600);
    let mut sim = build_demo(&setup)?;
    sim.run_ticks(ticks).map_err(|e| e.to_string())?;
    let world = sim.world();
    let map_id = EntityId::new(Kind::Map, 1);
    let Some(map) = world.maps.get(map_id) else {
        return Err("the demo world has no map".into());
    };

    let mut overlay = BTreeMap::new();
    if !p.has("no-routes") {
        for (_, pawn) in world.pawns.iter() {
            if let Some(r) = &pawn.route {
                for t in r.remaining() {
                    overlay.insert(*t, '+');
                }
            }
        }
    }
    for (_, o) in world.objects.iter() {
        if let pg_core::object::Location::OnMap { map: m, tile } = &o.location {
            if *m == map_id {
                overlay.insert(*tile, '*');
            }
        }
    }
    for (i, (_, pawn)) in world.pawns.iter().enumerate() {
        if pawn.position.map == map_id {
            overlay.insert(pawn.position.tile, pawn_letter(i));
        }
    }
    let minutes = world.clock.minute_of_day();
    println!(
        "{} {}x{}  tick {}  day {}  {:02}:{:02}   {}",
        map_id,
        map.width(),
        map.height(),
        world.clock.tick(),
        world.clock.day(),
        minutes / 60,
        minutes % 60,
        if setup.threads.is_some() {
            "(threaded paths)"
        } else {
            ""
        }
    );
    print!("{}", render(map, &overlay));
    println!("{LEGEND}\n");
    println!(
        "{:<3} {:<8} {:<10} {:<9} {:<9} route",
        "", "id", "name", "at", "goal"
    );
    for (i, (id, pawn)) in world.pawns.iter().enumerate() {
        let (goal, route) = pawn
            .route
            .as_ref()
            .map_or(("-".to_owned(), "idle".to_owned()), |r| {
                (
                    r.goal.to_string(),
                    format!(
                        "{} step(s) left{}",
                        r.remaining().len(),
                        if r.needs_repath { ", solving" } else { "" }
                    ),
                )
            });
        println!(
            "{:<3} {:<8} {:<10} {:<9} {:<9} {route}",
            pawn_letter(i),
            id.to_string(),
            pawn.name,
            pawn.position.tile.to_string(),
            goal
        );
    }
    println!("\nstate hash {}", world.state_hash());
    Ok(ExitCode::SUCCESS)
}

fn demo_map(seed: &str, w: i32, h: i32, style: u8) -> Result<(WorldState, EntityId), String> {
    let mut world = WorldState::new("Demo", seed);
    let id = world
        .create_map(MapKind::Overworld, w, h)
        .map_err(|e| e.to_string())?;
    pg_core::dev::generate_dev_map(&mut world, id, style);
    Ok((world, id))
}

const PATH_SPEC: Spec<'static> = Spec {
    values: &["seed", "size", "from", "to", "cap"],
    switches: &[],
    optional: &[],
};

/// `pg map path`: one path request, drawn.
fn path(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &PATH_SPEC)?;
    let (w, h, style) = parse_size(p.one("size").unwrap_or("48x32"))?;
    let (world, id) = demo_map(p.one("seed").unwrap_or("demo"), w, h, style.unwrap_or(1))?;
    let (fx, fy) = parse_tile(p.one("from").ok_or("path needs --from x,y")?)?;
    let (tx, ty) = parse_tile(p.one("to").ok_or("path needs --to x,y")?)?;
    let cap = p.parse::<u32>("cap")?.unwrap_or(DEFAULT_EXPANSION_CAP);
    let Some(map) = world.maps.get(id) else {
        return Err("no map".into());
    };
    let (from, to) = (Tile::new(fx, fy), Tile::new(tx, ty));
    let started = Instant::now();
    let outcome = find_path(map, &MoveCosts::default(), from, to, cap);
    let elapsed = started.elapsed();
    let mut overlay = BTreeMap::new();
    match &outcome {
        PathOutcome::Found {
            path,
            cost,
            expanded,
        } => {
            for t in path {
                overlay.insert(*t, '*');
            }
            println!(
                "found: {} step(s), cost {cost}, {expanded} node(s) expanded, {:.2?}",
                path.len(),
                elapsed
            );
        }
        PathOutcome::Unreachable { expanded } => {
            println!("unreachable after expanding {expanded} node(s)")
        }
        PathOutcome::TooFar { expanded } => {
            println!("gave up: expansion cap reached after {expanded} node(s) (--cap raises it)")
        }
        PathOutcome::BlockedDestination => println!("blocked destination: {to} cannot be entered"),
    }
    overlay.insert(from, 'S');
    overlay.insert(to, 'G');
    print!("{}", render(map, &overlay));
    println!("{LEGEND}   S start   G goal");
    Ok(if outcome.is_found() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

const BENCH_SPEC: Spec<'static> = Spec {
    values: &["seed", "size", "requests", "threads"],
    switches: &[],
    optional: &[],
};

/// `pg map bench-paths`: the same batch of path requests solved serially and on several thread counts,
/// checking that the results are identical (the determinism guarantee behind parallel pathfinding).
fn bench(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &BENCH_SPEC)?;
    let seed = p.one("seed").unwrap_or("bench");
    let (w, h, style) = parse_size(p.one("size").unwrap_or("128x96"))?;
    let n = p.parse::<usize>("requests")?.unwrap_or(2000);
    let thread_counts: Vec<usize> = p
        .one("threads")
        .unwrap_or("1,2,4,8")
        .split(',')
        .map(|t| {
            t.trim()
                .parse::<usize>()
                .map_err(|e| format!("--threads '{t}': {e}"))
        })
        .collect::<Result<_, _>>()?;
    let (world, id) = demo_map(seed, w, h, style.unwrap_or(1))?;
    let Some(map) = world.maps.get(id) else {
        return Err("no map".into());
    };
    let costs = MoveCosts::default();

    // Random requests between passable tiles.
    let rng = Rng::new(Seed::from_text(seed), "bench.requests", &[Key::Int(0)]);
    let mut jobs = Vec::new();
    let mut counter = 0u32;
    let pick = |counter: &mut u32| -> Option<Tile> {
        for _ in 0..200 {
            let x = rng.int_in(*counter, 0, w - 1)?;
            *counter += 1;
            let y = rng.int_in(*counter, 0, h - 1)?;
            *counter += 1;
            let t = Tile::new(x, y);
            if costs.step_cost(map, t).is_some() {
                return Some(t);
            }
        }
        None
    };
    while jobs.len() < n {
        let (Some(a), Some(b)) = (pick(&mut counter), pick(&mut counter)) else {
            break;
        };
        jobs.push(PathJob {
            map,
            costs: &costs,
            from: a,
            to: b,
            cap: DEFAULT_EXPANSION_CAP,
        });
    }
    if jobs.is_empty() {
        return Err("could not find passable tiles to build requests".into());
    }

    println!(
        "map {w}x{h}, {} requests, expansion cap {DEFAULT_EXPANSION_CAP}\n",
        jobs.len()
    );
    let started = Instant::now();
    let serial = SerialExecutor.solve(&jobs);
    let base = started.elapsed();
    let found = serial.iter().filter(|r| r.is_found()).count();
    let expanded: u64 = serial
        .iter()
        .map(|r| match r {
            PathOutcome::Found { expanded, .. }
            | PathOutcome::Unreachable { expanded }
            | PathOutcome::TooFar { expanded } => u64::from(*expanded),
            PathOutcome::BlockedDestination => 0,
        })
        .sum();
    println!(
        "{:<10} {:>10} {:>9}  identical to serial",
        "executor", "time", "speedup"
    );
    println!(
        "{:<10} {:>10.2?} {:>9}  (reference)",
        "serial", base, "1.00x"
    );
    let mut all_same = true;
    for t in thread_counts {
        let started = Instant::now();
        let out = ScopedThreads::new(t).solve(&jobs);
        let dt = started.elapsed();
        let same = out == serial;
        all_same &= same;
        let speedup = format!(
            "{}.{:02}x",
            base.as_micros() / dt.as_micros().max(1),
            (base.as_micros() * 100 / dt.as_micros().max(1)) % 100
        );
        println!(
            "{:<10} {:>10.2?} {:>9}  {}",
            format!("{t} thread(s)"),
            dt,
            speedup,
            if same { "yes" } else { "NO - BUG" }
        );
    }
    println!(
        "\n{found} of {} requests found a path; {expanded} nodes expanded in total",
        jobs.len()
    );
    Ok(if all_same {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
