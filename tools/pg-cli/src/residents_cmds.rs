//! Developer commands for residents (Stage 1): `residents generate` (1.0) and `residents inspect` (1.4).

use crate::args::{parse, Spec};
use crate::shared::{load_content, DEFAULT_CONTENT_DIR};
use crate::sim_cmds::{build_demo, demo_from_flags, SIM_SPEC};
use pg_core::id::EntityId;
use pg_core::memory::{select_relevant_memories, Context};
use pg_core::population::{plan_population, PopulationPlan};
use pg_core::rng::Seed;
use std::process::ExitCode;

pub fn residents_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.first().map(String::as_str) {
        Some("generate") => generate(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        Some("report") => report(&args[1..]),
        Some(other) => Err(format!(
            "unknown residents command '{other}' (try generate, inspect or report)"
        )),
        None => Err("residents needs a command: generate, inspect or report".into()),
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

const INSPECT_USAGE: &str = "usage: pg residents inspect <pawn> --seed <text> --town WxH [--residents N] [--content dir] [--days N | --at TICK] [--topic T] [--with PAWN]";

/// `pg residents inspect <pawn> ...`: builds the demo world, runs it, and shows one resident the way the
/// inspector will: needs, mood, conversation, memories (with the ones that matter most and why) and
/// relationships.
fn inspect(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SIM_SPEC)?;
    let pawn_text = p.positional.first().ok_or(INSPECT_USAGE)?;
    let id: EntityId = pawn_text
        .parse()
        .map_err(|e| format!("pawn '{pawn_text}': {e}"))?;
    let mut setup = demo_from_flags(&p)?;
    let content = setup
        .content
        .clone()
        .map_or_else(|| load_content(&[DEFAULT_CONTENT_DIR.to_owned()]), Ok)?;
    setup.content = Some(content.clone());
    let mut sim = build_demo(&setup)?;
    let ticks = match (p.parse::<u64>("ticks")?, p.parse::<u64>("days")?) {
        (Some(t), _) => t,
        (None, Some(d)) => d * pg_core::time::TICKS_PER_DAY,
        (None, None) => pg_core::time::TICKS_PER_DAY,
    };
    sim.run_ticks(ticks).map_err(|e| e.to_string())?;
    let world = sim.world();
    let pawn = world
        .pawns
        .get(id)
        .ok_or_else(|| format!("no pawn {id} (the world has {})", world.pawns.len()))?;
    let data = content.game();
    let text = |key: &str, params: &[(&str, &str)]| content.strings().text("en", key, params);
    let name = |who: EntityId| {
        world
            .pawns
            .get(who)
            .map_or_else(|| who.to_string(), |p| p.name.clone())
    };
    let job = pawn
        .occupation
        .as_ref()
        .and_then(|o| data.occupations.get(&o.template))
        .map_or_else(|| "no occupation".to_owned(), |o| text(&o.name_key, &[]));
    println!(
        "{} ({id})  {job}  mood {}  at {}  tick {}",
        pawn.name,
        pawn.mood,
        pawn.position.tile,
        world.clock.tick()
    );
    let needs: Vec<String> = pawn.needs.iter().map(|(k, v)| format!("{k} {v}")).collect();
    println!("needs: {}", needs.join(", "));
    match &pawn.talk {
        Some(t) => println!(
            "talking with {} about {} ({} tone) until tick {}",
            name(t.partner),
            t.topic,
            t.tone,
            t.ends
        ),
        None => println!("not in a conversation"),
    }
    let summary = |m: &pg_core::social::Memory| {
        let other = m
            .participants
            .first()
            .map_or_else(String::new, |o| name(*o));
        match &m.summary_key {
            Some(k) => text(k, &[("other", &other)]),
            None => format!("{} with {other}", m.ty),
        }
    };
    println!(
        "
memories ({}), newest first:",
        pawn.memories.len()
    );
    let mut newest: Vec<_> = pawn.memories.iter().collect();
    newest.sort_by(|a, b| b.tick.cmp(&a.tick).then(a.id.cmp(&b.id)));
    for m in newest.iter().take(12) {
        println!(
            "  {} tick {:>6}  importance {:>3}  retention {:>4}{}  {}",
            m.id,
            m.tick,
            m.importance,
            m.retention,
            if data
                .memory
                .as_ref()
                .is_some_and(|mp| pg_core::memory::is_persistent(m, mp))
            {
                "  persistent"
            } else {
                ""
            },
            summary(m)
        );
    }
    // The last few conversations, as remembered: recorded words, or the fallback lines rebuilt from the data.
    if let Some(params) = data.conversation.as_ref() {
        println!(
            "
recent conversations, as remembered:"
        );
        let given = |who: EntityId| {
            name(who)
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        for m in newest.iter().filter(|m| m.talk.is_some()).take(3) {
            let Some(lines) = pg_core::conversation::recall(world, params, id, m) else {
                continue;
            };
            let kept = m.talk.as_ref().is_some_and(|t| !t.lines.is_empty());
            println!(
                "  {} ({}){}",
                summary(m),
                m.talk.as_ref().map_or("", |t| t.tone.as_str()),
                if kept {
                    "  [recorded words]"
                } else {
                    "  [fallback lines]"
                }
            );
            for l in lines {
                let said = match &l.said {
                    pg_core::conversation::Spoken::Written(t) => t.clone(),
                    pg_core::conversation::Spoken::Key { key, memory } => {
                        let phrase = memory.as_ref().map_or_else(String::new, |m| {
                            text(&pg_content::gamedata::memory_phrase_key(m), &[])
                        });
                        text(
                            key,
                            &[
                                ("name", &given(l.speaker)),
                                ("other", &given(l.listener)),
                                ("memory", &phrase),
                            ],
                        )
                    }
                };
                println!("      {}: {said}", name(l.speaker));
            }
        }
    }
    let with = p
        .one("with")
        .map(|w| {
            w.parse::<EntityId>()
                .map_err(|e| format!("--with '{w}': {e}"))
        })
        .transpose()?;
    let ctx = Context {
        now: world.clock.tick(),
        with,
        topic: p.one("topic"),
    };
    println!(
        "
what matters most now:"
    );
    for r in select_relevant_memories(&pawn.memories, &ctx, 5) {
        let why: Vec<String> = r.reasons.iter().map(|x| x.text()).collect();
        println!(
            "  {:>4}  {}  [{}]",
            r.score,
            summary(r.memory),
            why.join(", ")
        );
    }
    let label = |key: &str| text(key, &[]);
    println!(
        "
relationships:"
    );
    for r in world.relationships.of(id) {
        let other = r.key.other(id).unwrap_or(id);
        let l = data
            .relationships
            .as_ref()
            .and_then(|rp| rp.labels.iter().find(|l| l.id == r.label))
            .map_or_else(|| r.label.clone(), |l| label(&l.label_key));
        println!(
            "  {:<22} {:>5} {:<13} last talked about {:<12} forgotten {}",
            name(other),
            r.affinity,
            l,
            r.last_topic.as_deref().unwrap_or("-"),
            r.forgotten
        );
    }
    Ok(ExitCode::SUCCESS)
}

const REPORT_USAGE: &str = "usage: pg residents report --seed <text> --town WxH [--residents N] [--days N] [--dot FILE] [--content dir]...";

/// `pg residents report`: how a town's social life went (suggestion S-069). Loneliness, moods, how many
/// conversations a day and on what, how the relationships are labelled, the strongest and weakest pairs, and
/// the friendship graph (as text, and as Graphviz with `--dot`). A way to see whether tuning did what was meant.
fn report(args: &[String]) -> Result<ExitCode, String> {
    use std::collections::BTreeMap;
    let p = parse(args, &SIM_SPEC)?;
    if p.one("seed").is_none() {
        return Err(REPORT_USAGE.into());
    }
    let mut setup = demo_from_flags(&p)?;
    let content = setup
        .content
        .clone()
        .map_or_else(|| load_content(&[DEFAULT_CONTENT_DIR.to_owned()]), Ok)?;
    setup.content = Some(content.clone());
    let mut sim = build_demo(&setup)?;
    let days = match (p.parse::<u64>("days")?, p.parse::<u64>("ticks")?) {
        (Some(d), _) => d,
        (None, Some(t)) => t.div_ceil(pg_core::time::TICKS_PER_DAY),
        (None, None) => 7,
    };
    let mut per_day: Vec<u64> = vec![0; days as usize];
    let mut topics: BTreeMap<String, u64> = BTreeMap::new();
    let mut tones: BTreeMap<String, u64> = BTreeMap::new();
    let mut cancelled = 0u64;
    let mut label_changes = 0u64;
    for _ in 0..days * pg_core::time::TICKS_PER_DAY {
        let r = sim.step().map_err(|e| e.to_string())?;
        for e in &r.events {
            match e.kind.as_str() {
                "conversation.closed" => {
                    let day = (e.tick / pg_core::time::TICKS_PER_DAY) as usize;
                    if let Some(d) = per_day.get_mut(day) {
                        *d += 1;
                    }
                    for (map, field) in [(&mut topics, "topic"), (&mut tones, "tone")] {
                        if let Some(v) = e.detail.get(field).and_then(|c| c.as_str()) {
                            *map.entry(v.to_owned()).or_default() += 1;
                        }
                    }
                }
                "conversation.cancelled" => cancelled += 1,
                "relationship.label_changed" => label_changes += 1,
                _ => {}
            }
        }
    }
    let world = sim.world();
    let data = content.game();
    let text = |key: &str| content.strings().text("en", key, &[]);
    let name = |id: EntityId| {
        world
            .pawns
            .get(id)
            .map_or_else(|| id.to_string(), |p| p.name.clone())
    };
    println!(
        "social report: {} resident(s), {days} day(s), seed '{}'
",
        world.pawns.len(),
        setup.seed
    );
    println!("conversations per day: {:?}  (total {}, called off {cancelled}, label changes {label_changes})", per_day, per_day.iter().sum::<u64>());
    let fmt = |m: &BTreeMap<String, u64>| {
        m.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "topics: {}
tones:  {}
",
        fmt(&topics),
        fmt(&tones)
    );
    // Social need, in fifths.
    let mut lonely = [0u32; 5];
    let mut moods: BTreeMap<String, u32> = BTreeMap::new();
    for (_, pw) in world.pawns.iter() {
        let level = pw.needs.get("social").copied().unwrap_or(0).clamp(0, 999);
        lonely[(level / 200) as usize] += 1;
        *moods.entry(pw.mood.clone()).or_default() += 1;
    }
    println!("social need now (residents per band, lowest first):");
    for (i, n) in lonely.iter().enumerate() {
        println!(
            "  {:>4}-{:<4} {:<3} {}",
            i * 200,
            i * 200 + 199,
            n,
            "#".repeat(*n as usize)
        );
    }
    println!(
        "
moods: {}",
        fmt(&moods
            .iter()
            .map(|(k, v)| (k.clone(), u64::from(*v)))
            .collect())
    );
    let mut labels: BTreeMap<String, u32> = BTreeMap::new();
    for r in world.relationships.iter() {
        *labels.entry(r.label.clone()).or_default() += 1;
    }
    let label_name = |id: &str| {
        data.relationships
            .as_ref()
            .and_then(|rp| rp.labels.iter().find(|l| l.id == id))
            .map_or_else(|| id.to_owned(), |l| text(&l.label_key))
    };
    println!(
        "
relationships ({}): {}",
        world.relationships.len(),
        labels
            .iter()
            .map(|(k, v)| format!("{} {v}", label_name(k)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut pairs: Vec<_> = world.relationships.iter().collect();
    pairs.sort_by(|a, b| b.affinity.cmp(&a.affinity).then(a.key.cmp(&b.key)));
    println!(
        "
strongest pairs:"
    );
    for r in pairs.iter().take(5) {
        println!(
            "  {:>5}  {} + {}  ({})",
            r.affinity,
            name(r.key.a),
            name(r.key.b),
            label_name(&r.label)
        );
    }
    println!("weakest pairs:");
    for r in pairs.iter().rev().take(5) {
        println!(
            "  {:>5}  {} + {}  ({})",
            r.affinity,
            name(r.key.a),
            name(r.key.b),
            label_name(&r.label)
        );
    }
    // The friendship graph: friends and better.
    let friend_min = data
        .relationships
        .as_ref()
        .and_then(|rp| rp.labels.iter().find(|l| l.id == "friend"))
        .map_or(550, |l| l.min);
    let friends: Vec<_> = world
        .relationships
        .iter()
        .filter(|r| r.affinity >= friend_min)
        .collect();
    println!(
        "
friendship graph ({} edge(s), affinity {friend_min} and up):",
        friends.len()
    );
    for r in &friends {
        println!("  {} -- {}  ({})", name(r.key.a), name(r.key.b), r.affinity);
    }
    if let Some(path) = p.one("dot") {
        let mut dot = String::from(
            "graph town {
  node [shape=ellipse];
",
        );
        for r in &friends {
            dot.push_str(&format!(
                "  \"{}\" -- \"{}\" [label=\"{}\"];
",
                name(r.key.a),
                name(r.key.b),
                r.affinity
            ));
        }
        dot.push_str(
            "}
",
        );
        std::fs::write(path, dot).map_err(|e| format!("cannot write {path}: {e}"))?;
        println!(
            "
Graphviz written to {path}"
        );
    }
    Ok(ExitCode::SUCCESS)
}
