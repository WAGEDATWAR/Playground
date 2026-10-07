//! `pg strings`: inspect and lint the string tables (suggestion S-026).

use crate::shared::{load_content, DEFAULT_CONTENT_DIR};
use pg_content::strings::{pseudo, StringLint};
use pg_core::reason::ReasonCode;
use std::collections::BTreeSet;
use std::process::ExitCode;

const USAGE: &str = "usage: pg strings lint [pack-dir...] | show <key> [--locale L] [--param name=value]... | pseudo <text>";

fn dirs(args: &[String]) -> Vec<String> {
    let d: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if d.is_empty() {
        vec![DEFAULT_CONTENT_DIR.to_owned()]
    } else {
        d
    }
}

pub fn strings_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, rest)) if c == "lint" => lint(rest),
        Some((c, rest)) if c == "show" => show(rest),
        Some((c, rest)) if c == "pseudo" => {
            println!("{}", pseudo(&rest.join(" ")));
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.into()),
    }
}

fn lint(args: &[String]) -> Result<ExitCode, String> {
    let set = load_content(&dirs(args))?;
    let strings = set.strings();
    // Keys the program refers to: every built-in reason sentence.
    let used: BTreeSet<String> = ReasonCode::known_codes()
        .map(|c| format!("reason.{c}"))
        .collect();
    let mut report: StringLint = strings.lint(Some(&used));
    // Only reason.* keys can be judged "unused" from code; other keys belong to menus and packs.
    report.unused.retain(|k| k.starts_with("reason."));
    println!(
        "{} locale(s): {}",
        strings.locales().len(),
        strings
            .locales()
            .iter()
            .map(|l| format!("{l} ({} keys)", strings.len(l)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (l, k) in &report.missing {
        println!("  missing in {l}: {k}");
    }
    for (l, k) in &report.orphaned {
        println!("  {l} has a key with no English original: {k}");
    }
    for (l, k) in &report.placeholder_mismatch {
        println!("  {l}: placeholders differ from English in {k}");
    }
    for k in &report.unused {
        println!("  English key nothing uses: {k}");
    }
    if strings.is_empty() {
        println!("  no strings are defined");
        return Ok(ExitCode::FAILURE);
    }
    if report.is_clean() {
        println!("strings OK");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("strings have problems");
        Ok(ExitCode::FAILURE)
    }
}

fn show(args: &[String]) -> Result<ExitCode, String> {
    let key = args.first().ok_or(USAGE)?;
    let mut locale = "en".to_owned();
    let mut params: Vec<(String, String)> = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--locale" => locale = it.next().ok_or("--locale needs a value")?.clone(),
            "--param" => {
                let kv = it.next().ok_or("--param needs name=value")?;
                let (k, v) = kv.split_once('=').ok_or("--param needs name=value")?;
                params.push((k.to_owned(), v.to_owned()));
            }
            other => return Err(format!("unknown option '{other}'\n{USAGE}")),
        }
    }
    let set = load_content(&[DEFAULT_CONTENT_DIR.to_owned()])?;
    let p: Vec<(&str, &str)> = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    println!("{}", set.strings().text(&locale, key, &p));
    Ok(ExitCode::SUCCESS)
}
