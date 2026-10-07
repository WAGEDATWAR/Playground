//! Developer commands for content (milestone 0.3): `content lint | list | resolve | components`.

use pg_content::schema::{FieldSchema, ParamSchema};
use pg_content::{
    load_pack, ComponentRegistry, ContentSet, DirPack, Limits, LoadedPack, TemplateId,
    ValidationReport,
};
use pg_core::canon::{Canon, ToCanon};
use std::process::ExitCode;

const DEFAULT_PACK_DIR: &str = "data/base";

fn pack_dirs(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        vec![DEFAULT_PACK_DIR.to_owned()]
    } else {
        args.to_vec()
    }
}

/// Loads every pack directory; prints any load errors and returns the packs that loaded.
fn load_all(dirs: &[String]) -> (Vec<LoadedPack>, bool) {
    let mut packs = Vec::new();
    let mut ok = true;
    for dir in dirs {
        match load_pack(&DirPack::new(dir), &Limits::default()) {
            Ok(p) => {
                println!(
                    "loaded {dir}: pack '{}' v{}  {} template(s), {} file(s), {} byte(s), hash {}",
                    p.manifest.id,
                    p.manifest.version,
                    p.templates.len(),
                    p.file_count,
                    p.total_bytes,
                    p.hash.chars().take(12).collect::<String>()
                );
                for w in p.warnings.issues() {
                    println!("  {w}");
                }
                packs.push(p);
            }
            Err(report) => {
                ok = false;
                println!("FAILED to load {dir}:");
                print_report(&report);
            }
        }
    }
    (packs, ok)
}

fn print_report(report: &ValidationReport) {
    for issue in report.issues() {
        println!("  {issue}");
    }
}

fn build(dirs: &[String]) -> Result<ContentSet, String> {
    let (packs, loaded_ok) = load_all(dirs);
    if !loaded_ok {
        return Err("one or more packs failed to load".into());
    }
    ContentSet::build(packs, ComponentRegistry::builtin()).map_err(|report| {
        println!("content set FAILED to build:");
        print_report(&report);
        format!("{} error(s)", report.error_count())
    })
}

fn pretty(c: &Canon) -> String {
    match serde_json::from_str::<serde_json::Value>(&c.to_canonical_string()) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| c.to_canonical_string()),
        Err(_) => c.to_canonical_string(),
    }
}

pub fn content_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("lint") => lint(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("resolve") => resolve(&args[1..]),
        Some("components") => Ok(components()),
        Some(other) => Err(format!(
            "unknown content command '{other}' (try lint, list, resolve, components)"
        )),
        None => Err("content needs a command: lint, list, resolve, components".into()),
    }
}

fn lint(args: &[String]) -> Result<ExitCode, String> {
    let dirs = pack_dirs(args);
    match build(&dirs) {
        Ok(set) => {
            let warnings = set.warnings().warnings().count();
            println!(
                "OK: {} template(s) from {} pack(s) [load order: {}], {warnings} warning(s)",
                set.len(),
                dirs.len(),
                set.load_order()
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(msg) => {
            println!("LINT FAILED: {msg}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn list(args: &[String]) -> Result<ExitCode, String> {
    let set = build(&pack_dirs(args))?;
    println!(
        "\n{:<24} {:<10} {:<6} {:<28} components",
        "template", "pack", "depth", "tags"
    );
    for id in set.ids() {
        let Some(r) = set.get(id) else { continue };
        let tags = r
            .tags()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let comps = r
            .components()
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{:<24} {:<10} {:<6} {:<28} {comps}",
            id.to_string(),
            set.origin(id).map(ToString::to_string).unwrap_or_default(),
            r.chain().len(),
            tags
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn resolve(args: &[String]) -> Result<ExitCode, String> {
    let id_text = args
        .first()
        .ok_or("content resolve needs <template-id> [pack-dir...]")?;
    let id: TemplateId = id_text.parse().map_err(|e| format!("{e}"))?;
    let set = build(&pack_dirs(&args[1..]))?;
    let Some(resolved) = set.get(&id) else {
        println!("no template named '{id}'. Known templates:");
        for known in set.ids() {
            println!("  {known}");
        }
        return Ok(ExitCode::FAILURE);
    };
    println!("\n{id}");
    println!(
        "  chain (root -> leaf): {}",
        resolved
            .chain()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" > ")
    );
    println!(
        "  tags: {}",
        resolved
            .tags()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  provenance (which template sets or deletes each component):");
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for link in resolved.chain() {
        if let Some(src) = set.source(link) {
            names.extend(src.components.keys().map(ToString::to_string));
        }
    }
    for name in &names {
        let mut parts = Vec::new();
        for link in resolved.chain() {
            if let Some(src) = set.source(link) {
                if let Some((_, op)) = src.components.iter().find(|(k, _)| k.as_str() == name) {
                    parts.push(format!(
                        "{link}{}",
                        if op.is_none() { " (deletes)" } else { "" }
                    ));
                }
            }
        }
        println!("    {name}: {}", parts.join(" -> "));
    }
    println!("  resolved components:");
    for (name, params) in resolved.components() {
        println!("    {name}: {}", params.to_canonical_string());
    }
    println!("\nfull resolved form:\n{}", pretty(&resolved.to_canon()));
    Ok(ExitCode::SUCCESS)
}

fn describe(field: &FieldSchema) -> String {
    match field {
        FieldSchema::Int { min, max } => format!("int {min}..={max}"),
        FieldSchema::Bool => "bool".into(),
        FieldSchema::Text { max_len } => format!("text(<= {max_len})"),
        FieldSchema::Enum(v) => format!("one of [{}]", v.join("|")),
        FieldSchema::Tag => "tag".into(),
        FieldSchema::TemplateRef => "template-id".into(),
        FieldSchema::Tile => "tile [x, y]".into(),
        FieldSchema::EntityId { kind } => format!("{kind} id"),
        FieldSchema::Optional(inner) => format!("optional {}", describe(inner)),
        FieldSchema::List { item, max_len } => format!("list(<= {max_len}) of {}", describe(item)),
        FieldSchema::Object(s) => format!("object {{{}}}", fields_summary(s)),
    }
}

fn fields_summary(schema: &ParamSchema) -> String {
    schema
        .fields
        .iter()
        .map(|(name, f)| {
            let req = if f.default.is_none() { "*" } else { "" };
            format!("{name}{req}: {}", describe(&f.schema))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn components() -> ExitCode {
    println!("registered components (* = required field):\n");
    for def in ComponentRegistry::builtin().iter() {
        println!("{}  - {}", def.name, def.doc);
        for (name, f) in &def.schema.fields {
            let default = f.default.as_ref().map_or_else(
                || "required".to_owned(),
                |d| format!("default {}", d.to_canonical_string()),
            );
            println!("    {name}: {}  ({default})", describe(&f.schema));
        }
        println!();
    }
    ExitCode::SUCCESS
}
