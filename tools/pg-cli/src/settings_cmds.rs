//! `pg settings list | get | set | reset`: the device settings registry (suggestion S-025).

use crate::args::{parse, Spec};
use pg_core::canon::Canon;
use pg_host_os::FsStorage;
use pg_runtime::settings::{device_registry, DeviceFile};
use std::process::ExitCode;

const SPEC: Spec<'static> = Spec {
    values: &["dir"],
    switches: &[],
    optional: &[],
};

const USAGE: &str = "usage: pg settings list | get <id> | set <id> <value> | reset <id>   (--dir <data dir>, default ./pg-data)";

fn show(v: &Canon) -> String {
    match v {
        Canon::Null => "(none)".to_owned(),
        Canon::Str(s) => s.clone(),
        other => other.to_canonical_string(),
    }
}

/// Reads `text` as the value type the setting wants: numbers and booleans as such, `none` as null.
fn typed(text: &str) -> Canon {
    match text {
        "true" => Canon::Bool(true),
        "false" => Canon::Bool(false),
        "none" | "null" => Canon::Null,
        t => t
            .parse::<i64>()
            .map_or_else(|_| Canon::str(t), |n| Canon::Int(i128::from(n))),
    }
}

pub fn settings_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let dir = p.one("dir").unwrap_or("pg-data");
    let storage = FsStorage::new(dir).map_err(|e| e.to_string())?;
    let file = DeviceFile::new(&storage, device_registry());
    let (mut values, report) = file.load();
    if !report.is_ok() {
        println!("note: the settings file has problems; the affected settings use their defaults:\n{report}");
    }
    let reg = file.registry();
    match p.positional.first().map(String::as_str) {
        Some("list") | None => {
            for d in reg.iter() {
                let v = values.get(&d.id).map_or_else(|| "?".to_owned(), show);
                let dflt = d.field.default.as_ref().map_or_else(String::new, show);
                println!(
                    "{:<28} {:<14} default {:<10}{}",
                    d.id,
                    v,
                    dflt,
                    if d.restart_required {
                        "  (restart needed)"
                    } else {
                        ""
                    }
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("get") => {
            let id = p.positional.get(1).ok_or(USAGE)?;
            let v = values.get(id).ok_or_else(|| format!("no setting '{id}'"))?;
            println!("{}", show(v));
            Ok(ExitCode::SUCCESS)
        }
        Some("set") => {
            let (id, text) = (
                p.positional.get(1).ok_or(USAGE)?,
                p.positional.get(2).ok_or(USAGE)?,
            );
            reg.set(&mut values, id, typed(text))?;
            file.save(&values)?;
            println!("{id} = {}", values.get(id).map_or_else(String::new, show));
            Ok(ExitCode::SUCCESS)
        }
        Some("reset") => {
            let id = p.positional.get(1).ok_or(USAGE)?;
            let d = reg.get(id).ok_or_else(|| format!("no setting '{id}'"))?;
            let default = d.field.default.clone().unwrap_or(Canon::Null);
            reg.set(&mut values, id, default)?;
            file.save(&values)?;
            println!(
                "{id} reset to {}",
                values.get(id).map_or_else(String::new, show)
            );
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("unknown settings command '{other}'\n{USAGE}")),
    }
}
