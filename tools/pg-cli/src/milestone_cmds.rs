//! `pg milestone` (suggestion S-068): the whole milestone checklist in one command.
//!
//! Runs formatting, the dependency rule, clippy, the test suite, `pg check --full` (every pinned hash, the
//! schema shape and the soak) and the app smoke test, then prints a short status in the form used for gate
//! reviews. Nothing is committed or pushed; `--tag NAME` creates a git tag once everything passed.

use std::process::{Command, ExitCode};
use std::time::Instant;

struct Step {
    name: &'static str,
    program: String,
    args: Vec<&'static str>,
}

fn steps(skip_tests: bool) -> Vec<Step> {
    let mut v = vec![
        Step {
            name: "formatting",
            program: "cargo".into(),
            args: vec!["fmt", "--all", "--", "--check"],
        },
        Step {
            name: "dependency rule",
            program: "python".into(),
            args: vec!["scripts/check_deps.py"],
        },
        Step {
            name: "clippy (warnings are errors)",
            program: "cargo".into(),
            args: vec![
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
    ];
    if !skip_tests {
        v.push(Step {
            name: "tests",
            program: "cargo".into(),
            args: vec!["test", "--workspace", "-q"],
        });
    }
    // The running binary checks itself: on Windows a running program cannot be rebuilt over, so this step
    // does not go back through cargo. (Run `pg milestone` from a release build so the soak is quick.)
    v.push(Step {
        name: "pg check --full (pins, schema shape, soak)",
        program: std::env::current_exe()
            .map_or_else(|_| "pg".into(), |p| p.to_string_lossy().into_owned()),
        args: vec!["check", "--full"],
    });
    v.push(Step {
        name: "app smoke test",
        program: "cargo".into(),
        args: vec!["run", "-q", "-p", "pg-app", "--", "--smoke"],
    });
    v
}

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

pub fn milestone_cmd(args: &[String]) -> Result<ExitCode, String> {
    let skip_tests = args.iter().any(|a| a == "--skip-tests");
    let tag = args
        .iter()
        .position(|a| a == "--tag")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let started = Instant::now();
    let mut results: Vec<(&str, bool, f64)> = Vec::new();
    for step in steps(skip_tests) {
        println!("== {} ==", step.name);
        let t = Instant::now();
        let ok = Command::new(&step.program)
            .args(&step.args)
            .status()
            .map_err(|e| format!("cannot run {}: {e}", step.program))?
            .success();
        results.push((step.name, ok, t.elapsed().as_secs_f64()));
        if !ok {
            break;
        }
    }
    println!("\nMilestone report");
    for (name, ok, secs) in &results {
        println!(
            "  {}  {name}  ({secs:.0}s)",
            if *ok { "PASS" } else { "FAIL" }
        );
    }
    let passed = results.iter().all(|(_, ok, _)| *ok) && results.len() == steps(skip_tests).len();
    let dirty = !git(&["status", "--porcelain"]).is_empty();
    println!(
        "\ncommit {}{}  |  {} step(s) in {:.0}s  |  {}",
        git(&["rev-parse", "--short", "HEAD"]),
        if dirty { " + uncommitted changes" } else { "" },
        results.len(),
        started.elapsed().as_secs_f64(),
        if passed { "ALL PASSED" } else { "FAILED" }
    );
    if passed && skip_tests {
        println!("(tests were skipped; this is not a full milestone check)");
    }
    if passed {
        if let Some(t) = tag {
            if dirty {
                return Err(format!("not tagging {t}: there are uncommitted changes"));
            }
            let out = Command::new("git")
                .args(["tag", &t])
                .status()
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                if out.success() {
                    format!("tagged {t} (push it with: git push --tags)")
                } else {
                    format!("could not create tag {t}")
                }
            );
        }
    }
    Ok(if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
