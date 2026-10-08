//! `pg pack`: authoring tools for packs with scripts (Blueprint §23.12), and `pg content schema`.
//!
//! * `pg pack lint <dir> [--base dir]`: load the pack with the base pack, run static checks over its
//!   scripts, start it in the script host and report everything a load would hit.
//! * `pg pack test <dir> [--days N] [--pawns N] [--update]`: run the pack in a headless world under golden
//!   hashes, then again rebuilding every VM at each day boundary (the VM-reload variant); both must agree.
//! * `pg pack docs [--luau]`: the generated API reference, or `pg.d.luau`.
//! * `pg pack new <id> <dir>`: scaffold a pack.
//! * `pg content schema [manifest|templates]`: JSON Schema for editors.

use crate::args::{parse, Spec};
use crate::shared::short;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits, LoadedPack};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::time::TICKS_PER_DAY;
use pg_core::world::WorldState;
use pg_runtime::keyframes::SimFactory;
use pg_script::host::{ScriptHost, ScriptLimits, ScriptPackInput};
use std::process::ExitCode;
use std::sync::Arc;

const SPEC: Spec<'static> = Spec {
    values: &["base", "days", "pawns", "with", "seed"],
    switches: &["update", "luau"],
    optional: &[],
};

pub fn pack_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("lint") => lint(&args[1..]),
        Some("test") => test(&args[1..]),
        Some("docs") => docs(&args[1..]),
        Some("new") => new_pack(&args[1..]),
        Some("bench") => bench(&args[1..]),
        Some(other) => Err(format!(
            "unknown pack command '{other}' (try lint, test, docs, new)"
        )),
        None => Err("pack needs a command: lint, test, docs, new".into()),
    }
}

pub fn schema_cmd(args: &[String]) -> Result<ExitCode, String> {
    let canon = match args.first().map(String::as_str) {
        None | Some("manifest") => pg_content::jsonschema::manifest_schema(),
        Some("templates") => pg_content::jsonschema::template_schema(&ComponentRegistry::builtin()),
        Some(other) => {
            return Err(format!(
                "unknown schema '{other}' (try manifest or templates)"
            ))
        }
    };
    let value: serde_json::Value =
        serde_json::from_str(&canon.to_canonical_string()).map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(ExitCode::SUCCESS)
}

fn docs(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    if p.has("luau") {
        print!("{}", pg_api::render_luau_defs());
    } else {
        print!("{}", pg_api::render_docs());
    }
    Ok(ExitCode::SUCCESS)
}

fn load(dir: &str) -> Result<LoadedPack, String> {
    load_pack(&DirPack::new(dir), &Limits::default())
        .map_err(|r| format!("cannot load pack '{dir}':\n{r}"))
}

/// The base pack, the pack under test and any `--with` packs, built into one content set.
fn build_set(dir: &str, p: &crate::args::Parsed) -> Result<Arc<ContentSet>, String> {
    let base = p.one("base").unwrap_or(crate::shared::DEFAULT_CONTENT_DIR);
    let mut packs = vec![load(base)?];
    for extra in p.all("with") {
        packs.push(load(extra)?);
    }
    packs.push(load(dir)?);
    ContentSet::build(packs, ComponentRegistry::builtin())
        .map(Arc::new)
        .map_err(|r| format!("content did not build:\n{r}"))
}

fn script_inputs(set: &ContentSet) -> Vec<ScriptPackInput> {
    set.script_packs()
        .iter()
        .map(ScriptPackInput::from_content)
        .collect()
}

// ---- static lint --------------------------------------------------------------------------------------------

/// A finding of the static script lint.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub error: bool,
    pub message: String,
}

fn strip_comment(line: &str) -> &str {
    // Good enough for a lint: a `--` inside a string is rare in scripts and only costs a missed finding.
    line.split("--").next().unwrap_or(line)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Every `pg.a.b` path on a line (identifiers separated by dots).
fn pg_paths(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let starts = bytes.get(i..i + 3) == Some(&['p', 'g', '.']);
        let boundary = i == 0
            || bytes
                .get(i - 1)
                .is_none_or(|c| !is_ident_char(*c) && *c != '.');
        if starts && boundary {
            let mut j = i;
            let mut path = String::new();
            while let Some(c) = bytes.get(j) {
                if is_ident_char(*c) || *c == '.' {
                    path.push(*c);
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(path.trim_end_matches('.').to_owned());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Static checks over one script file. `capabilities` are the pack's declared ones.
pub fn lint_source(file: &str, source: &str, capabilities: &[&str]) -> Vec<Finding> {
    let mut out = Vec::new();
    let functions = pg_api::function_paths();
    let mut modules: Vec<String> = pg_api::FUNCTIONS
        .iter()
        .filter(|f| !f.module.is_empty())
        .map(|f| format!("pg.{}", f.module))
        .collect();
    modules.sort();
    modules.dedup();
    let mut top_locals: Vec<String> = Vec::new();
    for (n, raw) in source.lines().enumerate() {
        let line_no = n + 1;
        let line = strip_comment(raw);
        let mut push = |error: bool, message: String| {
            out.push(Finding {
                file: file.to_owned(),
                line: line_no,
                error,
                message,
            });
        };
        // 1. `pg` paths: known, and allowed by the capabilities.
        for path in pg_paths(line) {
            if path == "pg" {
                continue;
            }
            if let Some(f) = pg_api::FUNCTIONS.iter().find(|f| f.path() == path) {
                if let Some(c) = f.capability {
                    if !capabilities.contains(&c) {
                        push(
                            true,
                            format!("{path} needs the '{c}' capability, which the manifest does not declare"),
                        );
                    }
                }
            } else if !modules.contains(&path)
                && !functions.iter().any(|f| f.starts_with(&format!("{path}.")))
            {
                let hint = pg_api::unknown_path_hint(&path);
                push(true, format!("{path} is not part of the pg API{hint}"));
            }
        }
        // 2. Order-sensitive iteration.
        if let Some(pos) = line.find("for ") {
            if let Some(inpos) = line[pos..].find(" in ") {
                let rest = line[pos + inpos + 4..].trim_start();
                let ok = rest.starts_with("pairs(")
                    || rest.starts_with("ipairs(")
                    || rest.starts_with("next")
                    || rest.contains("gmatch(")
                    || rest.starts_with("string.");
                if !ok {
                    push(
                        false,
                        "iterating a table directly has no guaranteed order; use pairs() (deterministic) or ipairs()".into(),
                    );
                }
            }
        }
        // 3. Things that are simply not there.
        for gone in [
            "os.",
            "io.",
            "debug.",
            "coroutine.",
            "loadstring",
            "getfenv",
            "setfenv",
            "collectgarbage",
            "math.random",
            "math.sin",
            "math.cos",
            "math.sqrt",
        ] {
            if line.contains(gone) {
                let before = line.find(gone).and_then(|p| line[..p].chars().last());
                if before.is_none_or(|c| !is_ident_char(c)) {
                    push(
                        true,
                        format!(
                            "'{}' does not exist in the sandbox",
                            gone.trim_end_matches('.')
                        ),
                    );
                }
            }
        }
        // 4. Module-level mutable state (the VM-reload test catches what this misses).
        let trimmed = line.trim_start();
        let indented = line.len() != trimmed.len();
        if !indented {
            if let Some(rest) = trimmed.strip_prefix("local ") {
                let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
                let value = rest[name.len()..].trim_start();
                let mutable_init = value.starts_with("= 0")
                    || value.starts_with("= {}")
                    || value.starts_with("= false")
                    || value.starts_with("= nil");
                if !name.is_empty() && mutable_init && !trimmed.contains("function") {
                    top_locals.push(name);
                }
            }
        } else {
            for name in &top_locals {
                let assigns = [" = ", " += ", " -= "]
                    .iter()
                    .any(|op| trimmed.starts_with(&format!("{name}{op}")))
                    || (trimmed.starts_with(&format!("{name}."))
                        || trimmed.starts_with(&format!("{name}[")))
                        && trimmed.contains(" = ");
                if assigns && !trimmed.starts_with("local ") {
                    push(
                        false,
                        format!("'{name}' is module-level state changed inside a function: it is lost when the VM is rebuilt, which breaks replay (keep state in components)"),
                    );
                }
            }
        }
    }
    out
}

fn lint(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let [dir] = p.positional.as_slice() else {
        return Err("usage: pg pack lint <pack-dir> [--base dir] [--with dir]...".into());
    };
    let set = build_set(dir, &p)?;
    let pack_id = set
        .load_order()
        .last()
        .map(|i| i.to_string())
        .unwrap_or_default();
    println!(
        "pack '{pack_id}': manifest and data OK ({} template(s) in the set)",
        set.len()
    );
    let mut errors = 0;
    let mut warnings = 0;
    for sp in set.script_packs().iter().filter(|s| s.id == pack_id) {
        for (file, source) in sp.sources.iter() {
            for f in lint_source(file, source, &sp.capabilities) {
                println!(
                    "{} {}:{}: {}",
                    if f.error { "error  " } else { "warning" },
                    f.file,
                    f.line,
                    f.message
                );
                if f.error {
                    errors += 1;
                } else {
                    warnings += 1;
                }
            }
        }
    }
    let host = ScriptHost::new(&script_inputs(&set), 1, ScriptLimits::default());
    for f in host.failures() {
        println!("error   load of '{}' failed: {}", f.pack, f.message);
        errors += 1;
    }
    for d in host.schemas().iter().filter(|d| d.pack == pack_id) {
        println!(
            "component {} ({} field(s), applies to {:?})",
            d.name,
            d.fields.len(),
            d.applies_to
        );
    }
    println!(
        "{} error(s), {warnings} warning(s); load fuel used {}",
        errors,
        host.fuel_used()
    );
    Ok(if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

// ---- headless test under golden hashes --------------------------------------------------------------------------

fn hashes_of(
    factory: &SimFactory,
    seed: &str,
    pawns: u32,
    days: u64,
    reload: bool,
) -> Result<Vec<String>, String> {
    let mut sim = factory.new_sim(WorldState::new("Pack Test", seed));
    let cmd = |c| SimInput::Command {
        actor: None,
        cmd: c,
    };
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 40,
            h: 30,
            style: 1,
        }),
    )
    .map_err(|e| e.to_string())?;
    for i in 0..pawns {
        sim.submit(
            0,
            cmd(Command::DevSpawnPawn {
                map: EntityId::new(Kind::Map, 1),
                at: None,
                name: format!("P{}", i + 1),
            }),
        )
        .map_err(|e| e.to_string())?;
    }
    let mut out = Vec::new();
    for _ in 0..days {
        sim.run_ticks(TICKS_PER_DAY).map_err(|e| e.to_string())?;
        out.push(sim.world().state_hash().to_hex());
        if reload {
            // Throw every VM away and carry on with new ones.
            sim = factory.restore(sim.snapshot());
        }
    }
    Ok(out)
}

fn test(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let [dir] = p.positional.as_slice() else {
        return Err(
            "usage: pg pack test <pack-dir> [--days N] [--pawns N] [--update] [--with dir]..."
                .into(),
        );
    };
    let set = build_set(dir, &p)?;
    let pack_id = set
        .load_order()
        .last()
        .map(|i| i.to_string())
        .unwrap_or_default();
    let days = p.parse::<u64>("days")?.unwrap_or(3);
    let pawns = p.parse::<u32>("pawns")?.unwrap_or(10);
    let seed = p.one("seed").unwrap_or("pack-test");
    let factory = SimFactory::dev(Some(set), 1);
    if !factory.runs_scripts() {
        println!("note: the pack has no scripts; running it as data only");
    }
    let straight = hashes_of(&factory, seed, pawns, days, false)?;
    let reloaded = hashes_of(&factory, seed, pawns, days, true)?;
    for (d, h) in straight.iter().enumerate() {
        println!("day {}: {}", d + 1, &h[..16]);
    }
    if straight != reloaded {
        println!("FAILED: rebuilding every VM at the day boundary changed the result (hidden script state?)");
        return Ok(ExitCode::FAILURE);
    }
    println!("VM-reload variant: identical");
    let golden_path = std::path::Path::new("packs/golden").join(format!("{pack_id}.json"));
    let text = format!(
        "{{\"pack\":\"{pack_id}\",\"seed\":\"{seed}\",\"pawns\":{pawns},\"days\":{days},\"hashes\":[{}]}}\n",
        straight.iter().map(|h| format!("\"{h}\"")).collect::<Vec<_>>().join(",")
    );
    if p.has("update") {
        std::fs::create_dir_all("packs/golden").map_err(|e| e.to_string())?;
        std::fs::write(&golden_path, text).map_err(|e| e.to_string())?;
        println!("golden updated: {}", golden_path.display());
        return Ok(ExitCode::SUCCESS);
    }
    match std::fs::read_to_string(&golden_path) {
        Ok(existing) if existing.trim() == text.trim() => {
            println!("PASSED: matches {}", golden_path.display());
            Ok(ExitCode::SUCCESS)
        }
        Ok(_) => {
            println!(
                "FAILED: differs from {} (run with --update if the change is intended)",
                golden_path.display()
            );
            Ok(ExitCode::FAILURE)
        }
        Err(_) => {
            println!(
                "no golden file yet at {}; run with --update to record one (final hash {})",
                golden_path.display(),
                short_hex(straight.last())
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// `pg pack bench`: what scripts cost (spike question 4). Run it with `--release`.
fn bench(args: &[String]) -> Result<ExitCode, String> {
    use pg_script::{Fuel, LuauVm, ScriptVm, SourceChunk, Val, VmConfig};
    use std::time::Instant;
    let p = parse(args, &SPEC)?;
    let pawns = p.parse::<u32>("pawns")?.unwrap_or(200);
    let days = p.parse::<u64>("days")?.unwrap_or(1);
    let dirs: Vec<String> = if p.positional.is_empty() {
        vec![
            "packs/cookbook/caffeine".into(),
            "packs/cookbook/evening_legs".into(),
            "packs/cookbook/birthdays".into(),
            "packs/cookbook/hardy".into(),
        ]
    } else {
        p.positional.clone()
    };
    let mut packs = vec![load(
        p.one("base").unwrap_or(crate::shared::DEFAULT_CONTENT_DIR),
    )?];
    for d in &dirs {
        packs.push(load(d)?);
    }
    let set = Arc::new(
        ContentSet::build(packs, ComponentRegistry::builtin()).map_err(|r| r.to_string())?,
    );
    let with = SimFactory::dev(Some(Arc::clone(&set)), 1);
    let without = with.safe_mode();
    let run = |f: &SimFactory| -> Result<std::time::Duration, String> {
        let mut sim = f.new_sim(WorldState::new("Bench", "bench"));
        let cmd = |c| SimInput::Command {
            actor: None,
            cmd: c,
        };
        sim.submit(
            0,
            cmd(Command::DevCreateMap {
                w: 96,
                h: 72,
                style: 1,
            }),
        )
        .map_err(|e| e.to_string())?;
        for i in 0..pawns {
            sim.submit(
                0,
                cmd(Command::DevSpawnPawn {
                    map: EntityId::new(Kind::Map, 1),
                    at: None,
                    name: format!("P{i}"),
                }),
            )
            .map_err(|e| e.to_string())?;
        }
        sim.run_ticks(1).map_err(|e| e.to_string())?;
        let t = Instant::now();
        sim.run_ticks(days * TICKS_PER_DAY)
            .map_err(|e| e.to_string())?;
        Ok(t.elapsed())
    };
    let base = run(&without)?;
    let scripted = run(&with)?;
    let ticks = (days * TICKS_PER_DAY) as f64;
    println!("{pawns} pawn(s), {days} day(s), packs: {}", dirs.join(", "));
    println!(
        "  without scripts: {:.3} ms per tick",
        base.as_secs_f64() * 1000.0 / ticks
    );
    println!(
        "  with scripts:    {:.3} ms per tick",
        scripted.as_secs_f64() * 1000.0 / ticks
    );
    println!(
        "  scripts add:     {:.3} ms per tick ({:.2}% of the 100 ms tick at 1x)",
        (scripted.as_secs_f64() - base.as_secs_f64()) * 1000.0 / ticks,
        (scripted.as_secs_f64() - base.as_secs_f64()) * 1000.0 / ticks
    );
    // The boundary itself: one handler called per entity, with an entity view as its argument.
    let mut vm =
        LuauVm::new(VmConfig::new("bench", &["data", "systems"], 1)).map_err(|e| e.to_string())?;
    vm.load(SourceChunk {
        name: "scripts/main.luau".into(),
        source: "pg.systems.register({ id = 's', cadence = 'minute', query = { kind = 'pawn' }, run = function(ctx, e) local c = e:get('c') ctx.cmd:set_field(e.id, 'c', 'v', c.v + 1) return c.v end })".into(),
    })
    .map_err(|e| e.to_string())?;
    let reg = vm
        .run_load_phase("scripts/main.luau", Fuel(100_000))
        .map_err(|e| e.to_string())?;
    let handler = reg.systems[0].handler;
    let args: Vec<Val> = (0..u64::from(pawns))
        .map(|i| {
            Val::map([
                ("ctx", Val::map([("tick", Val::Int(5))])),
                (
                    "entity",
                    Val::map([
                        ("id", Val::Str(format!("pawn_{i}"))),
                        (
                            "components",
                            Val::map([("c", Val::map([("v", Val::Int(3))]))]),
                        ),
                    ]),
                ),
            ])
        })
        .collect();
    let first = vm
        .call(handler, &args[0], Fuel(50_000))
        .map_err(|e| e.to_string())?;
    println!(
        "  one VM: {} KiB at rest after load; the sample handler costs {} fuel per call",
        vm.memory_used() / 1024,
        first.fuel
    );
    let rounds = 200u32;
    let t = Instant::now();
    for _ in 0..rounds {
        for a in &args {
            vm.call(handler, a, Fuel(50_000))
                .map_err(|e| e.to_string())?;
        }
    }
    let per_call = t.elapsed().as_secs_f64() * 1e6 / f64::from(rounds) / args.len() as f64;
    let t = Instant::now();
    for _ in 0..rounds {
        for r in vm.call_batch(handler, &args, Fuel(50_000)) {
            r.map_err(|e| e.to_string())?;
        }
    }
    let per_batch = t.elapsed().as_secs_f64() * 1e6 / f64::from(rounds) / args.len() as f64;
    println!(
        "  boundary: {per_call:.2} us per call, {per_batch:.2} us per call in a batch of {}",
        args.len()
    );
    println!(
        "  a per-minute system over {pawns} pawns costs about {:.2} ms once a minute",
        per_call * f64::from(pawns) / 1000.0
    );
    Ok(ExitCode::SUCCESS)
}

fn short_hex(h: Option<&String>) -> String {
    h.map_or_else(|| "-".to_owned(), |s| s.chars().take(8).collect())
}

fn new_pack(args: &[String]) -> Result<ExitCode, String> {
    let [id, dir] = args else {
        return Err("usage: pg pack new <id> <dir>".into());
    };
    if !id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err("the pack id must be lower-case letters, digits and underscores, starting with a letter".into());
    }
    let root = std::path::Path::new(dir);
    if root.exists() {
        return Err(format!("'{dir}' already exists"));
    }
    std::fs::create_dir_all(root.join("scripts")).map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("pack.json"),
        format!(
            "{{\n  \"id\": \"{id}\",\n  \"name\": \"{id}\",\n  \"version\": \"0.1.0\",\n  \"api\": \">=0.1 <0.2\",\n  \"depends\": [{{ \"id\": \"base\", \"version\": \">=0.1\" }}],\n  \"capabilities\": [\"data\", \"systems\"],\n  \"entry\": \"scripts/main.luau\"\n}}\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("scripts/main.luau"),
        "--!strict\n-- A starting point: one component and one system that counts minutes. See packs/cookbook for more.\n\npg.components.register({\n  name = \"counter\",\n  applies_to = { \"pawn\" },\n  fields = { minutes = pg.field.int(0, 1000000, 0) },\n})\n\npg.systems.register({\n  id = \"count\",\n  cadence = \"minute\",\n  query = { kind = \"pawn\", with = { \"counter\" } },\n  writes = { \"counter\" },\n  run = function(ctx, pawn)\n    ctx.cmd:set_field(pawn.id, \"counter\", \"minutes\", pawn:get(\"counter\").minutes + 1)\n  end,\n})\n",
    )
    .map_err(|e| e.to_string())?;
    println!("created {dir}; try: pg pack lint {dir} && pg pack test {dir}");
    let _ = short;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(src: &str, caps: &[&str]) -> Vec<String> {
        lint_source("t.luau", src, caps)
            .into_iter()
            .map(|f| f.message)
            .collect()
    }

    #[test]
    fn unknown_api_names_and_missing_capabilities_are_errors_with_hints() {
        let m = msgs("pg.component.register({})", &["data"]);
        assert!(
            m.iter()
                .any(|x| x.contains("not part of the pg API")
                    && x.contains("pg.components.register")),
            "{m:?}"
        );
        let m = msgs("pg.systems.register({})", &["data"]);
        assert!(
            m.iter()
                .any(|x| x.contains("needs the 'systems' capability")),
            "{m:?}"
        );
        assert!(msgs("pg.components.register({})\npg.field.int(0, 1, 0)\npg.math.idiv(1, 2)\nlocal x = pg.rand('a')", &["data"]).is_empty());
        assert!(
            msgs("-- pg.nonsense.call()", &[]).is_empty(),
            "comments are ignored"
        );
    }

    #[test]
    fn order_sensitive_iteration_and_missing_libraries_are_flagged() {
        assert!(msgs("for k, v in pairs(t) do end\nfor i, v in ipairs(t) do end\nfor w in s:gmatch('a') do end", &[]).is_empty());
        assert!(msgs("for k, v in t do end", &[])
            .iter()
            .any(|m| m.contains("no guaranteed order")));
        assert!(msgs("local t = os.time()", &[])
            .iter()
            .any(|m| m.contains("does not exist")));
        assert!(msgs("local r = math.random(5)", &[])
            .iter()
            .any(|m| m.contains("math.random")));
        assert!(
            msgs("local pos = 5", &[]).is_empty(),
            "'os' inside another word is fine"
        );
    }

    #[test]
    fn module_level_state_changed_inside_a_function_is_flagged() {
        let src = "local calls = 0\nlocal function f()\n  calls += 1\nend\n";
        assert!(msgs(src, &[])
            .iter()
            .any(|m| m.contains("module-level state")));
        let ok = "local LIMIT = 10\nlocal function f()\n  local calls = 0\n  calls += 1\nend\n";
        assert!(msgs(ok, &[]).is_empty(), "{:?}", msgs(ok, &[]));
    }

    #[test]
    fn pg_paths_are_extracted_precisely() {
        assert_eq!(
            pg_paths("x = pg.rand('a') + pg.math.idiv(1, 2)"),
            ["pg.rand", "pg.math.idiv"]
        );
        assert!(pg_paths("local apg.x = 1").is_empty());
        assert_eq!(pg_paths("pg.hooks.on(\"x\", f)"), ["pg.hooks.on"]);
    }
}
