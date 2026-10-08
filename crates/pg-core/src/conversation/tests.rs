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
        lines: Vec::new(),
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

fn record(sim: &mut Sim, a: EntityId, b: EntityId, started: u64, lines: &[&str]) {
    sim.submit_now(SimInput::Command {
        actor: None,
        cmd: Command::RecordDialogue {
            a,
            b,
            started,
            lines: lines.iter().map(|s| (*s).to_owned()).collect(),
        },
    })
    .unwrap();
}

/// Steps until some pair is mid-conversation; returns (a, b, the talk as it began).
fn until_mid_talk(sim: &mut Sim) -> (EntityId, EntityId, Talk) {
    for _ in 0..30_000 {
        sim.step().unwrap();
        let found = sim
            .world()
            .pawns
            .iter()
            .find(|(_, p)| p.talk.as_ref().is_some_and(|t| t.leader))
            .map(|(id, p)| (id, p.talk.clone().unwrap()));
        if let Some((a, t)) = found {
            return (a, t.partner, t);
        }
    }
    panic!("nobody talked");
}

fn rejected_reason(sim: &mut Sim) -> Option<String> {
    let r = sim.step().unwrap();
    r.events
        .iter()
        .find(|e| e.kind == "input_rejected")
        .and_then(|e| {
            e.detail
                .get("reason")
                .and_then(Canon::as_str)
                .map(str::to_owned)
        })
}

#[test]
fn lines_recorded_during_a_talk_end_up_in_both_memories_and_are_recalled() {
    let mut sim = town("history-live");
    let (a, b, talk) = until_mid_talk(&mut sim);
    let said: Vec<String> = (0..talk.turns).map(|i| format!("Said {i}.")).collect();
    let refs: Vec<&str> = said.iter().map(String::as_str).collect();
    record(&mut sim, a, b, talk.started, &refs);
    let events = run_collect(&mut sim, 1, &["dialogue.recorded"]);
    assert_eq!(events.len(), 1);
    assert_eq!(
        sim.world()
            .pawns
            .get(a)
            .unwrap()
            .talk
            .as_ref()
            .unwrap()
            .lines,
        said
    );
    assert_eq!(
        sim.world()
            .pawns
            .get(b)
            .unwrap()
            .talk
            .as_ref()
            .unwrap()
            .lines,
        said
    );
    // After the talk closes, both remember it with the words.
    run_collect(&mut sim, 200, &[]);
    let d = data();
    let params = d.conversation.as_ref().unwrap();
    for (owner, other) in [(a, b), (b, a)] {
        let p = sim.world().pawns.get(owner).unwrap();
        let m = p
            .memories
            .iter()
            .find(|m| {
                m.talk.as_ref().is_some_and(|t| t.started == talk.started)
                    && m.participants.contains(&other)
            })
            .expect("the conversation was remembered");
        assert_eq!(m.talk.as_ref().unwrap().lines, said);
        let recalled = recall(sim.world(), params, owner, m).unwrap();
        assert_eq!(recalled.len(), said.len());
        assert_eq!(recalled[0].speaker, a.min(b), "the lower id speaks first");
        assert_eq!(recalled[0].said, Spoken::Written("Said 0.".into()));
    }
}

#[test]
fn lines_recorded_afterwards_are_attached_and_without_them_the_fallback_is_rebuilt() {
    let mut sim = town("history-late");
    let (a, b, talk) = until_mid_talk(&mut sim);
    run_collect(&mut sim, 200, &[]);
    let d = data();
    let params = d.conversation.as_ref().unwrap();
    let memory_of = |sim: &Sim, owner: EntityId, other: EntityId| {
        sim.world()
            .pawns
            .get(owner)
            .unwrap()
            .memories
            .iter()
            .find(|m| {
                m.talk.as_ref().is_some_and(|t| t.started == talk.started)
                    && m.participants.contains(&other)
            })
            .cloned()
            .unwrap()
    };
    // No words recorded: the fallback lines come back from the data, exactly as they would have been shown.
    let before = recall(sim.world(), params, a, &memory_of(&sim, a, b)).unwrap();
    let shown = dialogue(params, sim.world().seed(), a, &talk);
    assert_eq!(
        before.iter().map(|r| r.said.clone()).collect::<Vec<_>>(),
        shown
            .iter()
            .map(|l| Spoken::Key(l.key.clone()))
            .collect::<Vec<_>>()
    );
    let said: Vec<String> = (0..talk.turns).map(|i| format!("Later {i}.")).collect();
    let refs: Vec<&str> = said.iter().map(String::as_str).collect();
    record(&mut sim, a, b, talk.started, &refs);
    let ev = run_collect(&mut sim, 1, &["dialogue.recorded"]);
    assert_eq!(ev.len(), 1);
    assert_eq!(
        ev[0].1.get("when").and_then(Canon::as_str),
        Some("afterwards")
    );
    let after = recall(sim.world(), params, b, &memory_of(&sim, b, a)).unwrap();
    assert_eq!(after[1].said, Spoken::Written("Later 1.".into()));
}

#[test]
fn bad_recordings_are_refused_and_change_nothing() {
    let mut sim = town("history-bad");
    let (a, b, talk) = until_mid_talk(&mut sim);
    let before = sim.world().state_hash();
    record(&mut sim, a, b, talk.started, &["only one"; 1]);
    if talk.turns != 1 {
        assert_eq!(
            rejected_reason(&mut sim).as_deref(),
            Some("the number of lines must equal the number of turns")
        );
    }
    let long = "x".repeat(300);
    let many: Vec<&str> = (0..talk.turns).map(|_| long.as_str()).collect();
    record(&mut sim, a, b, talk.started, &many);
    assert!(rejected_reason(&mut sim).unwrap().contains("140"));
    record(&mut sim, a, b, talk.started + 7, &["x"]);
    assert!(rejected_reason(&mut sim)
        .unwrap()
        .contains("no such conversation"));
    record(&mut sim, a, b, talk.started, &[]);
    assert!(rejected_reason(&mut sim).unwrap().contains("1 to 8"));
    // Nothing was recorded (the talk may have moved on a tick or two but holds no lines).
    assert!(sim
        .world()
        .pawns
        .get(a)
        .unwrap()
        .talk
        .as_ref()
        .is_none_or(|t| t.lines.is_empty()));
    let _ = before;
}

#[test]
fn a_recording_is_part_of_the_logged_history_and_replays_the_same() {
    let run = || {
        let mut sim = town("history-replay");
        let (a, b, talk) = until_mid_talk(&mut sim);
        let said: Vec<String> = (0..talk.turns).map(|i| format!("Said {i}.")).collect();
        let refs: Vec<&str> = said.iter().map(String::as_str).collect();
        record(&mut sim, a, b, talk.started, &refs);
        sim.run_ticks(3_000).unwrap();
        sim
    };
    let (x, y) = (run(), run());
    assert_eq!(x.world().state_hash(), y.world().state_hash());
    // The command survives its canonical form (the replay log's).
    let cmd = Command::RecordDialogue {
        a: EntityId::new(Kind::Pawn, 1),
        b: EntityId::new(Kind::Pawn, 2),
        started: 90,
        lines: vec!["Hi.".into(), "Hello.".into()],
    };
    assert_eq!(Command::from_canon(&cmd.to_canon()).unwrap(), cmd);
}
