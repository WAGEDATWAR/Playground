use super::*;
use crate::commands::Command;
use crate::hooks::{HookHost, HookPoint};
use crate::id::Kind;
use crate::input::SimInput;
use crate::map::Tile;
use crate::pawn::{Position, Task};
use crate::sim::Sim;
use pg_content::ActionId;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};

fn content() -> Arc<ContentSet> {
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap())
}

fn data() -> Arc<GameData> {
    Arc::new(content().game().clone())
}

fn town(seed: &str) -> Sim {
    let mut sim = Sim::new(
        WorldState::new("Talk", seed),
        crate::pipeline::Pipeline::new(),
    )
    .with_content(content());
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::GenerateTown {
                w: 48,
                h: 36,
                water: 15,
                residents: 12,
                tone: "standard".into(),
            },
        },
    )
    .unwrap();
    sim
}

fn run_collect(sim: &mut Sim, ticks: u64, kinds_wanted: &[&str]) -> Vec<(String, Canon)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let r = sim.step().unwrap();
        for e in r.events {
            if kinds_wanted.contains(&e.kind.as_str()) {
                out.push((e.kind, e.detail));
            }
        }
        // Whatever happens, talks are reciprocal and a pawn is in at most one.
        for (id, p) in sim.world().pawns.iter() {
            if let Some(t) = &p.talk {
                let other = sim
                    .world()
                    .pawns
                    .get(t.partner)
                    .and_then(|o| o.talk.as_ref());
                assert!(
                    other.is_some_and(|o| o.partner == id && o.started == t.started),
                    "{id} talks to {} but not the other way round",
                    t.partner
                );
                assert_ne!(t.leader, other.is_some_and(|o| o.leader));
            }
        }
    }
    out
}

#[test]
fn residents_talk_and_each_talk_changes_the_pair_and_leaves_memories() {
    let mut sim = town("talk-seed");
    let events = run_collect(
        &mut sim,
        2 * 14_400,
        &[
            "conversation.started",
            "conversation.closed",
            "memory.created",
        ],
    );
    let closed: Vec<_> = events
        .iter()
        .filter(|(k, _)| k == "conversation.closed")
        .collect();
    assert!(
        closed.len() >= 20,
        "{} conversations in two days",
        closed.len()
    );
    let created = events.iter().filter(|(k, _)| k == "memory.created").count();
    assert_eq!(created, closed.len() * 2, "one memory per participant");
    let d = data();
    let (mem, rel) = (
        d.memory.as_ref().unwrap(),
        d.relationships.as_ref().unwrap(),
    );
    let w = sim.world();
    for (_, p) in w.pawns.iter() {
        let ordinary = p
            .memories
            .iter()
            .filter(|m| !crate::memory::is_persistent(m, mem))
            .count();
        assert!(ordinary <= mem.max_memories as usize, "bounded memories");
        assert!(p.memories.iter().any(|m| m.ty == "conversation"));
    }
    for r in w.relationships.iter() {
        assert!((-1000..=1000).contains(&r.affinity));
        assert!(r.day_change.abs() <= rel.daily_cap, "{r:?}");
        assert!(rel.labels.iter().any(|l| l.id == r.label));
    }
    // Every closed talk names a topic and tone from the data.
    let params = d.conversation.as_ref().unwrap();
    for (_, detail) in closed {
        let topic = detail.get("topic").and_then(Canon::as_str).unwrap();
        let tone = detail.get("tone").and_then(Canon::as_str).unwrap();
        assert!(params.topic(topic).is_some() && params.tone(tone).is_some());
    }
}

#[test]
fn conversations_are_deterministic_and_survive_a_snapshot() {
    let build = || town("repeat");
    let mut whole = build();
    whole.run_ticks(20_000).unwrap();
    let mut again = build();
    again.run_ticks(20_000).unwrap();
    assert_eq!(whole.world().state_hash(), again.world().state_hash());
    // A snapshot taken mid-run (talks in progress included) resumes to the same place.
    let mut first = build();
    first.run_ticks(7_000).unwrap();
    let mut resumed =
        Sim::restore(first.snapshot(), crate::pipeline::Pipeline::new()).with_content(content());
    resumed.run_ticks(13_000).unwrap();
    assert_eq!(resumed.world().state_hash(), whole.world().state_hash());
    assert!(whole
        .world()
        .relationships
        .iter()
        .any(|r| r.last_interaction_tick > 0));
}

#[test]
fn history_survives_a_save_and_load() {
    let mut sim = town("history");
    sim.run_ticks(30_000).unwrap();
    let w = sim.world();
    let text = w.to_canon().to_canonical_string();
    let back = WorldState::from_canon(&crate::canon::json::parse(&text).unwrap()).unwrap();
    assert_eq!(back.state_hash(), w.state_hash());
    assert_eq!(back.relationships, w.relationships);
    let count = |w: &WorldState| w.pawns.iter().map(|(_, p)| p.memories.len()).sum::<usize>();
    assert_eq!(count(&back), count(w));
}

fn bare_pawn(n: u32) -> Pawn {
    let map = EntityId::new(Kind::Map, 1);
    Pawn::new(
        EntityId::new(Kind::Pawn, n),
        "P",
        Position {
            map,
            tile: Tile::new(1, 1),
        },
    )
}

fn task(action: &str, step: Step) -> Task {
    Task {
        reservation: 1,
        action: ActionId::new(action).unwrap(),
        steps: vec![step],
        current: 0,
        moving: false,
        interruptible: true,
    }
}

#[test]
fn only_free_residents_who_can_talk_are_available_and_company_is_wanted_when_lonely() {
    let params = data().conversation.clone().unwrap();
    let mut p = bare_pawn(1);
    assert!(available(&p), "a pawn with nothing to do is free");
    p.task = Some(task("socialise", Step::PerformUntil(99)));
    assert!(available(&p) && wants_company(&p, &params));
    p.task = Some(task("idle_at", Step::PerformUntil(99)));
    assert!(available(&p) && !wants_company(&p, &params));
    for busy in ["sleep", "eat", "rest"] {
        p.task = Some(task(busy, Step::PerformUntil(99)));
        assert!(!available(&p), "{busy}");
    }
    p.task = Some(task(
        "socialise",
        Step::MoveTo {
            goal: Tile::new(5, 5),
            within: 0,
        },
    ));
    assert!(!available(&p), "walking to the plaza is not chatting yet");
    p.task = None;
    p.capacities.talking = 100;
    assert!(!available(&p), "cannot talk");
    p.capacities = crate::capacity::Capacities::FULL;
    p.capacities.consciousness = 0;
    assert!(!available(&p), "unconscious");
    p.capacities = crate::capacity::Capacities::FULL;
    p.needs.insert("social".into(), 100);
    assert!(wants_company(&p, &params));
    p.needs.insert("social".into(), 900);
    assert!(!wants_company(&p, &params));
}

#[test]
fn the_tone_follows_the_data_rules_in_priority_order() {
    let params = data().conversation.clone().unwrap();
    let mut a = bare_pawn(1);
    let mut b = bare_pawn(2);
    let tone = |a: &Pawn, b: &Pawn, aff: i32| choose_tone(&params, aff, a, b).unwrap().id.clone();
    assert_eq!(tone(&a, &b, 0), "friendly");
    assert_eq!(tone(&a, &b, 450), "warm");
    assert_eq!(tone(&a, &b, -500), "hostile");
    a.mood = "upset".into();
    assert_eq!(tone(&a, &b, 0), "curt");
    assert_eq!(tone(&a, &b, -500), "hostile", "hostile outranks curt");
    a.mood = "neutral".into();
    b.mood = "cheerful".into();
    assert_eq!(tone(&a, &b, 0), "cheerful");
    assert_eq!(tone(&a, &b, 450), "warm", "warm outranks cheerful");
}

#[test]
fn a_talk_is_called_off_when_the_two_drift_apart_and_changes_nothing() {
    let mut sim = town("apart");
    // Run until someone is in a talk, then pull the partner far away.
    let mut found = None;
    for _ in 0..20_000 {
        sim.step().unwrap();
        found = sim
            .world()
            .pawns
            .iter()
            .find(|(_, p)| p.talk.as_ref().is_some_and(|t| t.leader))
            .map(|(id, p)| (id, p.talk.clone().unwrap()));
        if found.is_some() {
            break;
        }
    }
    let (leader, talk) = found.expect("someone talks within two days");
    let before = sim.world().relationships.get(leader, talk.partner).cloned();
    let mut snap = sim.snapshot();
    if let Some(p) = snap.world.pawns.get_mut(talk.partner) {
        p.position.tile = Tile::new(p.position.tile.x + 30, p.position.tile.y);
    }
    snap.world.rebuild_derived();
    let mut sim = Sim::restore(snap, crate::pipeline::Pipeline::new()).with_content(content());
    let events = run_collect(
        &mut sim,
        20,
        &["conversation.cancelled", "conversation.closed"],
    );
    assert!(
        events.iter().any(|(k, _)| k == "conversation.cancelled"),
        "{events:?}"
    );
    assert!(sim.world().pawns.get(leader).unwrap().talk.is_none());
    assert_eq!(
        sim.world().relationships.get(leader, talk.partner).cloned(),
        before,
        "a talk that was called off leaves the relationship alone"
    );
}

/// Answers `relationship.delta_modifier` with 0 and `memory.importance_modifier` with 2000.
struct Packs;

impl HookHost for Packs {
    fn ask(&mut self, point: &'static HookPoint, _w: &WorldState, _s: EntityId) -> Vec<i64> {
        match point.id {
            "relationship.delta_modifier" => vec![0],
            "memory.importance_modifier" => vec![5000],
            _ => Vec::new(),
        }
    }
}

#[test]
fn packs_can_scale_outcomes_within_the_clamps() {
    let mut plain = town("hooks");
    plain.run_ticks(14_400).unwrap();
    let mut hooked = town("hooks");
    hooked.set_hooks(Some(Box::new(Packs)));
    hooked.run_ticks(14_400).unwrap();
    // A modifier of 0 cancels every change (starting relationships keep their starting affinity).
    let moved = |s: &Sim| {
        s.world()
            .relationships
            .iter()
            .filter(|r| r.last_interaction_tick > 0)
            .map(|r| r.day_change.abs())
            .sum::<i32>()
    };
    assert!(moved(&plain) > 0);
    assert_eq!(moved(&hooked), 0);
    // The importance hook is clamped at 2000 permille (5000 asked), so a severity-1 talk at impact 0
    // (importance 10) becomes at most 20.
    let max_new = hooked
        .world()
        .pawns
        .iter()
        .flat_map(|(_, p)| p.memories.iter())
        .filter(|m| m.ty == "conversation")
        .map(|m| m.importance)
        .max()
        .unwrap();
    assert!(max_new <= 5 * 10 * 2, "{max_new}");
}

#[test]
fn fallback_dialogue_is_seeded_complete_and_alternates() {
    let d = data();
    let params = d.conversation.as_ref().unwrap();
    let (a, b) = (EntityId::new(Kind::Pawn, 1), EntityId::new(Kind::Pawn, 2));
    let seed = crate::rng::Seed::from_text("lines");
    let talk = |topic: &str, tone: &str, turns: u32| Talk {
        partner: b,
        topic: topic.into(),
        tone: tone.into(),
        started: 100,
        ends: 160,
        turns,
        leader: true,
    };
    let lines = dialogue(params, seed, a, &talk("food", "friendly", 4));
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines.iter().map(|l| l.speaker).collect::<Vec<_>>(),
        [a, b, a, b]
    );
    assert!(lines.iter().all(|l| l.key.starts_with("dialogue.food.")));
    assert_eq!(
        lines,
        dialogue(params, seed, a, &talk("food", "friendly", 4))
    );
    // A hostile talk uses hostile lines whatever the topic; an unknown topic falls back to the default set.
    let hostile = dialogue(params, seed, a, &talk("food", "hostile", 2));
    assert!(hostile
        .iter()
        .all(|l| l.key.starts_with("dialogue.hostile.")));
    let odd = dialogue(params, seed, a, &talk("nothing", "friendly", 2));
    assert!(odd.iter().all(|l| l.key.starts_with("dialogue.generic.")));
}
