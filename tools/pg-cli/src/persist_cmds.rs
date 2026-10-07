//! Developer commands for persistence (0.6): `save`, `bugbundle`, `scenario`.

use crate::args::{parse, Spec};
use crate::shared::{load_content, short, DEFAULT_CONTENT_DIR};
use crate::sim_cmds::{build_demo, demo_from_flags};
use pg_content::ContentSet;
use pg_core::pipeline::Pipeline;
use pg_core::replay::replay;
use pg_core::sim::{Sim, SimSnapshot};
use pg_core::time::TICKS_PER_DAY;
use pg_host::Storage;
use pg_host_os::FsStorage;
use pg_persist::codec;
use pg_persist::compat::ContentRefRecord;
use pg_persist::export::{commit_import, export_world, import_world, ImportOptions};
use pg_persist::logfile::{read_any, Bundle};
use pg_persist::scenario;
use pg_persist::store::{LoadError, LoadOptions, Recovery, SlotStore};
use std::process::ExitCode;
use std::sync::Arc;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The current UTC time as `YYYY-MM-DDTHH:MM:SSZ` (wall clock is allowed in tools; never in the core).
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    pg_host::iso_utc(secs)
}

fn refs_of(content: &Option<Arc<ContentSet>>) -> Vec<ContentRefRecord> {
    content.as_ref().map_or_else(Vec::new, |c| {
        c.refs()
            .into_iter()
            .map(|r| ContentRefRecord {
                pack_id: r.pack_id.to_string(),
                version: r.version.to_string(),
                hash: r.hash,
            })
            .collect()
    })
}

fn dev_pipeline() -> Pipeline {
    let mut p = Pipeline::new();
    pg_core::dev::install(&mut p);
    p
}

fn open(dir: &str) -> Result<FsStorage, String> {
    FsStorage::new(dir).map_err(|e| format!("cannot open save directory '{dir}': {e}"))
}

pub fn save_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, rest)) => match c.as_str() {
            "create" => create(rest),
            "list" => list(rest),
            "inspect" => inspect(rest),
            "verify" => verify(rest),
            "load" => load(rest),
            "export" => export(rest),
            "import" => import(rest),
            other => Err(format!(
                "unknown save command '{other}' (try create, list, inspect, verify, load, export, import)"
            )),
        },
        None => Err("save needs a command: create, list, inspect, verify, load, export, import".into()),
    }
}

const SAVE_SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "name",
        "days",
        "ticks",
        "nudge",
        "slot",
        "dev-map",
        "dev-pawns",
        "content",
        "threads",
        "object",
        "put",
        "propose",
        "world",
        "out",
        "overwrite-as",
    ],
    switches: &["overwrite"],
    optional: &[],
};

fn dir_and_world(p: &crate::args::Parsed, need_world: bool) -> Result<(String, String), String> {
    let dir = p
        .positional
        .first()
        .ok_or("needs a save directory")?
        .clone();
    let world = match (p.positional.get(1).map(String::as_str), p.one("world")) {
        (Some(w), _) | (None, Some(w)) => w.to_owned(),
        (None, None) if need_world => return Err("needs a world id".into()),
        (None, None) => "world".to_owned(),
    };
    Ok((dir, world))
}

fn create(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let (dir, world_id) = dir_and_world(&p, false)?;
    let setup = demo_from_flags(&p)?;
    let total = match (p.parse::<u64>("days")?, p.parse::<u64>("ticks")?) {
        (Some(d), None) => d.checked_mul(TICKS_PER_DAY).ok_or("--days is too large")?,
        (None, Some(t)) => t,
        (None, None) => TICKS_PER_DAY,
        (Some(_), Some(_)) => return Err("use either --days or --ticks, not both".into()),
    };
    let mut sim = build_demo(&setup)?;
    sim.run_ticks(total).map_err(|e| e.to_string())?;
    if !sim.snapshot().pending.is_empty() {
        println!("note: inputs queued for future ticks are not part of a save and were dropped");
    }
    let storage = open(&dir)?;
    let report = SlotStore::new(&storage)
        .save(&world_id, sim.world(), &refs_of(&setup.content), &iso_now())
        .map_err(|e| e.to_string())?;
    println!(
        "saved world '{world_id}' to {dir}: generation {}, tick {}, {} -> {} bytes, hash {}",
        report.generation,
        sim.world().clock.tick(),
        report.uncompressed_bytes,
        report.compressed_bytes,
        short(&sim.world().state_hash())
    );
    for l in &report.leftovers {
        println!("warning: could not delete old file {l}");
    }
    Ok(ExitCode::SUCCESS)
}

fn list(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let dir = p.positional.first().ok_or("list needs a save directory")?;
    let storage = open(dir)?;
    let store = SlotStore::new(&storage);
    let ids = store.list_worlds().map_err(|e| e.to_string())?;
    if ids.is_empty() {
        println!("no worlds in {dir}");
    }
    for id in ids {
        match store.manifest(&id) {
            Some(m) => {
                let g = m.current().map_or_else(String::new, |g| {
                    format!(
                        "generation {} day {} tick {} saved {}",
                        g.generation, g.day, g.play_ticks, g.saved_iso
                    )
                });
                println!(
                    "  {id:<20} {:?} seed {:?}  {g}{}",
                    m.name,
                    m.seed_text,
                    if store.is_marked_damaged(&id) {
                        "  [DAMAGED]"
                    } else {
                        ""
                    }
                );
            }
            None => println!(
                "  {id:<20} (no valid manifest){}",
                if store.is_marked_damaged(&id) {
                    "  [DAMAGED]"
                } else {
                    ""
                }
            ),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn inspect(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let (dir, world) = dir_and_world(&p, true)?;
    let storage = open(&dir)?;
    let store = SlotStore::new(&storage);
    match store.manifest(&world) {
        None => println!("{world}: no valid manifest"),
        Some(m) => {
            println!(
                "{world}: {:?}  seed {:?}  schema {}",
                m.name, m.seed_text, m.schema
            );
            for r in &m.content_refs {
                println!(
                    "  content: {} {} {}",
                    r.pack_id,
                    r.version,
                    r.hash.chars().take(8).collect::<String>()
                );
            }
            for g in &m.generations {
                println!(
                    "  generation {}: day {} tick {} saved {} hash {}",
                    g.generation,
                    g.day,
                    g.play_ticks,
                    g.saved_iso,
                    g.state_hash.chars().take(8).collect::<String>()
                );
            }
        }
    }
    let files = storage
        .list(&format!("worlds/{world}/"))
        .map_err(|e| e.to_string())?;
    for f in files {
        let tail = f.name.rsplit('/').next().unwrap_or(&f.name);
        if tail.ends_with(".pgsave") {
            let bytes = storage
                .read(&f.name)
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            match codec::parse_header(&bytes) {
                Ok(h) => println!(
                    "  file {tail}: {} bytes, schema {}, {} bytes uncompressed, checksum {}",
                    f.size,
                    h.schema,
                    h.uncompressed_len,
                    h.checksum
                        .iter()
                        .take(4)
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                ),
                Err(e) => println!("  file {tail}: {} bytes, UNREADABLE HEADER ({e})", f.size),
            }
        } else {
            println!("  file {tail}: {} bytes", f.size);
        }
    }
    if store.is_marked_damaged(&world) {
        println!("  this slot is marked DAMAGED");
    }
    Ok(ExitCode::SUCCESS)
}

fn load_options(
    content: Option<&ContentSet>,
    setup_refs: Option<Vec<ContentRefRecord>>,
) -> LoadOptions<'_> {
    LoadOptions {
        content,
        installed_refs: setup_refs,
        ..LoadOptions::default()
    }
}

fn verify(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let (dir, world) = dir_and_world(&p, true)?;
    let content = content_from(&p)?;
    let storage = open(&dir)?;
    let store = SlotStore::new(&storage);
    let opts = load_options(
        content.as_deref(),
        content.as_ref().map(|_| refs_of(&content)),
    );
    let results = match store.verify(&world, &opts) {
        Ok(r) => r,
        Err(e) => return Err(e.to_string()),
    };
    let mut newest_ok = None;
    for (g, r) in &results {
        match r {
            Ok(()) => println!("  generation {g}: OK"),
            Err(why) => println!("  generation {g}: FAILED ({why})"),
        }
        newest_ok.get_or_insert(r.is_ok());
    }
    match store.load(&world, &opts) {
        Ok(l) => {
            match &l.recovery {
                Recovery::Clean => println!("load: clean (generation {})", l.generation),
                Recovery::FellBack { wanted, used, reason, ticks_lost } => println!(
                    "load: FELL BACK from generation {wanted} to {used} ({reason}); {} tick(s) lost",
                    ticks_lost.map_or("unknown".to_owned(), |t| t.to_string())
                ),
                Recovery::NoManifest { used } => println!("load: manifest missing; used newest valid generation {used}"),
            }
            if let Some(c) = &l.compat {
                for line in c.explain() {
                    println!("content: {line}");
                }
            }
            Ok(if matches!(l.recovery, Recovery::Clean) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Err(LoadError::Damaged(a)) => {
            println!("load: DAMAGED, unrecoverable:");
            for line in a {
                println!("  {line}");
            }
            Ok(ExitCode::FAILURE)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn content_from(p: &crate::args::Parsed) -> Result<Option<Arc<ContentSet>>, String> {
    let dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if dirs.is_empty() {
        Ok(None)
    } else {
        load_content(&dirs).map(Some)
    }
}

fn load(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let (dir, world) = dir_and_world(&p, true)?;
    let content = content_from(&p)?;
    let storage = open(&dir)?;
    let store = SlotStore::new(&storage);
    let loaded = store
        .load(&world, &load_options(content.as_deref(), None))
        .map_err(|e| e.to_string())?;
    println!(
        "loaded '{world}' generation {}: tick {}, day {}, {} pawn(s), hash {} ({:?})",
        loaded.generation,
        loaded.world.clock.tick(),
        loaded.world.clock.day(),
        loaded.world.pawns.len(),
        short(&loaded.world.state_hash()),
        loaded.recovery
    );
    if let Some(from) = loaded.migrated_from {
        println!("migrated from schema {from}");
    }
    if let Some(ticks) = p.parse::<u64>("ticks")? {
        let mut sim = Sim::restore(
            SimSnapshot {
                world: loaded.world,
                pending: Vec::new(),
                next_seq: 0,
            },
            dev_pipeline(),
        );
        if let Some(c) = content {
            sim = sim.with_content(c);
        }
        sim.run_ticks(ticks).map_err(|e| e.to_string())?;
        println!(
            "ran {ticks} more tick(s): tick {}, hash {}",
            sim.world().clock.tick(),
            short(&sim.world().state_hash())
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn export(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let (dir, world) = dir_and_world(&p, true)?;
    let out = p.one("out").ok_or("export needs --out <file>")?;
    let storage = open(&dir)?;
    let store = SlotStore::new(&storage);
    let loaded = store
        .load(&world, &LoadOptions::default())
        .map_err(|e| e.to_string())?;
    let refs = loaded
        .manifest
        .as_ref()
        .map_or_else(Vec::new, |m| m.content_refs.clone());
    let bytes = export_world(&loaded.world, &refs, APP_VERSION, &iso_now());
    std::fs::write(out, &bytes).map_err(|e| format!("cannot write {out}: {e}"))?;
    println!("exported '{world}' to {out} ({} bytes)", bytes.len());
    Ok(ExitCode::SUCCESS)
}

fn import(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SAVE_SPEC)?;
    let dir = p
        .positional
        .first()
        .ok_or("import needs a save directory")?;
    let file = p.positional.get(1).ok_or("import needs the export file")?;
    let world = p.one("world").ok_or("import needs --world <new id>")?;
    let content = content_from(&p)?;
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let opts = ImportOptions {
        content: content.as_deref(),
        installed_refs: content.as_ref().map(|_| refs_of(&content)),
        ..ImportOptions::default()
    };
    let plan = import_world(&bytes, &opts).map_err(|e| e.to_string())?;
    println!(
        "valid export of {:?} made by version {} at {} (tick {}, {} pawn(s))",
        plan.world.meta.name,
        plan.app_version,
        plan.created_iso,
        plan.world.clock.tick(),
        plan.world.pawns.len()
    );
    if let Some(c) = &plan.compat {
        for line in c.explain() {
            println!("content: {line}");
        }
    }
    let storage = open(dir)?;
    let store = SlotStore::new(&storage);
    let r = commit_import(&store, world, &plan, &iso_now(), p.has("overwrite"))
        .map_err(|e| e.to_string())?;
    println!(
        "written as new world '{world}' (generation {})",
        r.generation
    );
    Ok(ExitCode::SUCCESS)
}

// ---- bug bundles -------------------------------------------------------------------------------------

const BUNDLE_SPEC: Spec<'static> = Spec {
    values: &["at", "note", "out", "content"],
    switches: &[],
    optional: &[],
};

pub fn bugbundle_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, rest)) if c == "create" => bundle_create(rest),
        Some((c, rest)) if c == "run" => bundle_run(rest),
        _ => Err("usage: pg bugbundle create <log> --at <tick> --out <file.pgbundle> [--note <text>] | pg bugbundle run <file.pgbundle>".into()),
    }
}

fn content_for_log(
    log: &pg_core::replay::ReplayLog,
    p: &crate::args::Parsed,
) -> Result<Option<Arc<ContentSet>>, String> {
    let dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if !dirs.is_empty() {
        return load_content(&dirs).map(Some);
    }
    if log.content.is_empty() {
        return Ok(None);
    }
    load_content(&[DEFAULT_CONTENT_DIR.to_owned()]).map(Some)
}

fn bundle_create(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &BUNDLE_SPEC)?;
    let path = p.positional.first().ok_or("create needs a replay log")?;
    let at: u64 = p.parse("at")?.ok_or("create needs --at <tick>")?;
    let out = p.one("out").ok_or("create needs --out <file>")?;
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let log = read_any(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let content = content_for_log(&log, &p)?;
    let bundle = Bundle::make(
        &log,
        at,
        content,
        p.one("note").unwrap_or(""),
        &iso_now(),
        APP_VERSION,
    )
    .map_err(|e| e.to_string())?;
    let packed = bundle.encode();
    std::fs::write(out, &packed).map_err(|e| format!("cannot write {out}: {e}"))?;
    println!(
        "bundle written to {out}: starts at tick {at}, {} tick(s) to replay, {} bytes",
        bundle.log.ticks,
        packed.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn bundle_run(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &BUNDLE_SPEC)?;
    let path = p.positional.first().ok_or("run needs a bundle file")?;
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let bundle = Bundle::decode(&bytes).map_err(|e| format!("{path}: {e}"))?;
    println!(
        "bundle from version {} at {}: {:?}",
        bundle.app_version, bundle.created_iso, bundle.note
    );
    println!(
        "world {:?}, starts at tick {}, replays {} tick(s), {} day hash(es) to check",
        bundle.log.world_name,
        bundle.log.start_tick(),
        bundle.log.ticks,
        bundle.log.day_hashes.len()
    );
    let content = content_for_log(&bundle.log, &p)?;
    let outcome = replay(&bundle.log, content).map_err(|e| e.to_string())?;
    for m in &outcome.mismatches {
        println!(
            "  MISMATCH {:?}: expected {} actual {} (tables: {})",
            m.day,
            m.expected,
            m.actual,
            m.tables.join(", ")
        );
    }
    println!(
        "{}",
        if outcome.ok() {
            "BUNDLE REPLAY OK"
        } else {
            "BUNDLE REPLAY FAILED"
        }
    );
    Ok(if outcome.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

// ---- scenarios ---------------------------------------------------------------------------------------

pub fn scenario_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, files)) if c == "run" && !files.is_empty() => {
            let mut all_ok = true;
            for file in files {
                let text = std::fs::read_to_string(file)
                    .map_err(|e| format!("cannot read {file}: {e}"))?;
                let sc = scenario::Scenario::parse(&text).map_err(|e| format!("{file}: {e}"))?;
                println!(
                    "scenario {:?} ({file}): {} step(s)",
                    sc.name,
                    sc.steps.len()
                );
                let report = scenario::run(&sc);
                for o in &report.outcomes {
                    println!(
                        "  {:>2} {:<14} {}  {}",
                        o.index + 1,
                        o.op,
                        if o.ok { "ok  " } else { "FAIL" },
                        o.message
                    );
                }
                if report.ok() {
                    println!(
                        "  PASSED  final hash {}",
                        &report.final_hash[..8.min(report.final_hash.len())]
                    );
                } else {
                    println!("  FAILED");
                    all_ok = false;
                }
            }
            Ok(if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        _ => Err("usage: pg scenario run <file.json>...".into()),
    }
}
