//! `pg pins` (suggestions S-051, S-059): every pinned hash in one place.
//!
//! A milestone that changes simulation state changes pins: the golden replay, the cookbook pack goldens, the
//! hashes in `scenarios/*.json`, the town generator's starting hash and the saved fixtures. This command lists
//! them, compares them all (`check`) and re-records them all (`update`), and says where a change first
//! showed up. Run it from the repository root, preferably in release mode so the 30-day soak is quick:
//!
//! * `pg pins list`
//! * `pg pins check [--full] [--only TEXT]`  (`--full` adds the slow soak)
//! * `pg pins update [--full] [--only TEXT]` (re-records, then prints what changed)

use crate::args::{parse, Spec};
use pg_core::canon::json;
use pg_core::world::WorldState;
use pg_persist::migrate::Migrations;
use std::path::Path;
use std::process::{Command, ExitCode};

const SPEC: Spec<'static> = Spec {
    values: &["only"],
    switches: &["full"],
    optional: &[],
};

const REPLAY: &str = "golden/dev-town-3days.json";
const TOWN_FILE: &str = "golden/town-start.txt";
const V3: &str = "fixtures/saves/world-v3";
const V4: &str = "fixtures/saves/world-v4";

#[derive(Clone, Debug)]
enum Kind {
    Replay,
    Pack(String),
    Scenario { path: String, slow: bool },
    Town,
    FixtureV3,
    FixtureV4,
}

#[derive(Clone, Debug)]
struct Pin {
    name: String,
    kind: Kind,
}

impl Pin {
    fn slow(&self) -> bool {
        matches!(self.kind, Kind::Scenario { slow: true, .. })
    }
}

fn pins() -> Vec<Pin> {
    let mut v = vec![Pin {
        name: "golden replay".into(),
        kind: Kind::Replay,
    }];
    let mut packs: Vec<String> = std::fs::read_dir("packs/cookbook")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("pack.json").exists() || e.path().is_dir())
        .map(|e| e.path().to_string_lossy().replace('\\', "/"))
        .collect();
    packs.sort();
    for p in packs {
        v.push(Pin {
            name: format!("pack golden: {}", p.rsplit('/').next().unwrap_or(&p)),
            kind: Kind::Pack(p),
        });
    }
    let mut scenarios: Vec<String> = std::fs::read_dir("scenarios")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().to_string_lossy().replace('\\', "/"))
        .filter(|p| p.ends_with(".json"))
        .collect();
    scenarios.sort();
    for s in scenarios {
        let slow = s.contains("soak");
        v.push(Pin {
            name: format!("scenario: {}", s.rsplit('/').next().unwrap_or(&s)),
            kind: Kind::Scenario { path: s, slow },
        });
    }
    v.push(Pin {
        name: "town generator starting hash".into(),
        kind: Kind::Town,
    });
    v.push(Pin {
        name: "fixture: world-v3 (migrated)".into(),
        kind: Kind::FixtureV3,
    });
    v.push(Pin {
        name: "fixture: world-v4".into(),
        kind: Kind::FixtureV4,
    });
    v
}

/// Runs this binary with `args`; returns (success, combined output).
fn run_self(args: &[&str]) -> (bool, String) {
    let exe = std::env::current_exe().unwrap_or_else(|_| "pg".into());
    match Command::new(exe).args(args).output() {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, format!("cannot run: {e}")),
    }
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n    ")
}

fn town_args(expect: &str) -> Vec<String> {
    [
        "worldgen",
        "preview",
        "--seed",
        "gate-town",
        "--size",
        "64x48",
        "--residents",
        "14",
        "--no-map",
        "--expect",
        expect,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// The pinned town hash (the first line of the pin file).
pub fn town_pin() -> String {
    std::fs::read_to_string(TOWN_FILE)
        .map(|t| t.trim().to_owned())
        .unwrap_or_default()
}

fn fixture_hash(base: &str, schema: i64) -> Result<String, String> {
    let text = std::fs::read_to_string(format!("{base}.json")).map_err(|e| e.to_string())?;
    let raw = json::parse(&text).map_err(|e| e.to_string())?;
    let migrated = Migrations::builtin()
        .migrate(raw, schema as u32)
        .map_err(|e| e.to_string())?;
    let world = WorldState::from_canon(&migrated).map_err(|e| e.to_string())?;
    Ok(world.state_hash().to_hex())
}

fn check_one(pin: &Pin) -> (bool, String) {
    match &pin.kind {
        Kind::Replay => {
            let (ok, out) = run_self(&["replay", REPLAY]);
            (ok, last_lines(&out, 3))
        }
        Kind::Pack(dir) => {
            let (ok, out) = run_self(&["pack", "test", dir]);
            (ok, last_lines(&out, 2))
        }
        Kind::Scenario { path, .. } => {
            let (ok, out) = run_self(&["scenario", "run", path]);
            let why = out
                .lines()
                .find(|l| l.contains("FAIL"))
                .map_or_else(|| last_lines(&out, 1), |l| l.trim().to_owned());
            (ok, why)
        }
        Kind::Town => {
            let args = town_args(&town_pin());
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, out) = run_self(&refs);
            let why = out
                .lines()
                .find(|l| l.contains("MISMATCH"))
                .map_or_else(|| last_lines(&out, 1), |l| l.trim().to_owned());
            (ok, why)
        }
        Kind::FixtureV3 | Kind::FixtureV4 => {
            let (base, schema) = if matches!(pin.kind, Kind::FixtureV3) {
                (V3, 3)
            } else {
                (V4, 4)
            };
            let want = std::fs::read_to_string(format!("{base}.hash")).unwrap_or_default();
            match fixture_hash(base, schema) {
                Ok(got) if got == want.trim() => (true, format!("matches {}", &got[..8])),
                Ok(got) => (
                    false,
                    format!(
                        "expected {}..., found {}...",
                        &want.trim()[..8.min(want.trim().len())],
                        &got[..8]
                    ),
                ),
                Err(e) => (false, e),
            }
        }
    }
}

/// Replaces `"hash": "<old>"` in a scenario file with the new prefix; returns false if nothing matched.
fn repin_scenario(path: &str, old: &str, new: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let from = format!("\"hash\": \"{old}\"");
    if !text.contains(&from) {
        return false;
    }
    let to = format!("\"hash\": \"{new}\"");
    std::fs::write(path, text.replacen(&from, &to, 1)).is_ok()
}

/// Pulls `(expected, found)` out of "state hash: expected X..., found Y".
fn expected_found(out: &str) -> Option<(String, String)> {
    let line = out.lines().find(|l| l.contains("state hash: expected"))?;
    let after = line.split("expected ").nth(1)?;
    let old = after.split("...").next()?.trim().to_owned();
    let new = after.split("found ").nth(1)?.trim().to_owned();
    Some((old, new))
}

fn update_one(pin: &Pin) -> (bool, String) {
    match &pin.kind {
        Kind::Replay => {
            // Keep the old log aside so the change can be located (S-059).
            let old = std::env::temp_dir().join("pg-pins-old-replay.json");
            let _ = std::fs::copy(REPLAY, &old);
            let (ok, out) = run_self(&["replay", REPLAY, "--rerecord"]);
            let mut msg = last_lines(&out, 1);
            if ok {
                let (_, diff) = run_self(&["replay", "--diff", &old.to_string_lossy(), REPLAY]);
                msg = format!("{msg}\n    where it changed: {}", last_lines(&diff, 3));
            }
            (ok, msg)
        }
        Kind::Pack(dir) => {
            let (ok, out) = run_self(&["pack", "test", dir, "--update"]);
            (ok, last_lines(&out, 1))
        }
        Kind::Scenario { path, .. } => {
            let mut notes = Vec::new();
            for _ in 0..8 {
                let (ok, out) = run_self(&["scenario", "run", path]);
                if ok {
                    return (
                        true,
                        if notes.is_empty() {
                            "already matched".into()
                        } else {
                            notes.join("; ")
                        },
                    );
                }
                match expected_found(&out) {
                    Some((old, new)) if repin_scenario(path, &old, &new) => {
                        notes.push(format!("{old} -> {new}"));
                    }
                    _ => {
                        let why = out
                            .lines()
                            .find(|l| l.contains("FAIL"))
                            .unwrap_or("failed")
                            .trim()
                            .to_owned();
                        return (false, format!("not a hash pin that can be updated: {why}"));
                    }
                }
            }
            (false, "still failing after eight re-pins".into())
        }
        Kind::Town => {
            let args = town_args("00000000");
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (_, out) = run_self(&refs);
            let Some(new) = out
                .lines()
                .find(|l| l.contains("generated "))
                .and_then(|l| l.split("generated ").nth(1))
                .map(|h| h.trim().to_owned())
            else {
                return (false, last_lines(&out, 2));
            };
            let old = town_pin();
            let ok = std::fs::write(TOWN_FILE, format!("{new}\n")).is_ok();
            (ok, format!("{old} -> {new}"))
        }
        Kind::FixtureV3 => match fixture_hash(V3, 3) {
            Ok(h) => (
                std::fs::write(format!("{V3}.hash"), &h).is_ok(),
                format!("recorded {}...", &h[..8]),
            ),
            Err(e) => (false, e),
        },
        Kind::FixtureV4 => {
            // The v4 file itself is regenerated by a deliberate (ignored) test, then its hash follows.
            let out = Command::new("cargo")
                .args([
                    "test",
                    "-q",
                    "-p",
                    "pg-persist",
                    "regenerate_v4_fixture",
                    "--",
                    "--ignored",
                ])
                .output();
            match out {
                Ok(o) if o.status.success() => (true, "regenerated".into()),
                Ok(o) => (false, last_lines(&String::from_utf8_lossy(&o.stdout), 3)),
                Err(e) => (false, format!("cannot run cargo: {e}")),
            }
        }
    }
}

pub fn pins_cmd(args: &[String]) -> Result<ExitCode, String> {
    let (sub, rest) = args
        .split_first()
        .ok_or("usage: pg pins list|check|update [--full] [--only TEXT]")?;
    let p = parse(rest, &SPEC)?;
    if !Path::new("scenarios").is_dir() {
        return Err("run `pg pins` from the repository root".into());
    }
    let full = p.has("full");
    let only = p.one("only").map(str::to_lowercase);
    let selected: Vec<Pin> = pins()
        .into_iter()
        .filter(|x| {
            only.as_ref()
                .is_none_or(|o| x.name.to_lowercase().contains(o))
        })
        .collect();
    match sub.as_str() {
        "list" => {
            for x in &selected {
                println!(
                    "{}{}",
                    x.name,
                    if x.slow() { "  (slow, --full)" } else { "" }
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        "check" | "update" => {
            let update = sub == "update";
            let mut failed = 0;
            for x in &selected {
                if x.slow() && !full {
                    println!("SKIP  {}  (slow: add --full)", x.name);
                    continue;
                }
                let (ok, msg) = if update { update_one(x) } else { check_one(x) };
                println!(
                    "{}  {}\n    {msg}",
                    if ok { "OK  " } else { "FAIL" },
                    x.name
                );
                if !ok {
                    failed += 1;
                }
            }
            if update {
                // Refresh the dependent hash file for the regenerated v4 fixture, then verify everything.
                if selected.iter().any(|x| matches!(x.kind, Kind::FixtureV4)) {
                    if let Ok(h) = fixture_hash(V4, 4) {
                        let _ = std::fs::write(format!("{V4}.hash"), h);
                    }
                }
                println!("\nre-recorded; now verifying with `pg pins check`");
                let mut again: Vec<String> = vec!["pins".into(), "check".into()];
                if full {
                    again.push("--full".into());
                }
                if let Some(o) = p.one("only") {
                    again.push("--only".into());
                    again.push(o.into());
                }
                let refs: Vec<&str> = again.iter().map(String::as_str).collect();
                let (ok, out) = run_self(&refs);
                print!("{out}");
                return Ok(if ok && failed == 0 {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                });
            }
            println!(
                "\n{} pin(s) checked, {failed} failed",
                selected.iter().filter(|x| full || !x.slow()).count()
            );
            Ok(if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        other => Err(format!(
            "unknown pins command '{other}' (list, check, update)"
        )),
    }
}
