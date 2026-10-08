//! Developer commands for residents (Stage 1, milestone 1.0): `residents generate`.

use crate::args::{parse, Spec};
use crate::shared::{load_content, DEFAULT_CONTENT_DIR};
use pg_core::population::{plan_population, PopulationPlan};
use pg_core::rng::Seed;
use std::process::ExitCode;

pub fn residents_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("generate") => generate(&args[1..]),
        Some(other) => Err(format!(
            "unknown residents command '{other}' (try generate)"
        )),
        None => Err("residents needs a command: generate".into()),
    }
}

const SPEC: Spec<'static> = Spec {
    values: &["seed", "count", "content"],
    switches: &[],
    optional: &[],
};

/// `pg residents generate [--seed S] [--count N] [--content dir]...`: the people a seed produces (households,
/// occupations, starting relationships and shared memories), without building a world.
fn generate(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let seed_text = p.one("seed").unwrap_or("playground").to_owned();
    let count = p.parse::<usize>("count")?.unwrap_or(14);
    let mut dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if dirs.is_empty() {
        dirs.push(DEFAULT_CONTENT_DIR.to_owned());
    }
    let content = load_content(&dirs)?;
    let data = content.game();
    let plan =
        plan_population(data, Seed::from_text(&seed_text), count).map_err(|e| e.to_string())?;
    let text = |key: &str| content.strings().text("en", key, &[]);
    print_plan(
        &plan,
        &seed_text,
        &|key| text(key),
        &|id| {
            data.occupations
                .get(id)
                .map_or_else(|| id.to_owned(), |o| text(&o.name_key))
        },
        &|affinity| {
            data.relationships
                .as_ref()
                .and_then(|r| r.label_for(affinity))
                .map_or_else(String::new, |l| text(&l.label_key))
        },
    );
    Ok(ExitCode::SUCCESS)
}

fn print_plan(
    plan: &PopulationPlan,
    seed: &str,
    string: &dyn Fn(&str) -> String,
    occupation: &dyn Fn(&str) -> String,
    label: &dyn Fn(i32) -> String,
) {
    println!(
        "seed '{seed}': {} resident(s) in {} household(s)\n",
        plan.residents.len(),
        plan.households.len()
    );
    for (i, h) in plan.households.iter().enumerate() {
        println!("household {} - the {} family", i + 1, h.family);
        for m in &h.members {
            if let Some(r) = plan.residents.get(*m) {
                let needs: Vec<String> = r.needs.iter().map(|(k, v)| format!("{k} {v}")).collect();
                println!(
                    "  {:<3} {:<22} {:<12} variation {:<4} needs: {}",
                    m,
                    r.full_name(),
                    occupation(&r.occupation),
                    r.variation,
                    needs.join(", ")
                );
            }
        }
    }
    println!("\nstarting relationships ({}):", plan.relationships.len());
    let name = |i: usize| {
        plan.residents
            .get(i)
            .map_or_else(|| format!("#{i}"), |r| r.full_name())
    };
    for r in &plan.relationships {
        println!(
            "  {:<22} - {:<22} {:>4} {:<13} ({}){}",
            name(r.a),
            name(r.b),
            r.affinity,
            label(r.affinity),
            r.tie.name(),
            if r.shared_memory {
                format!("  memory: {}", string(r.tie.summary_key()))
            } else {
                String::new()
            }
        );
    }
}
