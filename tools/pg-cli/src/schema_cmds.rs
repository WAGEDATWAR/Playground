//! `pg schema check|update` (suggestion S-066): a fingerprint of the *shape* of a saved world.
//!
//! A town is generated and run for two days; the world is written in its canonical form at several moments
//! and every field path is recorded with the kinds of value seen there (`pawns.*.talk: map|null`). The list
//! is kept in `golden/schema-shape.txt`. Changing what a save contains without bumping `SCHEMA_VERSION` or
//! adding a migration fails `pg check` with the paths that appeared and disappeared, so a missed migration
//! is found at once instead of whenever a fixture happens to trip.

use crate::shared::{load_content, DEFAULT_CONTENT_DIR};
use pg_core::canon::{Canon, ToCanon};
use pg_core::commands::Command as WorldCommand;
use pg_core::input::SimInput;
use pg_core::pipeline::Pipeline;
use pg_core::sim::Sim;
use pg_core::world::{WorldState, SCHEMA_VERSION};
use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

const FILE: &str = "golden/schema-shape.txt";

fn kind(c: &Canon) -> &'static str {
    match c {
        Canon::Null => "null",
        Canon::Bool(_) => "bool",
        Canon::Int(_) => "int",
        Canon::Str(_) => "text",
        Canon::List(_) => "list",
        Canon::Map(_) => "map",
    }
}

fn walk(path: &str, c: &Canon, out: &mut BTreeMap<String, BTreeSet<&'static str>>) {
    out.entry(path.to_owned()).or_default().insert(kind(c));
    match c {
        Canon::Map(m) => {
            // Tables keyed by entity id or pair (pawns, objects, relationships ...) and free-keyed maps
            // (needs by id) are rows: their keys are data, not shape.
            let rows = !m.is_empty()
                && ((path.split('.').count() == 2
                    && m.keys().all(|k| k.contains('_'))
                    && m.values().all(|v| matches!(v, Canon::Map(_))))
                    || path.ends_with(".needs"));
            if rows {
                for v in m.values() {
                    walk(&format!("{path}.*"), v, out);
                }
            } else {
                for (k, v) in m {
                    walk(&format!("{path}.{k}"), v, out);
                }
            }
        }
        Canon::List(l) => {
            for v in l {
                walk(&format!("{path}[]"), v, out);
            }
        }
        _ => {}
    }
}

/// The shape listing of a town at several moments of its first two days.
fn current_shape() -> Result<String, String> {
    let content = load_content(&[DEFAULT_CONTENT_DIR.to_owned()])?;
    let mut sim =
        Sim::new(WorldState::new("Shape", "schema-shape"), Pipeline::new()).with_content(content);
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: WorldCommand::GenerateTown {
                w: 48,
                h: 36,
                water: 15,
                residents: 12,
                tone: "standard".into(),
            },
        },
    )
    .map_err(|e| e.to_string())?;
    let mut shape: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    // Step a game minute at a time; look at the world at fixed moments and the first times a conversation
    // is under way (so talks, tasks and routes in progress all appear).
    let stops: BTreeSet<u64> = (1..=24u64)
        .map(|i| i * 150)
        .chain([4_300, 8_000, 12_000, 21_000, 28_800])
        .collect();
    let mut talks_seen = 0;
    let mut tick = 0u64;
    while tick < 28_800 {
        sim.run_ticks(10).map_err(|e| e.to_string())?;
        tick += 10;
        let talking = sim.world().pawns.iter().any(|(_, p)| p.talk.is_some());
        if talking && talks_seen < 2 {
            talks_seen += 1;
            walk("world", &sim.world().to_canon(), &mut shape);
        }
        if stops.iter().any(|s| *s > tick - 10 && *s <= tick) {
            walk("world", &sim.world().to_canon(), &mut shape);
        }
    }
    let mut text = format!("schema {SCHEMA_VERSION}\n");
    for (path, kinds) in &shape {
        let k: Vec<&str> = kinds.iter().copied().collect();
        text.push_str(&format!("{path}: {}\n", k.join("|")));
    }
    Ok(text)
}

pub fn schema_cmd(args: &[String]) -> Result<ExitCode, String> {
    let now = current_shape()?;
    match args.first().map(String::as_str) {
        Some("show") => {
            print!("{now}");
            Ok(ExitCode::SUCCESS)
        }
        Some("update") => {
            std::fs::write(FILE, &now).map_err(|e| format!("cannot write {FILE}: {e}"))?;
            println!(
                "schema shape recorded in {FILE} ({} lines)",
                now.lines().count()
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("check") | None => {
            let recorded = std::fs::read_to_string(FILE)
                .map_err(|e| format!("cannot read {FILE}: {e} (run `pg schema update`)"))?;
            let (old, new): (BTreeSet<&str>, BTreeSet<&str>) =
                (recorded.lines().collect(), now.lines().collect());
            if old == new {
                println!(
                    "schema shape matches ({} lines, schema {SCHEMA_VERSION})",
                    new.len()
                );
                return Ok(ExitCode::SUCCESS);
            }
            println!("SCHEMA SHAPE CHANGED:");
            for l in new.difference(&old) {
                println!("  + {l}");
            }
            for l in old.difference(&new) {
                println!("  - {l}");
            }
            println!(
                "A save written before this change will not match. Bump SCHEMA_VERSION and add a migration (pg-persist/migrate.rs), or, while schema {SCHEMA_VERSION} is still in development and unshipped, run `pg schema update` and regenerate the v4 fixture (`pg pins update`)."
            );
            Ok(ExitCode::FAILURE)
        }
        Some(other) => Err(format!(
            "unknown schema command '{other}' (check, update, show)"
        )),
    }
}
