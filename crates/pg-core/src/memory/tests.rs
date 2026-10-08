use super::*;
use crate::id::Kind;
use crate::map::{MapKind, Tile};
use crate::social::{PairKey, Relationship};

fn params(max: u32) -> MemoryParams {
    MemoryParams {
        persist_threshold: 300,
        max_memories: max,
        decay_k: 60,
        types: Vec::new(),
    }
}

fn world(pawns: usize) -> (WorldState, Vec<EntityId>) {
    let mut w = WorldState::new("Mem", "mem-seed");
    let m = w.create_map(MapKind::Overworld, 12, 12).unwrap();
    let ids = (0..pawns)
        .map(|i| {
            w.spawn_pawn(&format!("P{i}"), m, Tile::new(1 + i as i32, 1))
                .unwrap()
        })
        .collect();
    (w, ids)
}

fn mem(w: &mut WorldState, severity: u8, impact: i32, tick: u64, with: EntityId) -> Memory {
    let id = w.id_counters.allocate(Kind::Memory).unwrap();
    let mut m = Memory::new(id, "conversation", tick, vec![with], severity, impact, 60);
    m.topic = Some("small_talk".into());
    m
}

fn relationship(w: &mut WorldState, a: EntityId, b: EntityId) {
    w.relationships.set(Relationship {
        key: PairKey::new(a, b).unwrap(),
        affinity: 100,
        label: "acquaintance".into(),
        last_interaction_tick: 0,
        day: 0,
        day_change: 0,
        last_topic: None,
        forgotten: 0,
    });
}

#[test]
fn a_pawn_never_keeps_more_than_the_bound_and_forgets_the_least_valuable_first() {
    let (mut w, ids) = world(2);
    let (me, other) = (ids[0], ids[1]);
    relationship(&mut w, me, other);
    let p = params(3);
    // Importance 20, 30, 40: the first is the least valuable.
    for (i, sev_impact) in [(1u8, 10), (1, 20), (1, 30)].into_iter().enumerate() {
        let m = mem(&mut w, sev_impact.0, sev_impact.1, 100 + i as u64, other);
        assert!(remember(&mut w, me, m, &p).is_empty());
    }
    assert_eq!(w.pawns.get(me).unwrap().memories.len(), 3);
    let weakest = w.pawns.get(me).unwrap().memories[0].id;
    let m = mem(&mut w, 1, 25, 200, other);
    let gone = remember(&mut w, me, m, &p);
    assert_eq!(
        gone,
        vec![Forgotten {
            pawn: me,
            memory: weakest,
            why: Why::Crowded
        }]
    );
    assert_eq!(w.pawns.get(me).unwrap().memories.len(), 3);
    // What was forgotten is summarised on the relationship.
    let rel = w.relationships.get(me, other).unwrap();
    assert_eq!(
        (rel.forgotten, rel.last_topic.as_deref()),
        (1, Some("small_talk"))
    );
}

#[test]
fn ties_in_value_go_to_the_older_memory_then_the_lower_id() {
    let (mut w, ids) = world(2);
    let p = params(2);
    let a = mem(&mut w, 1, 10, 50, ids[1]);
    let b = mem(&mut w, 1, 10, 40, ids[1]);
    let (a_id, b_id) = (a.id, b.id);
    remember(&mut w, ids[0], a, &p);
    remember(&mut w, ids[0], b, &p);
    let c = mem(&mut w, 1, 10, 60, ids[1]);
    let gone = remember(&mut w, ids[0], c, &p);
    assert_eq!(gone[0].memory, b_id, "tick 40 is older than tick 50");
    assert_ne!(gone[0].memory, a_id);
}

#[test]
fn persistent_memories_survive_the_bound_and_the_passing_days() {
    let (mut w, ids) = world(2);
    let p = params(2);
    // Severity 5 with impact 100 has importance 550: persistent.
    let big = mem(&mut w, 5, 100, 10, ids[1]);
    assert!(is_persistent(&big, &p));
    let big_id = big.id;
    remember(&mut w, ids[0], big, &p);
    for t in 0..6 {
        let m = mem(&mut w, 1, 10, 20 + t, ids[1]);
        remember(&mut w, ids[0], m, &p);
    }
    let kept = &w.pawns.get(ids[0]).unwrap().memories;
    assert!(kept.iter().any(|m| m.id == big_id));
    assert_eq!(kept.len(), 2);
    for _ in 0..400 {
        age_memories(&mut w, ids[0], &p);
    }
    let kept = &w.pawns.get(ids[0]).unwrap().memories;
    assert_eq!(kept.len(), 1, "only the persistent one is left");
    assert_eq!(kept[0].id, big_id);
    assert_eq!(kept[0].retention, 1000, "persistent memories do not fade");
}

#[test]
fn low_severity_memories_fade_faster_and_are_forgotten_when_retention_runs_out() {
    let (mut w, ids) = world(2);
    let p = params(50);
    let minor = mem(&mut w, 1, 0, 10, ids[1]); // decay 60 a day
    let major = mem(&mut w, 3, 0, 10, ids[1]); // decay 20 a day, importance 30: ordinary
    assert_eq!((minor.decay_per_day, major.decay_per_day), (60, 20));
    let (minor_id, major_id) = (minor.id, major.id);
    remember(&mut w, ids[0], minor, &p);
    remember(&mut w, ids[0], major, &p);
    let mut first_gone = None;
    for day in 1..=60 {
        for g in age_memories(&mut w, ids[0], &p) {
            assert_eq!(g.why, Why::Faded);
            first_gone.get_or_insert((day, g.memory));
        }
    }
    assert_eq!(
        first_gone,
        Some((17, minor_id)),
        "1000 / 60 rounds up to 17 days"
    );
    assert!(w.pawns.get(ids[0]).unwrap().memories.is_empty());
    let _ = major_id;
}

#[test]
fn relevant_memories_are_ranked_with_reasons_and_ties_are_stable() {
    let (mut w, ids) = world(3);
    let (friend, stranger) = (ids[1], ids[2]);
    let now = 50 * 1440 * 10;
    let old_big = mem(&mut w, 4, 60, 10, stranger);
    let recent_friend = mem(&mut w, 1, 10, now - 100, friend);
    let mut topical = mem(&mut w, 1, 10, now - 100_000, stranger);
    topical.topic = Some("food".into());
    let list = vec![old_big.clone(), recent_friend.clone(), topical.clone()];
    let ctx = Context {
        now,
        with: Some(friend),
        topic: Some("food"),
    };
    let ranked = select_relevant_memories(&list, &ctx, 3);
    assert_eq!(ranked.len(), 3);
    assert_eq!(
        ranked[0].memory.id, recent_friend.id,
        "recent and about the friend outweighs one old, important memory"
    );
    let big_row = ranked.iter().find(|r| r.memory.id == old_big.id).unwrap();
    assert!(big_row.reasons.contains(&Reason::Important(280)));
    assert!(!big_row.reasons.contains(&Reason::Recent));
    let friend_row = ranked
        .iter()
        .find(|r| r.memory.id == recent_friend.id)
        .unwrap();
    assert!(friend_row.reasons.contains(&Reason::Participant));
    assert!(friend_row.reasons.contains(&Reason::Recent));
    let topic_row = ranked.iter().find(|r| r.memory.id == topical.id).unwrap();
    assert!(topic_row.reasons.contains(&Reason::Topic));
    assert_eq!(select_relevant_memories(&list, &ctx, 1).len(), 1);
    assert!(select_relevant_memories(&[], &ctx, 5).is_empty());
    // The same input gives the same order.
    let again = select_relevant_memories(&list, &ctx, 3);
    assert_eq!(
        ranked.iter().map(|r| r.memory.id).collect::<Vec<_>>(),
        again.iter().map(|r| r.memory.id).collect::<Vec<_>>()
    );
}
