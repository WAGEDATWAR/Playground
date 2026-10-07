//! `pg events list | show`: the typed event catalog (suggestion S-022).

use crate::content_cmds::describe_field;
use pg_core::events::EventCatalog;
use std::process::ExitCode;

const USAGE: &str = "usage: pg events list [category] | show <kind>";

pub fn events_cmd(args: &[String]) -> Result<ExitCode, String> {
    let catalog = EventCatalog::shared();
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let filter = args.get(1).map(String::as_str);
            let mut n = 0;
            for d in catalog
                .iter()
                .filter(|d| filter.is_none_or(|f| d.category.name() == f))
            {
                n += 1;
                println!(
                    "{:<22} {:<11} {} {}",
                    d.kind,
                    d.category.name(),
                    if d.default_visible {
                        "shown "
                    } else {
                        "hidden"
                    },
                    d.summary
                );
            }
            println!("{n} event kind(s)");
            Ok(ExitCode::SUCCESS)
        }
        Some("show") => {
            let kind = args.get(1).ok_or(USAGE)?;
            let d = catalog
                .get(kind)
                .ok_or_else(|| format!("no event kind '{kind}'"))?;
            println!(
                "{}  ({}, {})\n  {}",
                d.kind,
                d.category.name(),
                if d.default_visible {
                    "shown by default"
                } else {
                    "hidden by default"
                },
                d.summary
            );
            for (name, f) in &d.fields.fields {
                println!(
                    "  {name}: {}{}",
                    describe_field(&f.schema),
                    if f.default.is_some() {
                        " (optional)"
                    } else {
                        ""
                    }
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("unknown events command '{other}'\n{USAGE}")),
    }
}
