use super::*;
use crate::action::{ActionDef, ActionRegistry, RegisterError, StepTemplate};
use crate::activity::{ActivitySystem, PlanSource};
use crate::map::{MapKind, Tile};
use crate::pipeline::{Event, Pipeline, SystemSlot};
use crate::schedule::{DutyTemplate, PlanInputs};
use crate::sim::Sim;
use crate::social::{PairKey, Relationship};
use pg_content::schema::{Field, FieldSchema, ParamSchema};
use pg_content::{
    load_pack, ActionId, ComponentRegistry, ContentSet, DirPack, Limits, Origin, PackId,
};
use std::sync::Arc;

fn data() -> Arc<GameData> {
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    let set = ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap();
    Arc::new(set.game().clone())
}

fn pawn_param() -> ParamSchema {
    ParamSchema::new().field(
        "with",
        Field::required(FieldSchema::EntityId {
            kind: "pawn".into(),
        }),
    )
}

/// A pack action that stays put for the reservation and then applies `effects`.
fn action(name: &str, pre: Vec<Precondition>, effects: Vec<Effect>) -> ActionDef {
    ActionDef {
        id: ActionId::new(&format!("test.{name}")).unwrap(),
        origin: Origin::Pack(PackId::new("test").unwrap()),
        params: pawn_param(),
        steps: vec![StepTemplate::PerformUntilSlotEnd],
        interruptible: true,
        ai_proposable: false,
        summary: "test".into(),
        restores: Vec::new(),
        preconditions: pre,
        effects,
    }
}

/// Only the first pawn has a duty; the others stand about.
struct Fixed(PlanInputs, EntityId);

impl PlanSource for Fixed {
    fn id(&self) -> &str {
        "test.fixed"
    }
    fn inputs(&self, _w: &WorldState, p: EntityId, _d: u64) -> PlanInputs {
        if p == self.1 {
            self.0.clone()
        } else {
            PlanInputs::default()
        }
    }
}

struct Rig {
    sim: Sim,
    a: EntityId,
    b: EntityId,
}

/// Two pawns; `a` has a two-slot duty running `name` against `b`. Needs: hunger 400, social 100.
fn rig(defs: Vec<ActionDef>, name: &str, friends: bool) -> Rig {
    let data = data();
    let mut w = WorldState::new("t", "seed");
    let map = w.create_map(MapKind::Overworld, 16, 16).unwrap();
    let a = w.spawn_pawn("A", map, Tile::new(1, 1)).unwrap();
    let b = w.spawn_pawn("B", map, Tile::new(4, 1)).unwrap();
    for id in [a, b] {
        let p = w.pawns.get_mut(id).unwrap();
        p.needs.insert("hunger".into(), 400);
        p.needs.insert("social".into(), 100);
    }
    if friends {
        let label = data.relationships.as_ref().unwrap().labels[1].id.clone();
        w.relationships.set(Relationship {
            key: PairKey::new(a, b).unwrap(),
            affinity: 0,
            label,
            last_interaction_tick: 0,
            day: 0,
            day_change: 0,
            last_topic: None,
            forgotten: 0,
        });
    }
    // The built-in registry is closed; rebuild an open one for the test packs.
    let mut registry = ActionRegistry::new();
    for d in ActionRegistry::builtin().iter() {
        registry.register(d.clone()).unwrap();
    }
    for d in defs {
        registry.register(d).unwrap();
    }
    registry.freeze();
    let duty = DutyTemplate {
        action: ActionId::new(name).unwrap(),
        params: Canon::map([("with", b.to_canon())]),
        start: 2,
        len: 2,
        min_len: 2,
        shift_earlier: 0,
        shift_later: 0,
    };
    let plans = Arc::new(Fixed(
        PlanInputs {
            duties: vec![duty],
            ..PlanInputs::default()
        },
        a,
    ));
    let registry = Arc::new(registry);
    let mut pipeline = Pipeline::with_actions(plans, Arc::clone(&registry));
    pipeline.set_builtin(
        SystemSlot::Activity,
        Box::new(ActivitySystem::new(registry, Some(data))),
    );
    Rig {
        sim: Sim::new(w, pipeline),
        a,
        b,
    }
}

fn run_to(sim: &mut Sim, tick: u64) -> Vec<Event> {
    let mut all = Vec::new();
    while sim.world().clock.tick() < tick {
        all.extend(sim.step().unwrap().events);
    }
    all
}

fn kinds<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

/// The duty (slots 2 to 4, 300 ticks a slot) ends at tick 1200.
const END: u64 = 1300;

fn need(sim: &Sim, id: EntityId, n: &str) -> i32 {
    *sim.world().pawns.get(id).unwrap().needs.get(n).unwrap()
}

fn failure(r: &Rig) -> ReasonCode {
    r.sim
        .world()
        .pawns
        .get(r.a)
        .unwrap()
        .last_failure
        .clone()
        .unwrap()
}

#[test]
fn effects_land_when_the_task_finishes_and_not_before() {
    let eat = action(
        "snack",
        vec![],
        vec![Effect::AddNeed {
            need: "hunger".into(),
            delta: 300,
        }],
    );
    let mut r = rig(vec![eat], "test.snack", true);
    let _ = run_to(&mut r.sim, 700);
    assert_eq!(need(&r.sim, r.a, "hunger"), 400, "nothing until the end");
    let events = run_to(&mut r.sim, END);
    assert_eq!(kinds(&events, "task.done").len(), 1);
    assert_eq!(need(&r.sim, r.a, "hunger"), 700);
    assert_eq!(need(&r.sim, r.b, "hunger"), 400, "only the actor changes");
}

#[test]
fn need_effects_are_held_to_the_range() {
    let big = action(
        "feast",
        vec![],
        vec![Effect::AddNeed {
            need: "hunger".into(),
            delta: 1000,
        }],
    );
    let mut r = rig(vec![big], "test.feast", true);
    run_to(&mut r.sim, END);
    assert_eq!(need(&r.sim, r.a, "hunger"), 1000);
}

#[test]
fn a_relationship_effect_goes_through_the_daily_cap_and_a_memory_effect_remembers() {
    let chat = action(
        "chat",
        vec![],
        vec![
            Effect::AdjustRelationship {
                with: "with".into(),
                delta: 900,
            },
            Effect::AddMemory {
                with: Some("with".into()),
                memory_type: "shared_event".into(),
                impact: 120,
                summary_key: "memory.start.neighbors".into(),
            },
        ],
    );
    let mut r = rig(vec![chat], "test.chat", true);
    let cap = data().relationships.as_ref().unwrap().daily_cap;
    let events = run_to(&mut r.sim, END);
    let rel = r.sim.world().relationships.get(r.a, r.b).unwrap();
    assert_eq!(rel.affinity, cap.min(900), "held to the pair's daily cap");
    let mems = &r.sim.world().pawns.get(r.a).unwrap().memories;
    assert_eq!(mems.len(), 1);
    assert_eq!(mems[0].ty, "shared_event");
    assert_eq!(mems[0].participants, vec![r.b]);
    assert_eq!(
        mems[0].summary_key.as_deref(),
        Some("memory.start.neighbors")
    );
    assert!(mems[0].impact > 0);
    assert_eq!(kinds(&events, "memory.created").len(), 1);
    assert!(r.sim.world().pawns.get(r.b).unwrap().memories.is_empty());
}

#[test]
fn one_bad_effect_applies_none_of_them() {
    // The first effect is fine, the second names a memory type that does not exist: nothing is applied.
    let bad = action(
        "bad",
        vec![],
        vec![
            Effect::AddNeed {
                need: "hunger".into(),
                delta: 300,
            },
            Effect::AddMemory {
                with: None,
                memory_type: "no_such_type".into(),
                impact: 10,
                summary_key: "memory.start.neighbors".into(),
            },
        ],
    );
    let mut r = rig(vec![bad], "test.bad", true);
    let events = run_to(&mut r.sim, END);
    assert_eq!(
        need(&r.sim, r.a, "hunger"),
        400,
        "the need change was not applied"
    );
    assert_eq!(kinds(&events, "task.failed").len(), 1);
    assert!(kinds(&events, "task.done").is_empty());
    let why = failure(&r);
    assert_eq!(why.code, "precondition_failed");
    assert!(why.explain().contains("no_such_type"), "{}", why.explain());
}

#[test]
fn effects_that_name_missing_things_are_refused_with_a_reason() {
    // No relationship between the pair: the effect cannot apply.
    let chat = action(
        "chat",
        vec![],
        vec![Effect::AdjustRelationship {
            with: "with".into(),
            delta: 50,
        }],
    );
    let mut r = rig(vec![chat], "test.chat", false);
    let events = run_to(&mut r.sim, END);
    assert_eq!(kinds(&events, "task.failed").len(), 1);
    let why = failure(&r);
    assert!(
        why.explain().contains("do not know each other"),
        "{}",
        why.explain()
    );
}

#[test]
fn preconditions_gate_the_start_of_a_task() {
    // Needs hunger at most 300; the pawn is at 400, so the task never starts.
    let picky = action(
        "picky",
        vec![Precondition::NeedAtMost {
            need: "hunger".into(),
            max: 300,
        }],
        vec![Effect::AddNeed {
            need: "hunger".into(),
            delta: 100,
        }],
    );
    let mut r = rig(vec![picky], "test.picky", true);
    let events = run_to(&mut r.sim, END);
    assert!(kinds(&events, "task.started").is_empty());
    // The replan offers the rest of the duty again, and it is refused again: the reason is the same.
    assert!(!kinds(&events, "task.failed").is_empty());
    assert_eq!(need(&r.sim, r.a, "hunger"), 400);
    let why = failure(&r);
    assert_eq!(why.code, "precondition_failed");
    assert!(why.explain().contains("hunger is 400"), "{}", why.explain());
}

#[test]
fn a_precondition_that_holds_lets_the_task_run() {
    let ok = action(
        "ok",
        vec![
            Precondition::NeedAtLeast {
                need: "hunger".into(),
                min: 300,
            },
            Precondition::Capacity {
                id: "talking".into(),
                min: 500,
            },
            Precondition::Company { radius: 5 },
        ],
        vec![Effect::AddNeed {
            need: "social".into(),
            delta: 200,
        }],
    );
    let mut r = rig(vec![ok], "test.ok", true);
    let events = run_to(&mut r.sim, END);
    assert_eq!(kinds(&events, "task.done").len(), 1);
    assert_eq!(need(&r.sim, r.a, "social"), 300);
}

#[test]
fn company_is_judged_by_distance() {
    let mut w = WorldState::new("t", "s");
    let map = w.create_map(MapKind::Overworld, 16, 16).unwrap();
    let a = w.spawn_pawn("A", map, Tile::new(1, 1)).unwrap();
    let b = w.spawn_pawn("B", map, Tile::new(9, 1)).unwrap();
    let near = Precondition::Company { radius: 5 };
    assert!(near.check(&w, a).is_err());
    w.pawns.get_mut(b).unwrap().position.tile = Tile::new(4, 1);
    assert!(near.check(&w, a).is_ok());
    let nonsense = Precondition::Capacity {
        id: "nonsense".into(),
        min: 1,
    };
    assert!(nonsense.check(&w, a).is_err());
}

#[test]
fn registration_rejects_out_of_range_and_unknown_parameter_references() {
    let mut r = ActionRegistry::new();
    let too_big = action(
        "huge",
        vec![],
        vec![Effect::AddNeed {
            need: "hunger".into(),
            delta: 5000,
        }],
    );
    assert!(matches!(
        r.register(too_big),
        Err(RegisterError::BadEffect { .. })
    ));
    let mut no_param = action(
        "lost",
        vec![],
        vec![Effect::AdjustRelationship {
            with: "ghost".into(),
            delta: 5,
        }],
    );
    assert!(matches!(
        r.register(no_param.clone()),
        Err(RegisterError::BadEffect { .. })
    ));
    // A parameter that is not a pawn id is no better.
    no_param.params = ParamSchema::new().field("ghost", Field::required(FieldSchema::Tile));
    assert!(matches!(
        r.register(no_param),
        Err(RegisterError::BadEffect { .. })
    ));
    let wide = action("wide", vec![Precondition::Company { radius: 99 }], vec![]);
    assert!(matches!(
        r.register(wide),
        Err(RegisterError::BadEffect { .. })
    ));
    let one = Effect::AddNeed {
        need: "hunger".into(),
        delta: 1,
    };
    let many = action("many", vec![], vec![one; MAX_PER_ACTION + 1]);
    assert!(matches!(
        r.register(many),
        Err(RegisterError::BadEffect { .. })
    ));
}

#[test]
fn the_same_run_gives_the_same_hash() {
    let build = || {
        let chat = action(
            "chat",
            vec![],
            vec![
                Effect::AdjustRelationship {
                    with: "with".into(),
                    delta: 90,
                },
                Effect::AddMemory {
                    with: Some("with".into()),
                    memory_type: "shared_event".into(),
                    impact: 50,
                    summary_key: "memory.start.neighbors".into(),
                },
            ],
        );
        let mut r = rig(vec![chat], "test.chat", true);
        run_to(&mut r.sim, END);
        r.sim.world().state_hash()
    };
    assert_eq!(build(), build());
}
