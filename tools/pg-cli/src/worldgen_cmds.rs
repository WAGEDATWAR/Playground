//! Developer commands for the town generator (Stage 1, milestone 1.2): `worldgen preview`.

use crate::args::{parse, parse_size, Spec};
use crate::shared::{load_content, DEFAULT_CONTENT_DIR};
use pg_core::town::{BuildingRole, DistrictKind};
use pg_core::world::{TonePreset, WorldState};
use pg_core::worldgen::{generate_town, render_ascii, validate_world, GenParams};
use std::process::ExitCode;

pub fn worldgen_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("preview") => preview(&args[1..]),
        Some(other) => Err(format!("unknown worldgen command '{other}' (try preview)")),
        None => Err("worldgen needs a command: preview".into()),
    }
}

const SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "size",
        "water",
        "residents",
        "tone",
        "content",
        "expect",
    ],
    switches: &["no-map"],
    optional: &[],
};

/// `pg worldgen preview [--seed S] [--size WxH] [--water PERCENT] [--residents N] [--tone T] [--no-map]`:
/// generates a town and draws it with what the generator made.
fn preview(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let seed = p.one("seed").unwrap_or("playground").to_owned();
    let (w, h, _) = parse_size(p.one("size").unwrap_or("64x48"))?;
    let tone = match p.one("tone") {
        None => TonePreset::Standard,
        Some(t) => TonePreset::parse(t).ok_or("--tone is cozy, standard or mature")?,
    };
    let params = GenParams {
        width: w,
        height: h,
        water_percent: p.parse::<u32>("water")?.unwrap_or(18),
        residents: p.parse::<usize>("residents")?.unwrap_or(14),
        tone,
    };
    let mut dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if dirs.is_empty() {
        dirs.push(DEFAULT_CONTENT_DIR.to_owned());
    }
    let content = load_content(&dirs)?;
    let mut world = WorldState::new("Preview", seed.clone());
    let started = std::time::Instant::now();
    let outcome = generate_town(&mut world, content.game(), &params).map_err(|e| e.to_string())?;
    let took = started.elapsed();
    if !p.has("no-map") {
        println!("{}", render_ascii(&world));
        println!(
            "~ water  , sand  = road  : sidewalk  _ plaza floor  @ gathering place  . grass  h a resident's home tile\nH home  S shop  O office  C civic\n"
        );
    }
    let t = &world.town;
    let kinds: Vec<String> = DistrictKind::ALL
        .iter()
        .map(|k| {
            format!(
                "{} {}",
                t.districts.iter().filter(|d| d.kind == *k).count(),
                k.name()
            )
        })
        .collect();
    let roles: Vec<String> = BuildingRole::ALL
        .iter()
        .map(|r| format!("{} {}", t.buildings_with(*r).count(), r.name()))
        .collect();
    println!(
        "seed '{seed}', {w}x{h}, water {}%, tone {}",
        params.water_percent,
        tone.name()
    );
    println!("districts: {}", kinds.join(", "));
    println!("buildings: {} ({})", t.buildings.len(), roles.join(", "));
    println!(
        "residents: {} in {} household(s); {} starting relationship(s); {} gathering place(s)",
        outcome.residents,
        world.households.len(),
        world.relationships.len(),
        t.gathering.len()
    );
    println!(
        "attempt {} of {}, {:.0} ms; starting hash {}",
        outcome.attempt + 1,
        pg_core::worldgen::MAX_ATTEMPTS,
        took.as_secs_f64() * 1000.0,
        &outcome.starting_hash[..16]
    );
    if let Some(want) = p.one("expect") {
        if !outcome.starting_hash.starts_with(want) {
            println!(
                "STARTING HASH MISMATCH: expected {want}..., generated {}",
                &outcome.starting_hash[..want.len().clamp(8, 64)]
            );
            return Ok(ExitCode::FAILURE);
        }
        println!("starting hash matches the pinned {want}");
    }
    match validate_world(&world) {
        Ok(()) => println!("validation: OK"),
        Err(e) => {
            println!("validation FAILED: {e}");
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}
