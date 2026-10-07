//! `pg check` (suggestion S-032): one command that runs the developer checks and prints a single report.
//!
//! Each check is this same binary run with different arguments, so the report reflects exactly what a person
//! would get by typing the commands. It is meant to be run from the repository root; checks that need repo
//! files (golden replay, scenarios, base content) are skipped, and said to be skipped, elsewhere.

use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Instant;

struct Check {
    name: &'static str,
    args: Vec<String>,
    needs: Option<&'static str>,
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

pub fn check_cmd(rest: &[String]) -> Result<ExitCode, String> {
    let verbose = rest.iter().any(|a| a == "--verbose");
    let tmp = std::env::temp_dir().join(format!("pg-check-{}", std::process::id()));
    let saves = tmp.join("saves");
    let (saves, bundle) = (
        saves.to_string_lossy().into_owned(),
        tmp.join("check.pgbundle").to_string_lossy().into_owned(),
    );
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let checks = vec![
        Check {
            name: "determinism vectors",
            args: args(&["selftest"]),
            needs: None,
        },
        Check {
            name: "base content lints",
            args: args(&["content", "lint"]),
            needs: Some("data/base"),
        },
        Check {
            name: "string tables",
            args: args(&["strings", "lint"]),
            needs: Some("data/base"),
        },
        Check {
            name: "golden replay + snapshot/resume",
            args: args(&[
                "replay",
                "golden/dev-town-3days.json",
                "--snapshot-at",
                "12345",
            ]),
            needs: Some("golden/dev-town-3days.json"),
        },
        Check {
            name: "event catalog",
            args: args(&["events", "list"]),
            needs: None,
        },
        Check {
            name: "settings registry",
            args: args(&["settings", "list"]),
            needs: None,
        },
        Check {
            name: "dev-tool registry",
            args: args(&["tools"]),
            needs: None,
        },
        Check {
            name: "profile run (30 pawns, 2000 ticks)",
            args: args(&[
                "profile",
                "--dev-map",
                "48x36",
                "--dev-pawns",
                "30",
                "--ticks",
                "2000",
            ]),
            needs: None,
        },
        Check {
            name: "session run + shadow verification",
            args: vec![
                "run".into(),
                "--dir".into(),
                tmp.join("run").to_string_lossy().into_owned(),
                "--seconds".into(),
                "3".into(),
                "--speed".into(),
                "27x".into(),
                "--shadow".into(),
                "2".into(),
            ],
            needs: None,
        },
        Check {
            name: "action registry",
            args: args(&["actions"]),
            needs: None,
        },
        Check {
            name: "schedule explainer",
            args: args(&[
                "schedule",
                "explain",
                "pawn_1",
                "--seed",
                "check",
                "--dev-map",
                "40x30",
                "--dev-pawns",
                "4",
                "--at",
                "9000",
            ]),
            needs: None,
        },
        Check {
            name: "parallel pathfinding equals serial",
            args: args(&[
                "map",
                "bench-paths",
                "--size",
                "64x48",
                "--requests",
                "100",
                "--threads",
                "1,2,4",
            ]),
            needs: None,
        },
        Check {
            name: "redaction self-check (sentinel key)",
            args: args(&["ai", "selfcheck"]),
            needs: None,
        },
        Check {
            name: "persistence scenario",
            args: args(&["scenario", "run", "scenarios/persistence.json"]),
            needs: Some("scenarios/persistence.json"),
        },
        Check {
            name: "save create",
            args: vec![
                "save".into(),
                "create".into(),
                saves.clone(),
                "--world".into(),
                "town".into(),
                "--seed".into(),
                "check".into(),
                "--dev-map".into(),
                "40x30".into(),
                "--dev-pawns".into(),
                "6".into(),
                "--days".into(),
                "2".into(),
            ],
            needs: None,
        },
        Check {
            name: "save verify",
            args: vec!["save".into(), "verify".into(), saves, "town".into()],
            needs: None,
        },
        Check {
            name: "bug bundle create",
            args: vec![
                "bugbundle".into(),
                "create".into(),
                "golden/dev-town-3days.json".into(),
                "--at".into(),
                "20000".into(),
                "--out".into(),
                bundle.clone(),
            ],
            needs: Some("golden/dev-town-3days.json"),
        },
        Check {
            name: "bug bundle replay",
            args: vec!["bugbundle".into(), "run".into(), bundle],
            needs: Some("golden/dev-town-3days.json"),
        },
    ];
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut failed = 0;
    let mut rows = Vec::new();
    let start = Instant::now();
    for c in &checks {
        if let Some(p) = c.needs {
            if !Path::new(p).exists() {
                rows.push((
                    c.name,
                    "SKIP",
                    format!("needs {p} (run from the repository root)"),
                    0.0,
                ));
                continue;
            }
        }
        let t = Instant::now();
        let out = Command::new(&exe)
            .args(&c.args)
            .output()
            .map_err(|e| e.to_string())?;
        let secs = t.elapsed().as_secs_f32();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        if out.status.success() {
            rows.push((c.name, "PASS", last, secs));
            if verbose {
                println!("--- {} ---\n{text}", c.name);
            }
        } else {
            failed += 1;
            rows.push((c.name, "FAIL", last, secs));
            println!("--- {} (failed) ---\n{text}", c.name);
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    println!("\npg check report (pg {})", env!("CARGO_PKG_VERSION"));
    for (name, verdict, detail, secs) in &rows {
        println!("  {verdict}  {name:<40} {secs:>5.1}s  {detail}");
    }
    let skipped = rows.iter().filter(|r| r.1 == "SKIP").count();
    println!(
        "\n{} check(s), {} passed, {} failed, {} skipped, {:.1}s",
        rows.len(),
        rows.len() - failed - skipped,
        failed,
        skipped,
        start.elapsed().as_secs_f32()
    );
    println!(
        "{}",
        if failed == 0 {
            "ALL CHECKS PASSED"
        } else {
            "CHECKS FAILED"
        }
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
