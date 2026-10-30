use super::*;
use crate::id::Kind;
use crate::input::{Command, SimInput};
use crate::map::MapKind;
use crate::pipeline::{Event, Pipeline};
use crate::schedule::{Chore, DutyTemplate, PlanInputs};
use crate::sim::Sim;
use pg_content::ActionId;

const SLOT: u64 = 300; // ticks per 30-minute slot

fn t(x: i32, y: i32) -> Tile {
    Tile::new(x, y)
}

fn act(name: &str) -> ActionId {
    ActionId::new(name).unwrap()
}

fn at(tile: Tile) -> Canon {
    Canon::map([("at", tile.to_canon())])
}

/// Every pawn gets the same inputs; tests that need different plans use `PerPawn`.
struct Fixed(PlanInputs);

impl PlanSource for Fixed {
    fn id(&self) -> &str {
        "test.fixed"
    }
    fn inputs(&self, _w: &WorldState, _p: EntityId, _d: u64) -> PlanInputs {
        self.0.clone()
    }
}

struct PerPawn(Vec<(EntityId, PlanInputs)>);

impl PlanSource for PerPawn {
    fn id(&self) -> &str {
        "test.per_pawn"
    }
    fn inputs(&self, _w: &WorldState, p: EntityId, _d: u64) -> PlanInputs {
        self.0
            .iter()
            .find(|(id, _)| *id == p)
            .map(|(_, i)| i.clone())
            .unwrap_or_default()
    }
}

fn duty(action: &str, params: Canon, start: u32, len: u32) -> DutyTemplate {
    DutyTemplate {
        action: act(action),
        params,
        start,
        len,
        min_len: len,
        shift_earlier: 0,
        shift_later: 0,
    }
}

fn pipe(source: &Arc<dyn PlanSource>) -> Pipeline {
    Pipeline::with_plan_source(Arc::clone(source))
}

/// A sim on a 16x16 open map with `pawns` placed at the given tiles.
fn world(source: Arc<dyn PlanSource>, pawns: &[(i32, i32)]) -> (Sim, EntityId, Vec<EntityId>) {
    let mut w = WorldState::new("t", "seed");
    let map = w.create_map(MapKind::Overworld, 16, 16).unwrap();
    let ids = pawns
        .iter()
        .enumerate()
        .map(|(i, (x, y))| w.spawn_pawn(&format!("P{i}"), map, t(*x, *y)).unwrap())
        .collect();
    let sim = Sim::new(w, pipe(&source));
    (sim, map, ids)
}

fn run(sim: &mut Sim, ticks: u64) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        all.extend(sim.step().unwrap().events);
    }
    all
}

/// Steps until the world is at `tick` (collecting events).
fn run_to(sim: &mut Sim, tick: u64) -> Vec<Event> {
    let now = sim.world().clock.tick();
    run(sim, tick.saturating_sub(now))
}

fn pawn(sim: &Sim, id: EntityId) -> &crate::pawn::Pawn {
    sim.world().pawns.get(id).unwrap()
}

fn kinds<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.kind == kind).collect()
}

fn command(sim: &mut Sim, cmd: Command) {
    sim.submit_now(SimInput::Command { actor: None, cmd })
        .unwrap();
}

fn propose(
    sim: &mut Sim,
    a: EntityId,
    b: EntityId,
    start: u32,
    len: u32,
    place: Tile,
    expires_in: u32,
) {
    command(
        sim,
        Command::DevPropose {
            proposer: a,
            invitee: b,
            start,
            len,
            at: place,
            expires_in,
            reschedulable: false,
        },
    );
}

fn only_commitment(sim: &Sim) -> &Commitment {
    sim.world().commitments.iter().next().unwrap().1
}

#[test]
fn a_duty_is_walked_to_performed_and_finished() {
    let inputs = PlanInputs {
        duties: vec![duty("idle_at", at(t(6, 0)), 2, 2)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    let p = ids[0];
    // Before the slot starts the pawn is free and still.
    let events = run_to(&mut sim, 599);
    assert_eq!(pawn(&sim, p).intent, Intent::Free);
    assert_eq!(pawn(&sim, p).position.tile, t(0, 0));
    assert!(kinds(&events, "task.started").is_empty());
    // The slot begins: the activator sets the intent, the planner makes a task, the pawn walks.
    let events = run_to(&mut sim, 640);
    assert!(matches!(pawn(&sim, p).intent, Intent::Reservation(_)));
    assert_eq!(kinds(&events, "task.started").len(), 1);
    assert_eq!(
        pawn(&sim, p).position.tile,
        t(6, 0),
        "12 ticks of walking is enough for 6 tiles"
    );
    // It stays until the reservation ends, then the task completes exactly once.
    let events = run_to(&mut sim, 1250);
    assert_eq!(kinds(&events, "task.done").len(), 1);
    assert_eq!(pawn(&sim, p).intent, Intent::Free);
    assert_eq!(pawn(&sim, p).position.tile, t(6, 0));
    assert!(pawn(&sim, p).last_failure.is_none());
}

#[test]
fn a_blocked_destination_fails_the_task_with_a_reason_and_triggers_a_replan() {
    let inputs = PlanInputs {
        duties: vec![duty("idle_at", at(t(6, 0)), 2, 2)],
        ..PlanInputs::default()
    };
    let (mut sim, map, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    command(
        &mut sim,
        Command::DevSetBlocked {
            map,
            at: t(6, 0),
            blocked: true,
        },
    );
    let events = run(&mut sim, 2 * SLOT + 5);
    let failed = kinds(&events, "task.failed");
    assert_eq!(failed.len(), 1);
    let p = pawn(&sim, ids[0]);
    assert_eq!(p.last_failure.as_ref().unwrap().code, "blocked_destination");
    assert_eq!(p.intent, Intent::Free);
    assert!(p.task.is_none());
    assert!(p.replan.is_some(), "the next slot boundary replans");
    // The replan happens at the next boundary and is explained on the schedule.
    let events = run(&mut sim, SLOT);
    assert_eq!(kinds(&events, "schedule.replanned").len(), 1);
    let s = pawn(&sim, ids[0]).schedule.as_ref().unwrap();
    assert!(
        s.notes.iter().any(|n| n.code == "replanned"),
        "{:?}",
        s.notes
    );
    s.check_invariants().unwrap();
}

#[test]
fn an_unreachable_destination_reports_why() {
    // A pocket at (10,10) sealed by blocked tiles.
    let inputs = PlanInputs {
        duties: vec![duty("idle_at", at(t(10, 10)), 2, 2)],
        ..PlanInputs::default()
    };
    let (mut sim, map, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    for (x, y) in [(9, 10), (11, 10), (10, 9), (10, 11)] {
        command(
            &mut sim,
            Command::DevSetBlocked {
                map,
                at: t(x, y),
                blocked: true,
            },
        );
    }
    run(&mut sim, 2 * SLOT + 60);
    let reason = pawn(&sim, ids[0]).last_failure.clone().unwrap();
    assert_eq!(
        reason.code,
        "unreachable_or_too_far",
        "{}",
        reason.explain()
    );
}

#[test]
fn an_unknown_action_or_bad_parameters_fail_cleanly() {
    let inputs = PlanInputs {
        duties: vec![
            duty("fly", Canon::Null, 2, 1),
            duty("move_to", Canon::map([("to", Canon::str("nowhere"))]), 4, 1),
        ],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    let events = run(&mut sim, 2 * SLOT + 5);
    assert_eq!(
        pawn(&sim, ids[0]).last_failure.as_ref().unwrap().code,
        "unknown_action"
    );
    let events2 = run(&mut sim, 2 * SLOT);
    assert_eq!(
        pawn(&sim, ids[0]).last_failure.as_ref().unwrap().code,
        "bad_params"
    );
    assert_eq!(
        kinds(&events, "task.failed").len() + kinds(&events2, "task.failed").len(),
        2
    );
}

#[test]
fn a_pawn_that_appears_mid_slot_is_planned_at_the_next_boundary() {
    let (mut sim, map, _) = world(Arc::new(Fixed(PlanInputs::default())), &[]);
    run(&mut sim, SLOT + 50);
    command(
        &mut sim,
        Command::DevSpawnPawn {
            map,
            at: Some(t(1, 1)),
            name: "Late".into(),
        },
    );
    run(&mut sim, 1);
    let id = sim.world().pawns.iter().next().unwrap().0;
    assert!(pawn(&sim, id).schedule.is_none(), "not planned mid-slot");
    run(&mut sim, SLOT);
    let s = pawn(&sim, id).schedule.as_ref().unwrap();
    assert_eq!(s.day, 0);
    // Slots before the boundary the pawn was planned at are not offered.
    assert!(s.reservations().all(|r| r.start >= 2));
}

#[test]
fn a_new_day_gets_a_fresh_plan() {
    let inputs = PlanInputs {
        chores: vec![Chore {
            id: 1,
            action: act("move_to"),
            params: Canon::map([("to", t(2, 2).to_canon())]),
            len: 1,
            urgency: 1,
            deadline_slot: None,
            created_tick: 0,
        }],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    run(&mut sim, crate::time::TICKS_PER_DAY + 10);
    let s = pawn(&sim, ids[0]).schedule.as_ref().unwrap();
    assert_eq!(s.day, 1);
    assert_eq!(s.reservations().count(), 1);
}

// ---- commitments ------------------------------------------------------------------------------------

fn two_free_pawns() -> (Sim, Vec<EntityId>) {
    let (sim, _, ids) = world(Arc::new(Fixed(PlanInputs::default())), &[(0, 0), (9, 9)]);
    (sim, ids)
}

#[test]
fn both_pawns_reserve_walk_meet_and_the_commitment_completes() {
    let (mut sim, ids) = two_free_pawns();
    run_to(&mut sim, 305); // both planned
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    let events = run_to(&mut sim, 605); // the boundary at 600: the invitee answers
    assert_eq!(
        only_commitment(&sim).state,
        CommitState::Accepted,
        "{:?}",
        only_commitment(&sim).reason
    );
    assert_eq!(kinds(&events, "commitment.accepted").len(), 1);
    for p in &ids {
        let s = pawn(&sim, *p).schedule.as_ref().unwrap();
        let held: Vec<_> = s
            .reservations()
            .filter(|r| r.commitment.is_some())
            .collect();
        assert_eq!(held.len(), 1);
        assert_eq!((held[0].start, held[0].len), (4, 2));
        s.check_invariants().unwrap();
    }
    let events = run_to(&mut sim, 1210); // slot 4 begins at 1200
    assert_eq!(only_commitment(&sim).state, CommitState::Active);
    assert_eq!(kinds(&events, "commitment.active").len(), 1);
    run_to(&mut sim, 1300);
    let (a, b) = (
        pawn(&sim, ids[0]).position.tile,
        pawn(&sim, ids[1]).position.tile,
    );
    assert_ne!(a, b, "two pawns never share a tile");
    for tile in [a, b] {
        assert!(
            tile.manhattan(t(5, 5)) <= crate::action::MEET_RADIUS,
            "{tile:?}"
        );
    }
    let events = run_to(&mut sim, 1810); // the commitment ends at slot 6 = tick 1800
    assert_eq!(only_commitment(&sim).state, CommitState::Completed);
    assert_eq!(kinds(&events, "commitment.completed").len(), 1);
}

#[test]
fn acceptance_is_atomic_when_either_side_cannot_keep_the_slot() {
    // The invitee has a duty at slots 4-5; the proposer is free.
    let busy = PlanInputs {
        duties: vec![duty("idle_at", at(t(9, 9)), 4, 2)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(
        Arc::new(PerPawn(vec![(EntityId::new(Kind::Pawn, 2), busy)])),
        &[(0, 0), (9, 9)],
    );
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    run(&mut sim, SLOT);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Declined);
    assert!(
        c.reason
            .as_ref()
            .unwrap()
            .explain()
            .contains("duties come first"),
        "{}",
        c.reason.as_ref().unwrap().explain()
    );
    // Neither schedule holds the commitment.
    for p in &ids {
        let s = pawn(&sim, *p).schedule.as_ref().unwrap();
        assert!(s.reservations().all(|r| r.commitment.is_none()));
    }
    // And the proposer's slots were not touched by the failed attempt.
    assert!(pawn(&sim, ids[0])
        .schedule
        .as_ref()
        .unwrap()
        .is_free_run(4, 2));
}

#[test]
fn a_commitment_displaces_a_chore_which_moves_later() {
    let chore = |id, urgency| Chore {
        id,
        action: act("move_to"),
        params: Canon::map([("to", t(3, 3).to_canon())]),
        len: 2,
        urgency,
        deadline_slot: None,
        created_tick: 0,
    };
    let inputs = PlanInputs {
        chores: vec![chore(1, 4), chore(2, 3)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0), (9, 9)]);
    run_to(&mut sim, 305); // planned at the boundary of slot 1: chores at 1..3 and 3..5
    let second = pawn(&sim, ids[0]).schedule.as_ref().unwrap().by_start()[1].start;
    assert_eq!(second, 3);
    propose(&mut sim, ids[0], ids[1], 3, 2, t(5, 5), 100_000);
    run_to(&mut sim, 605); // answered at the boundary of slot 2
    assert_eq!(
        only_commitment(&sim).state,
        CommitState::Accepted,
        "{:?}",
        only_commitment(&sim).reason
    );
    let s = pawn(&sim, ids[0]).schedule.as_ref().unwrap();
    let moved = s
        .reservations()
        .find(|r| r.start >= 5 && r.commitment.is_none())
        .unwrap();
    assert_eq!(moved.reason.code, "moved_later");
    s.check_invariants().unwrap();
}

#[test]
fn unanswered_and_stale_proposals_expire() {
    let (mut sim, ids) = two_free_pawns();
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 1);
    let events = run(&mut sim, SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Expired);
    assert_eq!(kinds(&events, "commitment.expired").len(), 1);
    assert!(pawn(&sim, ids[0])
        .schedule
        .as_ref()
        .unwrap()
        .reservations()
        .all(|r| r.commitment.is_none()));
}

#[test]
fn a_slot_that_has_already_started_is_declined() {
    let (mut sim, ids) = two_free_pawns();
    run(&mut sim, 3 * SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 1, 1, t(5, 5), 100_000);
    run(&mut sim, SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Declined);
    assert!(only_commitment(&sim)
        .reason
        .as_ref()
        .unwrap()
        .explain()
        .contains("already started"));
}

#[test]
fn cancelling_frees_both_schedules_and_interrupts_a_running_task() {
    let (mut sim, ids) = two_free_pawns();
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    run_to(&mut sim, 605);
    run_to(&mut sim, 1300); // slot 4 is under way, tasks are running
    assert!(pawn(&sim, ids[0]).task.is_some());
    let id = only_commitment(&sim).id;
    command(&mut sim, Command::DevCancelCommitment { commitment: id });
    let events = run(&mut sim, 3);
    assert_eq!(only_commitment(&sim).state, CommitState::Cancelled);
    assert_eq!(kinds(&events, "commitment.cancelled").len(), 1);
    for p in &ids {
        assert!(pawn(&sim, *p)
            .schedule
            .as_ref()
            .unwrap()
            .reservations()
            .all(|r| r.commitment.is_none()));
        assert!(
            pawn(&sim, *p).task.is_none(),
            "the running task was interrupted"
        );
    }
    assert_eq!(kinds(&events, "task.failed").len(), 2);
    assert_eq!(
        pawn(&sim, ids[0]).last_failure.as_ref().unwrap().code,
        "interrupted"
    );
}

#[test]
fn a_no_show_fails_the_commitment_with_a_reason() {
    // The place is walled in after acceptance, so neither pawn can arrive.
    let (mut sim, map, ids) = world(Arc::new(Fixed(PlanInputs::default())), &[(0, 0), (9, 9)]);
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 1, t(12, 12), 100_000);
    run(&mut sim, SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Accepted);
    for (x, y) in [(11, 12), (13, 12), (12, 11), (12, 13)] {
        command(
            &mut sim,
            Command::DevSetBlocked {
                map,
                at: t(x, y),
                blocked: true,
            },
        );
    }
    run(&mut sim, 5 * SLOT);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Failed);
    assert!(c
        .reason
        .as_ref()
        .unwrap()
        .explain()
        .contains("not at the meeting place"));
}

#[test]
fn proposals_for_nonsense_are_rejected_without_changing_the_world() {
    let (mut sim, ids) = two_free_pawns();
    propose(&mut sim, ids[0], ids[0], 4, 2, t(5, 5), 10); // self
    propose(
        &mut sim,
        ids[0],
        EntityId::new(Kind::Pawn, 99),
        4,
        2,
        t(5, 5),
        10,
    ); // unknown
    propose(&mut sim, ids[0], ids[1], 47, 5, t(5, 5), 10); // past the end of the day
    propose(&mut sim, ids[0], ids[1], 4, 0, t(5, 5), 10); // empty
    propose(&mut sim, ids[0], ids[1], 4, 2, t(99, 5), 10); // off the map
    let events = run(&mut sim, 1);
    assert_eq!(kinds(&events, "input_rejected").len(), 5);
    assert_eq!(sim.world().commitments.len(), 0);
}

#[test]
fn changing_the_slot_length_cancels_commitments_and_replans_the_day() {
    let (mut sim, ids) = two_free_pawns();
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    run_to(&mut sim, 605);
    assert_eq!(only_commitment(&sim).state, CommitState::Accepted);
    sim.submit_now(SimInput::SettingChange(
        crate::input::SettingChange::SlotMinutes(60),
    ))
    .unwrap();
    let events = run_to(&mut sim, 1250); // the next 60-minute boundary is tick 1200
    assert_eq!(only_commitment(&sim).state, CommitState::Cancelled);
    assert_eq!(kinds(&events, "commitment.cancelled").len(), 1);
    for p in &ids {
        let s = pawn(&sim, *p).schedule.as_ref().unwrap();
        assert_eq!(s.slots_per_day(), 24, "replanned in the new slot size");
        assert!(s.reservations().all(|r| r.commitment.is_none()));
        s.check_invariants().unwrap();
    }
}

// ---- determinism --------------------------------------------------------------------------------------

fn busy_sim() -> Sim {
    let inputs = PlanInputs {
        duties: vec![
            duty("idle_at", at(t(6, 6)), 6, 4),
            duty("idle_at", at(t(2, 9)), 20, 6),
        ],
        leisure: vec![crate::schedule::LeisureOption {
            action: act("move_to"),
            params: Canon::map([("to", t(12, 3).to_canon())]),
            len: 1,
            weight: 5,
            earliest: 0,
            latest: u32::MAX,
            because: String::new(),
        }],
        open_weight: 10,
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0), (9, 9), (3, 12)]);
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 10, 2, t(7, 7), 100_000);
    sim
}

#[test]
fn the_same_inputs_produce_the_same_world() {
    let mut a = busy_sim();
    let mut b = busy_sim();
    run(&mut a, 2 * crate::time::TICKS_PER_DAY);
    run(&mut b, 2 * crate::time::TICKS_PER_DAY);
    assert_eq!(a.world().state_hash(), b.world().state_hash());
    assert_eq!(a.day_hashes(), b.day_hashes());
}

#[test]
fn restoring_a_snapshot_mid_task_continues_identically() {
    let source: Arc<dyn PlanSource> = Arc::new(Fixed(PlanInputs {
        duties: vec![duty("idle_at", at(t(6, 6)), 6, 4)],
        ..PlanInputs::default()
    }));
    let (mut whole, _, _) = world(Arc::clone(&source), &[(0, 0), (9, 9)]);
    run(&mut whole, 6 * SLOT + 7); // mid-walk
    let snap = whole.snapshot();
    run(&mut whole, 3 * SLOT);
    let mut resumed = Sim::restore(snap, pipe(&source));
    run(&mut resumed, 3 * SLOT);
    assert_eq!(whole.world().state_hash(), resumed.world().state_hash());
}

#[test]
fn no_plan_source_means_open_days_and_no_tasks() {
    let (mut sim, _, ids) = world(Arc::new(NoPlans), &[(0, 0)]);
    let events = run(&mut sim, 3 * SLOT);
    assert!(kinds(&events, "task.started").is_empty());
    assert_eq!(
        pawn(&sim, ids[0]).schedule.as_ref().unwrap().free_slots(),
        48
    );
}

// ---- what kept and broken plans leave behind (2.1) -------------------------------------------------

fn outcome_data() -> Arc<GameData> {
    use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    let set = ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap();
    Arc::new(set.game().clone())
}

fn outcome_pipeline() -> Pipeline {
    let source: Arc<dyn PlanSource> = Arc::new(Fixed(PlanInputs::default()));
    let mut pipeline = pipe(&source);
    pipeline.set_builtin(
        crate::pipeline::SystemSlot::Commitment,
        Box::new(CommitmentSystem::new(Some(outcome_data()))),
    );
    pipeline
}

/// Two pawns far apart with a social need of 100, and the commitment system that knows the game data.
fn outcome_world() -> (Sim, EntityId, Vec<EntityId>) {
    let mut w = WorldState::new("t", "seed");
    let map = w.create_map(MapKind::Overworld, 16, 16).unwrap();
    let ids: Vec<EntityId> = [(0, 0), (9, 9)]
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            let id = w.spawn_pawn(&format!("P{i}"), map, t(*x, *y)).unwrap();
            w.pawns
                .get_mut(id)
                .unwrap()
                .needs
                .insert("social".into(), 100);
            id
        })
        .collect();
    (Sim::new(w, outcome_pipeline()), map, ids)
}

fn commitment_params() -> pg_content::gamedata::CommitmentOutcomes {
    outcome_data()
        .relationships
        .as_ref()
        .unwrap()
        .commitment
        .clone()
}

fn memory_keys(sim: &Sim, p: EntityId) -> Vec<String> {
    pawn(sim, p)
        .memories
        .iter()
        .filter_map(|m| m.summary_key.clone())
        .collect()
}

fn wall_in(sim: &mut Sim, map: EntityId, around: (i32, i32)) {
    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
        command(
            sim,
            Command::DevSetBlocked {
                map,
                at: t(around.0 + dx, around.1 + dy),
                blocked: true,
            },
        );
    }
}

#[test]
fn a_kept_plan_leaves_memories_warmth_and_less_loneliness() {
    let (mut sim, _, ids) = outcome_world();
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    let events = run_to(&mut sim, 1810);
    assert_eq!(only_commitment(&sim).state, CommitState::Completed);
    let o = commitment_params();
    let rel = sim.world().relationships.get(ids[0], ids[1]).unwrap();
    assert_eq!(rel.affinity, o.kept_affinity);
    for (me, other) in [(ids[0], ids[1]), (ids[1], ids[0])] {
        assert_eq!(memory_keys(&sim, me), ["memory.commitment.kept"]);
        assert_eq!(pawn(&sim, me).memories[0].participants, vec![other]);
        assert_eq!(pawn(&sim, me).needs["social"], 100 + o.kept_social);
    }
    // The completed event names the pair, so the journal can say who met.
    let done = kinds(&events, "commitment.completed");
    assert_eq!(done.len(), 1);
    assert!(done[0].detail.get("proposer").is_some());
    assert_eq!(
        sim.world().journal.entries().last().unwrap().kind,
        "plan_kept"
    );
}

#[test]
fn a_plan_one_pawn_misses_is_remembered_by_both_and_cools_the_pair() {
    let (mut sim, map, ids) = outcome_world();
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 1, t(5, 5), 100_000);
    run(&mut sim, SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Accepted);
    // The second pawn is walled in; the first can arrive.
    wall_in(&mut sim, map, (9, 9));
    let events = run(&mut sim, 5 * SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Failed);
    let o = commitment_params();
    let rel = sim.world().relationships.get(ids[0], ids[1]).unwrap();
    assert_eq!(rel.affinity, o.stood_up_affinity);
    assert_eq!(memory_keys(&sim, ids[0]), ["memory.commitment.stood_up"]);
    assert_eq!(memory_keys(&sim, ids[1]), ["memory.commitment.missed"]);
    assert!(pawn(&sim, ids[0]).memories[0].impact < 0);
    assert_eq!(pawn(&sim, ids[0]).needs["social"], 100);
    assert_eq!(kinds(&events, "commitment.failed").len(), 1);
    assert_eq!(
        sim.world().journal.entries().last().unwrap().kind,
        "plan_broken"
    );
}

#[test]
fn when_neither_comes_the_cooling_is_halved() {
    let (mut sim, map, ids) = outcome_world();
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 1, t(12, 12), 100_000);
    run(&mut sim, SLOT);
    wall_in(&mut sim, map, (12, 12));
    run(&mut sim, 5 * SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Failed);
    let o = commitment_params();
    let rel = sim.world().relationships.get(ids[0], ids[1]).unwrap();
    assert_eq!(rel.affinity, o.stood_up_affinity / 2);
    for p in &ids {
        assert_eq!(memory_keys(&sim, *p), ["memory.commitment.missed"]);
    }
}

#[test]
fn a_pawn_who_could_not_come_is_excused() {
    let (mut sim, _, ids) = outcome_world();
    run(&mut sim, SLOT + 5);
    propose(&mut sim, ids[0], ids[1], 4, 1, t(5, 5), 100_000);
    run(&mut sim, SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Accepted);
    // The second pawn passes out (the needs system is not installed, so nothing wakes them).
    let mut snap = sim.snapshot();
    snap.world
        .pawns
        .get_mut(ids[1])
        .unwrap()
        .capacities
        .cap("consciousness", 0);
    sim = Sim::restore(snap, outcome_pipeline());
    run(&mut sim, 5 * SLOT);
    assert_eq!(only_commitment(&sim).state, CommitState::Failed);
    assert!(
        sim.world().relationships.get(ids[0], ids[1]).is_none(),
        "nobody is blamed, so nothing between them changes"
    );
    assert!(pawn(&sim, ids[0]).memories.is_empty());
    assert!(pawn(&sim, ids[1]).memories.is_empty());
}

#[test]
fn an_invitee_on_bad_terms_declines() {
    let (sim, _, ids) = outcome_world();
    let data = outcome_data();
    let rel_params = data.relationships.as_ref().unwrap();
    let mut snap = sim.snapshot();
    snap.world.relationships.set(crate::social::Relationship {
        key: crate::social::PairKey::new(ids[0], ids[1]).unwrap(),
        affinity: rel_params.commitment.decline_below - 50,
        label: rel_params.labels[0].id.clone(),
        last_interaction_tick: 0,
        day: 0,
        day_change: 0,
        last_topic: None,
        forgotten: 0,
    });
    let mut sim = Sim::restore(snap, outcome_pipeline());
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 4, 2, t(5, 5), 100_000);
    let events = run_to(&mut sim, 605);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Declined);
    assert!(c.reason.as_ref().unwrap().explain().contains("bad terms"));
    assert_eq!(kinds(&events, "commitment.declined").len(), 1);
    for p in &ids {
        assert!(pawn(&sim, *p)
            .schedule
            .as_ref()
            .unwrap()
            .reservations()
            .all(|r| r.commitment.is_none()));
    }
}

// ---- the leisure hook (2.2) --------------------------------------------------------------------------

struct Answer(i64);

impl crate::hooks::HookHost for Answer {
    fn ask(
        &mut self,
        point: &'static pg_api::HookPoint,
        _w: &WorldState,
        _s: EntityId,
    ) -> Vec<i64> {
        if point.id == "schedule.leisure_weight" {
            vec![self.0]
        } else {
            Vec::new()
        }
    }
}

fn leisure_day(answer: Option<i64>) -> usize {
    let inputs = PlanInputs {
        leisure: vec![crate::schedule::LeisureOption {
            action: act("idle_at"),
            params: at(t(3, 3)),
            len: 1,
            weight: 10,
            earliest: 0,
            latest: u32::MAX,
            because: String::new(),
        }],
        open_weight: 10,
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(inputs)), &[(0, 0)]);
    if let Some(a) = answer {
        sim.set_hooks(Some(Box::new(Answer(a))));
    }
    run(&mut sim, SLOT + 5);
    pawn(&sim, ids[0])
        .schedule
        .as_ref()
        .unwrap()
        .reservations()
        .count()
}

#[test]
fn a_pack_can_make_a_pawn_keener_or_less_keen_on_leisure_within_the_clamp() {
    let plain = leisure_day(None);
    let none = leisure_day(Some(0));
    let keen = leisure_day(Some(100_000)); // clamped to 2000
    assert!(plain > 0);
    assert_eq!(none, 0, "0 leaves all the time open");
    assert!(keen > plain, "{keen} vs {plain}");
}

// ---- conflicts between plans, and why (2.4) --------------------------------------------------------

fn propose_flexible(sim: &mut Sim, a: EntityId, b: EntityId, start: u32, len: u32, place: Tile) {
    command(
        sim,
        Command::DevPropose {
            proposer: a,
            invitee: b,
            start,
            len,
            at: place,
            expires_in: 100_000,
            reschedulable: true,
        },
    );
}

fn commitments(sim: &Sim) -> Vec<&Commitment> {
    sim.world().commitments.iter().map(|(_, c)| c).collect()
}

fn reservation_reasons(sim: &Sim, p: EntityId) -> Vec<String> {
    pawn(sim, p)
        .schedule
        .as_ref()
        .unwrap()
        .reservations()
        .filter(|r| r.commitment.is_some())
        .map(|r| r.reason.explain())
        .collect()
}

#[test]
fn of_two_overlapping_commitments_the_earlier_keeps_the_time_and_the_other_is_told_why() {
    let (mut sim, ids) = two_free_pawns();
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 6, 2, t(5, 5), 100_000);
    run(&mut sim, 1);
    propose(&mut sim, ids[1], ids[0], 7, 2, t(6, 6), 100_000);
    run_to(&mut sim, 605);
    let all = commitments(&sim);
    assert_eq!(all[0].state, CommitState::Accepted);
    assert_eq!(all[1].state, CommitState::Declined);
    let why = all[1].reason.as_ref().unwrap().explain();
    assert!(
        why.contains("made earlier") && why.contains("keeps the time"),
        "{why}"
    );
}

#[test]
fn a_commitment_that_meets_a_duty_loses_and_says_duties_come_first() {
    let busy = PlanInputs {
        duties: vec![duty("idle_at", at(t(9, 9)), 6, 3)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(busy)), &[(0, 0), (9, 9)]);
    run_to(&mut sim, 305);
    propose(&mut sim, ids[0], ids[1], 7, 2, t(5, 5), 100_000);
    run_to(&mut sim, 605);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Declined);
    let why = c.reason.as_ref().unwrap().explain();
    assert!(why.contains("duties come first"), "{why}");
}

#[test]
fn a_reschedulable_commitment_moves_to_the_nearest_time_that_suits_both_and_says_so() {
    // Both have a duty over slots 6 to 8, so the asked-for slot 7 is taken; the nearest free two-slot
    // stretches are 9 to 10 (two slots from the ask) and 4 to 5 (three), so the later one is nearer.
    let busy = PlanInputs {
        duties: vec![duty("idle_at", at(t(9, 9)), 6, 3)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(Arc::new(Fixed(busy)), &[(0, 0), (9, 9)]);
    run_to(&mut sim, 305);
    propose_flexible(&mut sim, ids[0], ids[1], 7, 2, t(5, 5));
    run_to(&mut sim, 605);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Accepted, "{:?}", c.reason);
    assert_eq!(c.start, 9, "moved to the nearest start that suits both");
    for p in &ids {
        let s = pawn(&sim, *p).schedule.as_ref().unwrap();
        let held: Vec<_> = s
            .reservations()
            .filter(|r| r.commitment.is_some())
            .collect();
        assert_eq!(held.len(), 1);
        assert_eq!((held[0].start, held[0].len), (9, 2));
        s.check_invariants().unwrap();
        let why = reservation_reasons(&sim, *p).remove(0);
        assert!(why.contains("moved from slot 7 to slot 9"), "{why}");
    }
}

#[test]
fn a_reschedulable_commitment_with_no_time_that_suits_both_is_declined_with_the_reason() {
    // One of them is busy all day, so there is no other time.
    let all_day = PlanInputs {
        duties: vec![duty("idle_at", at(t(9, 9)), 2, 46)],
        ..PlanInputs::default()
    };
    let (mut sim, _, ids) = world(
        Arc::new(PerPawn(vec![(EntityId::new(Kind::Pawn, 2), all_day)])),
        &[(0, 0), (9, 9)],
    );
    run_to(&mut sim, 305);
    propose_flexible(&mut sim, ids[0], ids[1], 7, 2, t(5, 5));
    run_to(&mut sim, 605);
    let c = only_commitment(&sim);
    assert_eq!(c.state, CommitState::Declined);
    let why = c.reason.as_ref().unwrap().explain();
    assert!(why.contains("no other time today suits both"), "{why}");
    assert_eq!(c.start, 7, "a declined commitment keeps its asked-for time");
}
