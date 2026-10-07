//! Developer commands for the scheduler and actions: `schedule explain`, `actions` (0.5).

use crate::args::parse;
use crate::content_cmds::describe_field;
use crate::sim_cmds::{build_demo, demo_from_flags, SIM_SPEC};
use pg_core::action::{ActionRegistry, StepTemplate};
use pg_core::id::EntityId;
use pg_core::pawn::{Intent, Pawn};
use pg_core::schedule::DaySchedule;
use pg_core::time::TICKS_PER_DAY;
use std::process::ExitCode;

const USAGE: &str = "usage: pg schedule explain <pawn> --seed <text> --dev-map WxH --dev-pawns <n> [--day <n>] [--at <tick>] [sim flags]";

pub fn schedule_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((sub, rest)) if sub == "explain" => explain(rest),
        _ => Err(USAGE.into()),
    }
}

/// `HH:MM` of the start of `slot`.
fn clock(slot: u32, slot_minutes: u32) -> String {
    let m = u64::from(slot) * u64::from(slot_minutes);
    format!("{:02}:{:02}", m / 60 % 24, m % 60)
}

fn explain(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SIM_SPEC)?;
    let pawn_text = p.positional.first().ok_or(USAGE)?;
    let pawn_id: EntityId = pawn_text
        .parse()
        .map_err(|e| format!("pawn '{pawn_text}': {e}"))?;
    let day = p.parse::<u64>("day")?.unwrap_or(0);
    let setup = demo_from_flags(&p)?;
    let mut sim = build_demo(&setup)?;
    // Default: just after the day's first slot boundary, when the day's plan exists even for day 0.
    let at = p.parse::<u64>("at")?.unwrap_or(day * TICKS_PER_DAY + 301);
    sim.run_ticks(at).map_err(|e| e.to_string())?;
    let world = sim.world();
    let pawn = world
        .pawns
        .get(pawn_id)
        .ok_or_else(|| format!("no pawn {pawn_id} (the world has {})", world.pawns.len()))?;
    let minutes = world.settings.slot_minutes.get();
    print_pawn(pawn, world.clock.tick(), minutes);
    match &pawn.schedule {
        None => println!("\nno schedule yet (planning happens at a slot boundary)"),
        Some(s) => print_schedule(s, minutes),
    }
    let mine: Vec<_> = world
        .commitments
        .iter()
        .filter(|(_, c)| c.parties().contains(&pawn_id))
        .collect();
    if !mine.is_empty() {
        println!("\ncommitments:");
        for (id, c) in mine {
            let why = c.reason.as_ref().map_or(String::new(), |r| r.explain());
            println!(
                "  {id}  {} {}  slots {}..{} ({}-{})  {}  {why}",
                if c.proposer == pawn_id {
                    "proposed to"
                } else {
                    "invited by"
                },
                if c.proposer == pawn_id {
                    c.invitee
                } else {
                    c.proposer
                },
                c.start,
                c.start + c.len,
                clock(c.start, minutes),
                clock(c.start + c.len, minutes),
                c.state.name()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_pawn(p: &Pawn, tick: u64, minutes: u32) {
    let slot = (tick % TICKS_PER_DAY) / (u64::from(minutes) * 10);
    println!(
        "{} ({})  at {} on {}   tick {tick}  day {}  slot {slot} ({})",
        p.name,
        p.id,
        p.position.tile,
        p.position.map,
        tick / TICKS_PER_DAY,
        clock(u32::try_from(slot).unwrap_or(0), minutes)
    );
    let intent = match p.intent {
        Intent::Free => "free".to_owned(),
        Intent::Reservation(id) => format!("reservation {id}"),
    };
    let task = p.task.as_ref().map_or("none".to_owned(), |t| {
        format!(
            "{} (step {}/{}{})",
            t.action,
            t.current.min(t.steps.len()),
            t.steps.len(),
            if t.is_done() { ", done" } else { "" }
        )
    });
    println!("intent: {intent}   task: {task}");
    if let Some(r) = &p.last_failure {
        println!("last failure: {}", r.explain());
    }
    if let Some(r) = &p.replan {
        println!("replan pending from slot {}: {}", r.from, r.why);
    }
}

fn print_schedule(s: &DaySchedule, minutes: u32) {
    let n = s.slots_per_day();
    println!(
        "\nschedule for day {} ({n} slots of {minutes} min): {} reserved, {} open",
        s.day,
        n - s.free_slots(),
        s.free_slots()
    );
    for r in s.by_start() {
        println!(
            "  slots {:>2}..{:<2} {}-{}  {:<11} {} {}  [res {}]",
            r.start,
            r.end(),
            clock(r.start, minutes),
            clock(r.end(), minutes),
            r.priority.name(),
            r.action,
            r.params.to_canonical_string(),
            r.id
        );
        println!("        why: {}", r.reason.explain());
    }
    let open = open_ranges(s);
    if !open.is_empty() {
        let text: Vec<String> = open
            .iter()
            .map(|(a, b)| format!("{}-{}", clock(*a, minutes), clock(*b, minutes)))
            .collect();
        println!("  open: {}", text.join(", "));
    }
    if !s.dropped.is_empty() {
        println!("not placed:");
        for d in &s.dropped {
            println!("  - {}: {}", d.what, d.reason.explain());
        }
    }
    if !s.notes.is_empty() {
        println!("notes:");
        for r in &s.notes {
            println!("  - {}", r.explain());
        }
    }
}

fn open_ranges(s: &DaySchedule) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for slot in 0..s.slots_per_day() {
        if s.owner_id(slot).is_some() {
            continue;
        }
        match out.last_mut() {
            Some((_, end)) if *end == slot => *end = slot + 1,
            _ => out.push((slot, slot + 1)),
        }
    }
    out
}

pub fn actions_cmd() -> ExitCode {
    let registry = ActionRegistry::builtin();
    println!(
        "{} action(s), registry {}",
        registry.iter().count(),
        if registry.is_frozen() {
            "closed"
        } else {
            "open"
        }
    );
    for d in registry.iter() {
        let origin = match &d.origin {
            pg_content::Origin::Builtin => "builtin".to_owned(),
            pg_content::Origin::Pack(p) => format!("pack {p}"),
        };
        println!("\n{}  ({origin})  {}", d.id, d.summary);
        for (name, f) in &d.params.fields {
            println!(
                "    param {name}: {}{}",
                describe_field(&f.schema),
                if f.default.is_some() {
                    "  (optional)"
                } else {
                    ""
                }
            );
        }
        let steps: Vec<String> = d
            .steps
            .iter()
            .map(|s| match s {
                StepTemplate::MoveTo { param, within: 0 } => format!("move to {param}"),
                StepTemplate::MoveTo { param, within } => {
                    format!("move to within {within} of {param}")
                }
                StepTemplate::PerformUntilSlotEnd => "stay until the reservation ends".to_owned(),
            })
            .collect();
        println!("    steps: {}", steps.join(" -> "));
        println!(
            "    interruptible: {}   ai-proposable: {}",
            d.interruptible, d.ai_proposable
        );
    }
    ExitCode::SUCCESS
}
