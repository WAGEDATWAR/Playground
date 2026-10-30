//! `pg residents digest` (suggestion S-070): the story a seeded town tells, as prose.
//!
//! A hash says that something changed; a digest says what. It runs a generated town for some days and writes a
//! short, deterministic text: the firsts (first conversation, friendship, rift, collapse), who talked most, what
//! they talked about, how the relationships ended up and the last notable journal entries. A digest is recorded as
//! a golden under `golden/story-digest-*.txt`; the golden is checked with the other pins (`pg pins`, `pg check
//! --full`), so a change that alters behaviour in a way you can feel shows up as a few lines of prose to review.
//!
//! A golden file begins with a `# ...` line holding its settings, so the file alone says how to regenerate it:
//! `# seed=digest-a town=48x36 residents=14 days=10 content=data/base`.

use crate::args::{parse, parse_size, Spec};
use crate::shared::{load_content, short};
use crate::sim_cmds::{build_demo, DemoSetup};
use pg_content::ContentSet;
use pg_core::id::EntityId;
use pg_core::time::{TICKS_PER_DAY, TICKS_PER_GAME_MINUTE};
use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

const USAGE: &str = "usage: pg residents digest --seed <text> --town WxH [--residents N] [--days N] [--content dir]...\n       pg residents digest --check <golden file>   (regenerate and compare; exit 1 on a difference)\n       pg residents digest --update <golden file>  (regenerate and rewrite the file's text)";

const SPEC: Spec<'static> = Spec {
    values: &[
        "seed",
        "town",
        "residents",
        "days",
        "content",
        "check",
        "update",
    ],
    switches: &[],
    optional: &[],
};

/// What a digest is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestSpec {
    pub seed: String,
    pub w: i32,
    pub h: i32,
    pub residents: u32,
    pub days: u64,
    pub content: Vec<String>,
}

impl DigestSpec {
    fn header(&self) -> String {
        format!(
            "# seed={} town={}x{} residents={} days={} content={}",
            self.seed,
            self.w,
            self.h,
            self.residents,
            self.days,
            self.content.join(",")
        )
    }

    fn from_header(line: &str) -> Result<DigestSpec, String> {
        let bad = || format!("not a digest header: '{line}'");
        let rest = line.strip_prefix("# ").ok_or_else(bad)?;
        let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
        for part in rest.split(' ') {
            let (k, v) = part.split_once('=').ok_or_else(bad)?;
            kv.insert(k, v);
        }
        let (w, h, _) = parse_size(kv.get("town").ok_or_else(bad)?)?;
        Ok(DigestSpec {
            seed: (*kv.get("seed").ok_or_else(bad)?).to_owned(),
            w,
            h,
            residents: kv
                .get("residents")
                .and_then(|v| v.parse().ok())
                .ok_or_else(bad)?,
            days: kv
                .get("days")
                .and_then(|v| v.parse().ok())
                .ok_or_else(bad)?,
            content: kv
                .get("content")
                .ok_or_else(bad)?
                .split(',')
                .map(str::to_owned)
                .collect(),
        })
    }
}

fn clock(tick: u64) -> String {
    let minute = (tick % TICKS_PER_DAY) / TICKS_PER_GAME_MINUTE;
    format!(
        "day {}, {:02}:{:02}",
        tick / TICKS_PER_DAY,
        minute / 60,
        minute % 60
    )
}

/// Runs the town and writes its story. Everything in it comes from the event stream and the final world, so it is
/// as deterministic as the simulation is.
pub fn digest(spec: &DigestSpec, content: &Arc<ContentSet>) -> Result<String, String> {
    let setup = DemoSetup {
        seed: spec.seed.clone(),
        name: "Digest Town".into(),
        map: None,
        pawns: 0,
        town: Some((spec.w, spec.h, 18, spec.residents)),
        nudges: Vec::new(),
        slots: Vec::new(),
        objects: Vec::new(),
        puts: Vec::new(),
        proposals: Vec::new(),
        content: Some(Arc::clone(content)),
        threads: None,
    };
    let mut sim = build_demo(&setup)?;
    let data = content.game();
    let text = |key: &str| content.strings().text("en", key, &[]);
    // Labels from coldest to warmest, so a change of label can be called a rift or a warming.
    let mut ranked: Vec<(i32, String, String)> = data
        .relationships
        .as_ref()
        .map(|r| {
            r.labels
                .iter()
                .map(|l| (l.min, l.id.clone(), text(&l.label_key)))
                .collect()
        })
        .unwrap_or_default();
    ranked.sort();
    let rank = |id: &str| ranked.iter().position(|(_, l, _)| l == id);
    let label_name = |id: &str| {
        ranked
            .iter()
            .find(|(_, l, _)| l == id)
            .map_or_else(|| id.to_owned(), |(_, _, n)| n.clone())
    };
    let id_of = |e: &pg_core::pipeline::Event, key: &str| -> Option<EntityId> {
        e.detail.get(key)?.as_str()?.parse().ok()
    };

    let mut first_talk: Option<(u64, EntityId, EntityId, String, String)> = None;
    let mut first_warm: Option<(u64, EntityId, EntityId, String, String)> = None;
    let mut first_rift: Option<(u64, EntityId, EntityId, String, String)> = None;
    let mut first_collapse: Option<(u64, EntityId)> = None;
    let mut talks: BTreeMap<EntityId, u64> = BTreeMap::new();
    let mut topics: BTreeMap<String, u64> = BTreeMap::new();
    let (mut total_talks, mut warmings, mut rifts, mut collapses) = (0u64, 0u64, 0u64, 0u64);
    let mut plans: BTreeMap<String, u64> = BTreeMap::new();
    for _ in 0..spec.days * TICKS_PER_DAY {
        let report = sim.step().map_err(|e| e.to_string())?;
        for e in &report.events {
            match e.kind.as_str() {
                "conversation.closed" => {
                    total_talks += 1;
                    let (Some(a), Some(b)) = (id_of(e, "a"), id_of(e, "b")) else {
                        continue;
                    };
                    *talks.entry(a).or_default() += 1;
                    *talks.entry(b).or_default() += 1;
                    let topic = e
                        .detail
                        .get("topic")
                        .and_then(|c| c.as_str())
                        .unwrap_or("?")
                        .to_owned();
                    let tone = e
                        .detail
                        .get("tone")
                        .and_then(|c| c.as_str())
                        .unwrap_or("?")
                        .to_owned();
                    *topics.entry(topic.clone()).or_default() += 1;
                    if first_talk.is_none() {
                        first_talk = Some((e.tick, a, b, topic, tone));
                    }
                }
                "relationship.label_changed" => {
                    let (Some(a), Some(b)) = (id_of(e, "a"), id_of(e, "b")) else {
                        continue;
                    };
                    let from = e
                        .detail
                        .get("from")
                        .and_then(|c| c.as_str())
                        .unwrap_or("?")
                        .to_owned();
                    let to = e
                        .detail
                        .get("to")
                        .and_then(|c| c.as_str())
                        .unwrap_or("?")
                        .to_owned();
                    if rank(&to) < rank(&from) {
                        rifts += 1;
                        if first_rift.is_none() {
                            first_rift = Some((e.tick, a, b, from, to));
                        }
                    } else {
                        warmings += 1;
                        if first_warm.is_none() {
                            first_warm = Some((e.tick, a, b, from, to));
                        }
                    }
                }
                kind @ ("commitment.proposed"
                | "commitment.completed"
                | "commitment.failed"
                | "commitment.declined"
                | "commitment.expired") => {
                    *plans.entry(kind.to_owned()).or_default() += 1;
                }
                "pawn.collapsed" => {
                    collapses += 1;
                    if first_collapse.is_none() {
                        if let Some(p) = id_of(e, "pawn") {
                            first_collapse = Some((e.tick, p));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let world = sim.world();
    let name = |id: EntityId| {
        world
            .pawns
            .get(id)
            .map_or_else(|| id.to_string(), |p| p.name.clone())
    };
    // Topics are named by their ids (`small_talk` reads as "small talk").
    let topic_name = |id: &str| id.replace('_', " ");
    let mut out = String::new();
    out.push_str(&format!(
        "{}\nStory digest: seed '{}', a {}x{} town, {} residents, {} days. Final state {}.\n\n",
        spec.header(),
        spec.seed,
        spec.w,
        spec.h,
        world.pawns.len(),
        spec.days,
        short(&world.state_hash())
    ));
    out.push_str("Firsts\n");
    match &first_talk {
        Some((t, a, b, topic, tone)) => out.push_str(&format!(
            "  first conversation: {} - {} and {} ({}, {tone})\n",
            clock(*t),
            name(*a),
            name(*b),
            topic_name(topic)
        )),
        None => out.push_str("  first conversation: none\n"),
    }
    for (what, found) in [("first warming", &first_warm), ("first rift", &first_rift)] {
        match found {
            Some((t, a, b, from, to)) => out.push_str(&format!(
                "  {what}: {} - {} and {}, {} to {}\n",
                clock(*t),
                name(*a),
                name(*b),
                label_name(from),
                label_name(to)
            )),
            None => out.push_str(&format!("  {what}: none\n")),
        }
    }
    match first_collapse {
        Some((t, p)) => out.push_str(&format!("  first collapse: {} - {}\n", clock(t), name(p))),
        None => out.push_str("  first collapse: none\n"),
    }
    let mut busy: Vec<(EntityId, u64)> = talks.into_iter().collect();
    busy.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.push_str("\nWho talked most\n");
    for (p, n) in busy.iter().take(3) {
        out.push_str(&format!("  {}: {n} conversations\n", name(*p)));
    }
    if busy.is_empty() {
        out.push_str("  nobody\n");
    }
    out.push_str(&format!(
        "\nThe town: {total_talks} conversations, {warmings} relationships warmed, {rifts} soured, {collapses} collapses.\n"
    ));
    let plan = |kind: &str| {
        plans
            .get(&format!("commitment.{kind}"))
            .copied()
            .unwrap_or(0)
    };
    out.push_str(&format!(
        "  plans: {} asked, {} kept, {} broken, {} declined, {} unanswered.
",
        plan("proposed"),
        plan("completed"),
        plan("failed"),
        plan("declined"),
        plan("expired")
    ));
    let mut by_topic: Vec<(&String, &u64)> = topics.iter().collect();
    by_topic.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    out.push_str(&format!(
        "  topics: {}\n",
        by_topic
            .iter()
            .map(|(t, n)| format!("{} {n}", topic_name(t)))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let mut by_label: BTreeMap<String, u32> = BTreeMap::new();
    for r in world.relationships.iter() {
        *by_label.entry(r.label.clone()).or_default() += 1;
    }
    out.push_str(&format!(
        "  relationships ({}): {}\n",
        world.relationships.len(),
        ranked
            .iter()
            .rev()
            .filter_map(|(_, id, n)| by_label.get(id).map(|c| format!("{n} {c}")))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let mut pairs: Vec<_> = world.relationships.iter().collect();
    pairs.sort_by(|a, b| b.affinity.cmp(&a.affinity).then(a.key.cmp(&b.key)));
    if let (Some(best), Some(worst)) = (pairs.first(), pairs.last()) {
        out.push_str(&format!(
            "  closest pair: {} and {} ({}); coolest: {} and {} ({})\n",
            name(best.key.a),
            name(best.key.b),
            best.affinity,
            name(worst.key.a),
            name(worst.key.b),
            worst.affinity
        ));
    }
    out.push_str("\nFrom the journal (the most recent notable entries)\n");
    let notable: Vec<_> = world
        .journal
        .entries()
        .iter()
        .filter(|e| e.weight >= 2)
        .collect();
    for e in notable.iter().rev().take(6).rev() {
        // An argument named `x_key` holds a string-table key that fills `{x}` (the rule the journal window uses).
        let resolved: Vec<(String, String)> = e
            .args
            .iter()
            .map(|(k, v)| match k.strip_suffix("_key") {
                Some(base) => (base.to_owned(), text(v)),
                None => (k.clone(), v.clone()),
            })
            .collect();
        let params: Vec<(&str, &str)> = resolved
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        out.push_str(&format!(
            "  {} - {}\n",
            clock(e.tick),
            content
                .strings()
                .text("en", &format!("journal.{}", e.kind), &params)
        ));
    }
    if notable.is_empty() {
        out.push_str("  nothing notable\n");
    }
    Ok(out)
}

/// Reads a golden file: its settings line and its text.
fn read_golden(path: &str) -> Result<(DigestSpec, String), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let first = text.lines().next().unwrap_or("");
    Ok((DigestSpec::from_header(first)?, text))
}

pub fn digest_cmd(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    if let Some(path) = p.one("check").or_else(|| p.one("update")) {
        let updating = p.one("update").is_some();
        let (spec, old) = read_golden(path)?;
        let content = load_content(&spec.content)?;
        let new = digest(&spec, &content)?;
        if updating {
            std::fs::write(path, &new).map_err(|e| format!("cannot write {path}: {e}"))?;
            println!("rewrote {path}");
            return Ok(ExitCode::SUCCESS);
        }
        if new == old {
            println!(
                "{path}: story digest unchanged ({} lines)",
                new.lines().count()
            );
            return Ok(ExitCode::SUCCESS);
        }
        println!("{path}: STORY DIGEST CHANGED. Lines that differ (- recorded, + now):");
        let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
        for i in 0..a.len().max(b.len()) {
            let (x, y) = (
                a.get(i).copied().unwrap_or(""),
                b.get(i).copied().unwrap_or(""),
            );
            if x != y {
                println!("  - {x}\n  + {y}");
            }
        }
        println!("If the change is intended, run: pg residents digest --update {path}");
        return Ok(ExitCode::FAILURE);
    }
    if p.one("seed").is_none() || p.one("town").is_none() {
        return Err(USAGE.into());
    }
    let (w, h, _) = parse_size(p.one("town").unwrap_or("48x36"))?;
    let mut dirs: Vec<String> = p.all("content").into_iter().map(str::to_owned).collect();
    if dirs.is_empty() {
        dirs.push(crate::shared::DEFAULT_CONTENT_DIR.to_owned());
    }
    let spec = DigestSpec {
        seed: p.one("seed").unwrap_or("digest").to_owned(),
        w,
        h,
        residents: p.parse::<u32>("residents")?.unwrap_or(14),
        days: p.parse::<u64>("days")?.unwrap_or(10),
        content: dirs.clone(),
    };
    let content = load_content(&dirs)?;
    print!("{}", digest(&spec, &content)?);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_round_trips_and_a_bad_one_is_refused() {
        let spec = DigestSpec {
            seed: "digest-a".into(),
            w: 48,
            h: 36,
            residents: 14,
            days: 10,
            content: vec!["data/base".into()],
        };
        assert_eq!(DigestSpec::from_header(&spec.header()).unwrap(), spec);
        assert!(DigestSpec::from_header("seed=x").is_err());
        assert!(DigestSpec::from_header("# seed=x town=48x36").is_err());
    }

    #[test]
    fn clock_text_is_day_and_time() {
        assert_eq!(clock(0), "day 0, 00:00");
        assert_eq!(clock(TICKS_PER_DAY + 10 * 61), "day 1, 01:01");
    }
}
