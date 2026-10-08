//! `pg`: headless developer tooling (Blueprint §20). Developer-only; never part of the shipped game.
//!
//! Commands so far: `selftest`, `rand`, `hash`, `id` (0.1); `sim`, `replay`, `time`, `pipeline` (0.2);
//! `content` (0.3); `map`, plus replay `--diff` / `--bisect` and event filters (0.4); `schedule explain`,
//! `actions` (0.5); `save`, `bugbundle`, `scenario`, `content diff` (0.6); `ai`, `check` (0.7). Later milestones add
//! `pack`, `bench`.

use pg_core::hash::hash_canon;
use pg_core::id::{EntityId, Kind};
use pg_core::rng::{Key, Rng, Seed, Stream};
use pg_core::vectors;
use std::process::ExitCode;

mod ai_cmds;
mod args;
mod check_cmds;
mod content_cmds;
mod events_cmds;
mod map_cmds;
mod pack_cmds;
mod persist_cmds;
mod residents_cmds;
mod runtime_cmds;
mod sched_cmds;
mod settings_cmds;
mod shared;
mod sim_cmds;
mod strings_cmds;
mod worldgen_cmds;

const USAGE: &str = "\
pg - Playground developer tool

USAGE:
    pg selftest
        Run the determinism test vectors (published and pinned). Exit 1 on any failure.
    pg rand <seed-text> <stream> [--key <k>]... [--count <n>] [--range <n>] [--chance <permille>]
        Print draws for (seed, stream, keys). Keys: an integer, an entity id (pawn_1a), or text;
        prefix with int: / id: / str: to force a type.
    pg hash <file.json> [--show]
        Parse JSON (integers only), print its canonical form (--show) and its BLAKE3 state hash.
    pg sim --seed <text> [--needs] [--days <n> | --ticks <n>] [--name <text>] [--dev-map WxH[:style]]
           [--dev-pawns <n>] [--content <pack-dir>]... [--threads <n>] [--nudge <tick>:<amount>]...
           [--object <template>@x,y]... [--put <child>:<owner>:<container>]...
           [--propose <tick>:<proposer>:<invitee>:<start>:<len>:<x>,<y>]...
           [--slot <tick>:<minutes>]... [--events [kind-prefix]] [--since <tick>] [--until <tick>]
           [--log <file>] [--pretty]
        Run the dev simulation headless; print the state hash at every day boundary (with per-table
        hashes) and optionally write a replay log. --dev-map creates a generated town (style 1) or open
        grass (style 0) with --dev-pawns wandering pawns. --events prints events, filtered by kind prefix
        and tick range.
    pg replay <log.json> [--content <pack-dir>]... [--snapshot-at <tick>]
        Re-run a replay log and verify every recorded hash (combined and per table). With --snapshot-at,
        also verify that snapshotting at that tick and resuming gives an identical result. Exit 1 on any
        mismatch.
    pg replay --diff <a.json> <b.json>
        Compare two logs from their recorded data: first differing input, day and table.
    pg replay --bisect <a.json> <b.json>
        Re-run both logs in lockstep and report the exact tick (and table) where they first differ.
    pg map show [--seed <text>] [--dev-map WxH[:style]] [--dev-pawns <n>] [--ticks <n>] [--no-routes]
        Draw a demo map after N ticks: terrain, objects, pawns and their routes, plus a pawn table.
    pg map path --from x,y --to x,y [--size WxH[:style]] [--seed <text>] [--cap <n>]
        Run one path request on a demo map and draw the route.
    pg map bench-paths [--size WxH[:style]] [--requests <n>] [--threads 1,2,4,8] [--seed <text>]
        Solve the same batch of requests serially and on several thread counts; verifies identical results.
    pg schedule explain <pawn> --seed <text> --dev-map WxH --dev-pawns <n> [--day <n>] [--at <tick>]
                           [--propose ...]... [other sim flags]
        Run the dev simulation to a tick (default: just after the day's first slot boundary) and print
        the pawn's day: every reservation with its priority and the plain-sentence reason it is there,
        open time, what could not be placed and why, and the pawn's commitments.
    pg actions                List the closed action registry: ids, parameters, steps.
    pg save create <dir> [--world <id>] --seed <text> [--days <n> | --ticks <n>] [sim flags]
        Run the dev simulation and save it as a world slot (two generations, atomic writes).
    pg save list <dir>                      List the worlds in a save directory.
    pg save inspect <dir> <world>           Manifest, generations, file headers.
    pg save verify <dir> <world> [--content <pack-dir>]...
        Check every generation (checksum, decode, hash, containment) and the load path. Exit 1 if the
        newest generation is not clean.
    pg save load <dir> <world> [--ticks <n>] [--content <pack-dir>]...
        Load a world (recovering from damage if needed) and optionally keep running it.
    pg save export <dir> <world> --out <file>     Write a portable export.
    pg save import <dir> <file> --world <new-id> [--overwrite]   Validate an export and write it as a new slot.
    pg bugbundle create <log> --at <tick> --out <file.pgbundle> [--note <text>]
        Cut a replay log at a tick: a snapshot plus the tail, small enough to attach to a bug report.
    pg bugbundle run <file.pgbundle>        Replay a bundle and verify its hashes.
    pg scenario run <file.json>...          Run scenario files (build, run, save, reload, damage, recover, assert).
    pg content diff <packs-a> <packs-b>     Compare two content sets (comma-separated pack dirs each).
    pg ai providers                         The five providers, their hosts, recommended models and whether a key is stored.
    pg ai key set <provider> [--from-env VAR] | clear <provider> | status
        Manage provider keys in the OS credential store (never in files).
    pg ai settings show | set [--provider P] [--model M | --default-model] [--enable | --disable] [--dir D]
    pg ai test [--provider P] [--model M] [--dry-run]
        Run the connection test, or with --dry-run print the exact request (credentials hidden) without sending it.
    pg ai selfcheck                         Run the AI client with a sentinel key and scan everything for leaks.
    pg run [--dir D] [--world id] [--seed s --dev-map WxH --dev-pawns N] [--seconds S]
           [--speed 1x|3x|9x|27x] [--shadow N] [--autosave-minutes M] [--scrub TICK] [--content dir]
        Run a world on the simulation thread in real time with autosave, optional shadow
        verification and a final save; creates the slot if absent, loads it if present.
    pg profile [--dev-map WxH] [--dev-pawns N] [--days D | --ticks N] [--threads N]
        Time every system over a headless run (default 200 pawns, one day); prints the table.
    pg pack lint <dir> [--base dir] [--with dir]...
        Load a pack with the base pack, lint its scripts (API names, capabilities, order-sensitive
        iteration, module-level state) and start it in the script host.
    pg pack test <dir> [--days N] [--pawns N] [--update] [--with dir]...
        Run the pack headless under golden hashes (packs/golden/<id>.json), then again rebuilding every VM at
        each day boundary; both must agree.
    pg pack docs [--luau]     The generated API reference, or pg.d.luau for editors.
    pg pack new <id> <dir>    Scaffold a pack.
    pg content schema [manifest|templates]   JSON Schema for editors.
    pg tools                  The developer-tool registry (command-line and overlay tools).
    pg events list [category] | show <kind> The typed event catalog: kinds, categories, fields, default visibility.
    pg settings list | get <id> | set <id> <value> | reset <id> [--dir D]
                                            The device settings registry: typed, ranged, validated (default dir ./pg-data).
    pg strings lint [pack-dir...]           Check string tables: missing, orphaned and mismatched keys, unused sentences.
    pg strings show <key> [--locale L] [--param name=value]...   Look up one string (pseudo locale supported).
    pg strings pseudo <text>                Show the pseudo-locale rendering of a text.
    pg check [--verbose]                    Run the developer checks (vectors, content, golden replay, scenarios,
                                            saves, bundles, redaction) and print one PASS/FAIL report.
    pg time <tick> [--slot-minutes <m>]    Show day / clock time / slot / boundary flags for a tick.
    pg pipeline               Show the tick pipeline: systems in execution order and their cadence.
    pg content lint [pack-dir...]         Load and validate packs (default: data/base). Exit 1 on errors.
    pg content tree [--dot] [pack-dir...] Which template extends which, with each pack (or Graphviz).
    pg worldgen preview [--seed S] [--size WxH] [--water PERCENT] [--residents N] [--tone T] [--no-map] [--expect HASH]
                                          Generate a town and draw it: districts, roads, buildings, plazas, homes.
    pg residents generate [--seed S] [--count N] [--content dir]...
                                          The people a seed produces: households, occupations, starting
                                          relationships and shared memories.
    pg content list [pack-dir...]         List every resolved template: pack, chain depth, tags, components.
    pg content resolve <id> [pack-dir...] Show a template's inheritance chain, which template sets each
                                          component, and the fully resolved result.
    pg content components                 List the registered components and their parameter schemas.
    pg id <text>              Parse an id such as pawn_1a and show its parts.
    pg id <kind> <counter>    Format an id from a kind and a decimal counter.
    pg version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("selftest") => Ok(selftest()),
        Some("rand") => rand_cmd(&args[1..]),
        Some("hash") => hash_cmd(&args[1..]),
        Some("id") => id_cmd(&args[1..]),
        Some("content") => content_cmds::content_cmd(&args[1..]),
        Some("residents") => residents_cmds::residents_cmd(&args[1..]),
        Some("worldgen") => worldgen_cmds::worldgen_cmd(&args[1..]),
        Some("map") => map_cmds::map_cmd(&args[1..]),
        Some("sim") => sim_cmds::sim_cmd(&args[1..]),
        Some("replay") => sim_cmds::replay_cmd(&args[1..]),
        Some("time") => sim_cmds::time_cmd(&args[1..]),
        Some("pipeline") => Ok(sim_cmds::pipeline_cmd()),
        Some("schedule") => sched_cmds::schedule_cmd(&args[1..]),
        Some("save") => persist_cmds::save_cmd(&args[1..]),
        Some("ai") => ai_cmds::ai_cmd(&args[1..]),
        Some("strings") => strings_cmds::strings_cmd(&args[1..]),
        Some("events") => events_cmds::events_cmd(&args[1..]),
        Some("pack") => pack_cmds::pack_cmd(&args[1..]),
        Some("profile") => runtime_cmds::profile_cmd(&args[1..]),
        Some("run") => runtime_cmds::run_cmd(&args[1..]),
        Some("tools") => Ok(runtime_cmds::tools_cmd()),
        Some("settings") => settings_cmds::settings_cmd(&args[1..]),
        Some("check") => check_cmds::check_cmd(&args[1..]),
        Some("bugbundle") => persist_cmds::bugbundle_cmd(&args[1..]),
        Some("scenario") => persist_cmds::scenario_cmd(&args[1..]),
        Some("actions") => Ok(sched_cmds::actions_cmd()),
        Some("version" | "--version" | "-V") => {
            println!("pg {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("unknown command '{other}'\n\n{USAGE}")),
    };
    match result {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

fn selftest() -> ExitCode {
    let results = vectors::run();
    let mut failed = 0;
    for r in &results {
        let tag = match r.origin {
            vectors::Origin::Published => "published",
            vectors::Origin::Pinned => "pinned   ",
        };
        if r.passed() {
            println!("PASS  {tag}  {}", r.name);
        } else {
            failed += 1;
            println!(
                "FAIL  {tag}  {}\n        expected {}\n        actual   {}",
                r.name, r.expected, r.actual
            );
        }
    }
    println!("\n{} vectors, {} failed", results.len(), failed);
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// An owned key parsed from the command line.
#[derive(Debug, PartialEq, Eq)]
enum OwnedKey {
    Int(i64),
    Str(String),
    Id(EntityId),
}

fn parse_key(text: &str) -> Result<OwnedKey, String> {
    if let Some(rest) = text.strip_prefix("int:") {
        return rest
            .parse()
            .map(OwnedKey::Int)
            .map_err(|e| format!("bad int key '{rest}': {e}"));
    }
    if let Some(rest) = text.strip_prefix("id:") {
        return rest
            .parse()
            .map(OwnedKey::Id)
            .map_err(|e| format!("bad id key '{rest}': {e}"));
    }
    if let Some(rest) = text.strip_prefix("str:") {
        return Ok(OwnedKey::Str(rest.to_owned()));
    }
    if let Ok(n) = text.parse::<i64>() {
        return Ok(OwnedKey::Int(n));
    }
    if let Ok(id) = text.parse::<EntityId>() {
        return Ok(OwnedKey::Id(id));
    }
    Ok(OwnedKey::Str(text.to_owned()))
}

fn flag_value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    args.get(*i)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn rand_cmd(args: &[String]) -> Result<ExitCode, String> {
    let (seed_text, stream) = match (args.first(), args.get(1)) {
        (Some(a), Some(b)) => (a.as_str(), b.as_str()),
        _ => return Err(format!("rand needs <seed-text> <stream>\n\n{USAGE}")),
    };
    let mut keys = Vec::new();
    let (mut count, mut range, mut chance) = (8u32, None::<u32>, None::<i32>);
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--key" => keys.push(parse_key(flag_value(args, &mut i, "--key")?)?),
            "--count" => {
                count = flag_value(args, &mut i, "--count")?
                    .parse()
                    .map_err(|e| format!("--count: {e}"))?
            }
            "--range" => {
                range = Some(
                    flag_value(args, &mut i, "--range")?
                        .parse()
                        .map_err(|e| format!("--range: {e}"))?,
                )
            }
            "--chance" => {
                chance = Some(
                    flag_value(args, &mut i, "--chance")?
                        .parse()
                        .map_err(|e| format!("--chance: {e}"))?,
                )
            }
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 1;
    }
    let borrowed: Vec<Key<'_>> = keys
        .iter()
        .map(|k| match k {
            OwnedKey::Int(n) => Key::Int(*n),
            OwnedKey::Str(s) => Key::Str(s),
            OwnedKey::Id(id) => Key::Id(*id),
        })
        .collect();
    let seed = Seed::from_text(seed_text);
    let rng = Rng::new(seed, Stream::parse(stream), &borrowed);
    println!(
        "seed {seed_text:?} -> {:016x}   stream {stream:?}   keys {keys:?}",
        seed.0
    );
    let chance = chance
        .map(|p| pg_core::num::Permille::new(p).map_err(|e| format!("--chance: {e}")))
        .transpose()?;
    for c in 0..count {
        let mut line = format!("[{c:>4}]  draw {:08x}", rng.draw(c));
        if let Some(n) = range {
            match rng.range(c, n) {
                Some(v) => line.push_str(&format!("   range({n}) = {v}")),
                None => line.push_str("   range: n must be > 0"),
            }
        }
        if let Some(p) = chance {
            line.push_str(&format!("   chance({p}) = {}", rng.chance(c, p)));
        }
        println!("{line}");
    }
    Ok(ExitCode::SUCCESS)
}

fn hash_cmd(args: &[String]) -> Result<ExitCode, String> {
    let path = args
        .first()
        .ok_or_else(|| format!("hash needs <file.json>\n\n{USAGE}"))?;
    let show = args.iter().skip(1).any(|a| a == "--show");
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let canon = pg_core::canon::json::parse(&text).map_err(|e| format!("{path}: {e}"))?;
    if show {
        println!("{}", canon.to_canonical_string());
    }
    println!("{}", hash_canon(&canon));
    Ok(ExitCode::SUCCESS)
}

fn id_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args {
        [text] => {
            let id: EntityId = text.parse().map_err(|e| format!("{e}"))?;
            println!(
                "{id}  kind={} (code {})  counter={} (decimal)",
                id.kind().prefix(),
                id.kind() as u8,
                id.counter()
            );
            Ok(ExitCode::SUCCESS)
        }
        [kind, counter] => {
            let kind = Kind::from_prefix(kind).ok_or_else(|| format!("unknown kind '{kind}'"))?;
            let n: u32 = counter.parse().map_err(|e| format!("counter: {e}"))?;
            println!("{}", EntityId::new(kind, n));
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(format!("id needs <text> or <kind> <counter>\n\n{USAGE}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_with_inference_and_overrides() {
        assert_eq!(parse_key("42"), Ok(OwnedKey::Int(42)));
        assert_eq!(parse_key("-7"), Ok(OwnedKey::Int(-7)));
        assert_eq!(
            parse_key("pawn_1a"),
            Ok(OwnedKey::Id(EntityId::new(Kind::Pawn, 46)))
        );
        assert_eq!(parse_key("hello"), Ok(OwnedKey::Str("hello".into())));
        assert_eq!(parse_key("str:42"), Ok(OwnedKey::Str("42".into())));
        assert_eq!(parse_key("int:9"), Ok(OwnedKey::Int(9)));
        assert!(parse_key("int:x").is_err());
        assert!(parse_key("id:nope").is_err());
    }

    #[test]
    fn the_strict_parser_accepts_integers_and_rejects_floats_and_duplicate_keys() {
        use pg_core::canon::json::parse;
        assert_eq!(
            parse(r#"{"b":[1,-2,true,null],"a":"x"}"#)
                .unwrap()
                .to_canonical_string(),
            r#"{"a":"x","b":[1,-2,true,null]}"#
        );
        assert_eq!(
            parse("18446744073709551615").unwrap().to_canonical_string(),
            "18446744073709551615"
        );
        for bad in ["1.5", "1e3", r#"{"x":[0.1]}"#, r#"{"a":1,"a":2}"#] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_key_order_does_not_change_the_hash() {
        use pg_core::canon::json::parse;
        let a = parse(r#"{"x":1,"y":2}"#).unwrap();
        let b = parse(r#"{ "y": 2, "x": 1 }"#).unwrap();
        assert_eq!(hash_canon(&a), hash_canon(&b));
    }
}
