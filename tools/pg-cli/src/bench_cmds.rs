//! `pg bench` (suggestion S-071): one place for every measurable target, with a report file.
//!
//! * `pg bench --list`
//! * `pg bench --targets ticks,paths` or `pg bench --all` [`--runs N`] [`--out DIR`] [`--baseline FILE.json`]
//!
//! Each target is a command the tool already has (so what is measured is exactly what a person can run by
//! hand); every run is timed from the outside and the median and worst of the runs are reported against the
//! target's budget, if it has one. Reports are written to `bench/reports/<date>-<commit>.json` (for tools)
//! and `.md` (for people), recording the machine and build so two reports are comparable. Timings never touch
//! simulation state. Run it in release mode (`cargo run --release -p pg-cli -- bench --all`); a debug build
//! is reported as such and its budgets are not enforced.

use crate::args::{parse, Spec};
use serde_json::{json, Value};
use std::process::{Command, ExitCode};
use std::time::Instant;

const SPEC: Spec<'static> = Spec {
    values: &["targets", "runs", "out", "baseline"],
    switches: &["list", "all"],
    optional: &[],
};

struct Target {
    id: &'static str,
    what: &'static str,
    args: &'static [&'static str],
    /// Wall-clock budget for one run, in milliseconds (release builds only).
    budget_ms: Option<u64>,
}

const TARGETS: &[Target] = &[
    Target {
        id: "ticks",
        what: "tick loop: 30 dev pawns, 2000 ticks",
        args: &[
            "profile",
            "--dev-map",
            "48x36",
            "--dev-pawns",
            "30",
            "--ticks",
            "2000",
        ],
        budget_ms: Some(3_000),
    },
    Target {
        id: "town",
        what: "generated town of 20 residents for 3 game days (needs, schedules, conversations)",
        args: &[
            "sim",
            "--seed",
            "bench",
            "--town",
            "64x48",
            "--residents",
            "20",
            "--content",
            "data/base",
            "--days",
            "3",
        ],
        budget_ms: Some(20_000),
    },
    Target {
        id: "crowd",
        what: "generated town of 50 residents (the most a town makes) for two game days (scale of the conversation search)",
        args: &[
            "sim",
            "--seed",
            "bench",
            "--town",
            "96x72",
            "--residents",
            "50",
            "--content",
            "data/base",
            "--days",
            "2",
        ],
        budget_ms: Some(30_000),
    },
    Target {
        id: "worldgen",
        what: "town generation, 96x72 with 50 residents",
        args: &[
            "worldgen",
            "preview",
            "--seed",
            "bench",
            "--size",
            "96x72",
            "--residents",
            "50",
            "--no-map",
        ],
        budget_ms: Some(5_000),
    },
    Target {
        id: "paths",
        what: "300 path requests on 96x72, serial and parallel",
        args: &[
            "map",
            "bench-paths",
            "--size",
            "96x72",
            "--requests",
            "300",
            "--threads",
            "1,2,4",
        ],
        budget_ms: Some(10_000),
    },
    Target {
        id: "replay",
        what: "the golden three-day replay with hash verification",
        args: &["replay", "golden/dev-town-3days.json"],
        budget_ms: Some(20_000),
    },
];

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

pub fn bench_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    if p.has("list") {
        for t in TARGETS {
            println!("{:<10} {}", t.id, t.what);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let chosen: Vec<&Target> = if p.has("all") {
        TARGETS.iter().collect()
    } else if let Some(list) = p.one("targets") {
        let mut v = Vec::new();
        for id in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            v.push(TARGETS.iter().find(|t| t.id == id).ok_or_else(|| {
                format!(
                    "unknown target '{id}' (known: {})",
                    TARGETS.iter().map(|t| t.id).collect::<Vec<_>>().join(", ")
                )
            })?);
        }
        v
    } else {
        return Err("usage: pg bench --list | --all | --targets a,b [--runs N] [--out DIR] [--baseline FILE.json]".into());
    };
    let runs = p.parse::<usize>("runs")?.unwrap_or(3).clamp(1, 50);
    let release = !cfg!(debug_assertions);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let baseline: Option<Value> = p
        .one("baseline")
        .map(|f| {
            std::fs::read_to_string(f)
                .map_err(|e| format!("cannot read {f}: {e}"))
                .and_then(|t| serde_json::from_str(&t).map_err(|e| format!("{f}: {e}")))
        })
        .transpose()?;
    let mut rows = Vec::new();
    let mut failed = false;
    for t in &chosen {
        let mut times = Vec::new();
        let mut ok = true;
        for _ in 0..runs {
            let start = Instant::now();
            let out = Command::new(&exe)
                .args(t.args)
                .output()
                .map_err(|e| e.to_string())?;
            ok &= out.status.success();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let worst = times.iter().copied().fold(0.0, f64::max);
        let med = median(&mut times.clone());
        let over = release && t.budget_ms.is_some_and(|b| med > b as f64);
        failed |= !ok || over;
        let was = baseline
            .as_ref()
            .and_then(|b| b["targets"].as_array())
            .and_then(|a| a.iter().find(|r| r["id"] == t.id))
            .and_then(|r| r["median_ms"].as_f64());
        println!(
            "{:<9} median {:>8.0} ms  worst {:>8.0} ms  {}{}{}",
            t.id,
            med,
            worst,
            t.budget_ms
                .map_or(String::new(), |b| format!("budget {b} ms  ")),
            if !ok {
                "FAILED TO RUN"
            } else if over {
                "OVER BUDGET"
            } else {
                "ok"
            },
            was.map_or(String::new(), |w| format!(
                "  (baseline {w:.0} ms, {:+.0}%)",
                (med / w.max(1.0) - 1.0) * 100.0
            ))
        );
        rows.push(json!({
            "id": t.id, "what": t.what, "args": t.args, "runs": runs,
            "median_ms": med.round(), "worst_ms": worst.round(),
            "budget_ms": t.budget_ms, "ok": ok && !over, "baseline_median_ms": was,
        }));
    }
    let commit = git(&["rev-parse", "--short", "HEAD"]);
    let date = git(&["log", "-1", "--format=%cd", "--date=short"]);
    let report = json!({
        "format": "playground-bench", "version": 1,
        "pg_version": env!("CARGO_PKG_VERSION"),
        "commit": commit, "date": date,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "threads": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "build": if release { "release" } else { "debug (budgets not enforced)" },
        "dirty": !git(&["status", "--porcelain"]).is_empty(),
        "targets": rows,
    });
    let dir = p.one("out").unwrap_or("bench/reports");
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir}: {e}"))?;
    let stem = format!("{dir}/{date}-{commit}");
    std::fs::write(
        format!("{stem}.json"),
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())?;
    let mut md = format!(
        "# Benchmark report {date} ({commit})\n\n{} {} | {} thread(s) | {} build | pg {}\n\n| Target | What | Median ms | Worst ms | Budget ms | Status |\n| --- | --- | ---: | ---: | ---: | --- |\n",
        report["os"].as_str().unwrap_or(""),
        report["arch"].as_str().unwrap_or(""),
        report["threads"],
        report["build"].as_str().unwrap_or(""),
        report["pg_version"].as_str().unwrap_or("")
    );
    for r in report["targets"].as_array().into_iter().flatten() {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            r["id"].as_str().unwrap_or(""),
            r["what"].as_str().unwrap_or(""),
            r["median_ms"],
            r["worst_ms"],
            r["budget_ms"]
                .as_u64()
                .map_or("-".to_owned(), |b| b.to_string()),
            if r["ok"].as_bool() == Some(true) {
                "ok"
            } else {
                "FAILED"
            }
        ));
    }
    std::fs::write(format!("{stem}.md"), md).map_err(|e| e.to_string())?;
    println!("\nreport written to {stem}.json and {stem}.md");
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
