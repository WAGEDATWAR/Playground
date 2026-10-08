use super::*;
use crate::id::Kind;
use proptest::prelude::*;

fn act(name: &str) -> ActionId {
    ActionId::new(name).unwrap()
}

fn pawn() -> EntityId {
    EntityId::new(Kind::Pawn, 1)
}

fn seed() -> Seed {
    Seed::from_text("sched-test")
}

fn duty(start: u32, len: u32) -> DutyTemplate {
    DutyTemplate {
        action: act("idle_at"),
        params: Canon::Null,
        start,
        len,
        min_len: len,
        shift_earlier: 0,
        shift_later: 0,
    }
}

fn chore(id: u32, len: u32, urgency: u32, deadline: Option<u32>) -> Chore {
    Chore {
        id,
        action: act("idle_at"),
        params: Canon::Null,
        len,
        urgency,
        deadline_slot: deadline,
        created_tick: 0,
    }
}

fn commitment(n: u32, start: u32, len: u32, reschedulable: bool, created: u64) -> CommitmentReq {
    CommitmentReq {
        id: EntityId::new(Kind::Commitment, n),
        action: act("idle_at"),
        params: Canon::Null,
        start,
        len,
        reschedulable,
        created_tick: created,
    }
}

fn urgent(need: &str, predicted: u32, len: u32) -> UrgentNeed {
    UrgentNeed {
        need: need.into(),
        earliest: 0,
        predicted_slot: predicted,
        action: act("idle_at"),
        params: Canon::Null,
        len,
    }
}

fn plan(inputs: &PlanInputs) -> DaySchedule {
    plan_day(seed(), pawn(), 3, 48, inputs)
}

fn put(s: &mut DaySchedule, p: Priority, start: u32, len: u32, urgency: u32) -> u32 {
    let mut r = blank(
        p,
        &act("idle_at"),
        &Canon::Null,
        start,
        len,
        ReasonCode::simple("slots_open"),
    );
    r.urgency = urgency;
    s.place(r).unwrap()
}

#[test]
fn an_empty_day_is_all_open_and_says_so() {
    let s = plan(&PlanInputs::default());
    assert_eq!(s.free_slots(), 48);
    assert_eq!(s.notes.len(), 1);
    assert_eq!(s.notes[0].explain(), "48 slot(s) were left open.");
    s.check_invariants().unwrap();
}

#[test]
fn overlapping_duties_drop_the_later_one_with_a_reason() {
    let inputs = PlanInputs {
        duties: vec![duty(16, 8), duty(20, 4), duty(30, 2)],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let starts: Vec<u32> = s.by_start().iter().map(|r| r.start).collect();
    assert_eq!(starts, [16, 30]);
    assert_eq!(s.dropped.len(), 1);
    assert!(s.dropped[0]
        .reason
        .explain()
        .contains("overlapped a stronger claim"));
    s.check_invariants().unwrap();
}

#[test]
fn duty_variation_stays_inside_its_bounds_and_is_seeded() {
    let d = DutyTemplate {
        shift_earlier: 2,
        shift_later: 3,
        min_len: 4,
        ..duty(20, 8)
    };
    let inputs = PlanInputs {
        duties: vec![d],
        ..PlanInputs::default()
    };
    let mut seen = std::collections::BTreeSet::new();
    for day in 0..60 {
        let s = plan_day(seed(), pawn(), day, 48, &inputs);
        let r = s.by_start()[0];
        assert!((18..=23).contains(&r.start), "{}", r.start);
        assert!((4..=8).contains(&r.len));
        assert_eq!(
            s,
            plan_day(seed(), pawn(), day, 48, &inputs),
            "same seed, same day"
        );
        seen.insert((r.start, r.len));
    }
    assert!(seen.len() > 4, "variation actually varies: {seen:?}");
}

#[test]
fn urgent_needs_take_the_earliest_free_run_before_the_predicted_slot() {
    let inputs = PlanInputs {
        duties: vec![duty(0, 6)],
        urgent: vec![urgent("hunger", 20, 2), urgent("energy", 10, 3)],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let by: Vec<(u32, u32, Priority)> = s
        .by_start()
        .iter()
        .map(|r| (r.start, r.len, r.priority))
        .collect();
    // energy (predicted 10) is placed first, then hunger.
    assert_eq!(
        by,
        [
            (0, 6, Priority::Duty),
            (6, 3, Priority::UrgentNeed),
            (9, 2, Priority::UrgentNeed)
        ]
    );
}

#[test]
fn an_urgent_need_with_no_room_before_its_deadline_is_reported() {
    let inputs = PlanInputs {
        duties: vec![duty(0, 10)],
        urgent: vec![urgent("hunger", 8, 2)],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    assert_eq!(s.dropped.len(), 1);
    assert!(s.dropped[0]
        .reason
        .explain()
        .starts_with("There was no room before slot 8"));
}

#[test]
fn commitments_hold_their_slot_or_follow_the_reschedulable_flag() {
    let inputs = PlanInputs {
        duties: vec![duty(10, 4)],
        commitments: vec![
            commitment(1, 20, 2, false, 5),
            commitment(2, 11, 2, true, 6),
            commitment(3, 12, 2, false, 7),
        ],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let c = |n: u32| {
        s.reservations()
            .find(|r| r.commitment == Some(EntityId::new(Kind::Commitment, n)))
    };
    assert_eq!(c(1).unwrap().start, 20);
    // #2 may move: the nearest free run of 2 to slot 11 starts at 8 (3 away) or 14 (3 away); the earlier wins.
    assert_eq!(c(2).unwrap().start, 8);
    assert!(c(2)
        .unwrap()
        .reason
        .explain()
        .contains("moved from slot 11 to slot 8"));
    // #3 cannot move and loses to the duty.
    assert!(c(3).is_none());
    assert!(s
        .dropped
        .iter()
        .any(|d| d.reason.code == "commitment_failed"));
    s.check_invariants().unwrap();
}

#[test]
fn equal_priority_commitments_are_ordered_by_creation_then_id() {
    let inputs = PlanInputs {
        commitments: vec![
            commitment(2, 10, 2, false, 5),
            commitment(1, 10, 2, false, 5),
            commitment(3, 10, 2, false, 4),
        ],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let held = s.owner(10).unwrap().commitment.unwrap();
    assert_eq!(held, EntityId::new(Kind::Commitment, 3), "created earliest");
    assert_eq!(s.dropped.len(), 2);
}

#[test]
fn chores_sort_by_urgency_then_deadline_then_id_and_respect_deadlines() {
    let inputs = PlanInputs {
        duties: vec![duty(0, 46)],
        chores: vec![
            chore(1, 1, 1, None),
            chore(2, 1, 5, Some(48)),
            chore(3, 1, 5, Some(40)), // deadline cannot be met
            chore(4, 1, 5, None),
        ],
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let placed: Vec<(u32, u32)> = s
        .reservations()
        .filter(|r| r.priority == Priority::Chore)
        .map(|r| (r.start, r.urgency))
        .collect();
    // Two slots remain (46, 47). Order: #3 (deadline 40, cannot fit), #2 (deadline 48), #4, then #1.
    assert_eq!(placed, [(46, 5), (47, 5)]);
    assert_eq!(s.dropped.len(), 2);
    assert!(s
        .dropped
        .iter()
        .any(|d| d.reason.explain().contains("before slot 40")));
}

#[test]
fn leisure_fills_some_slots_and_leaves_the_rest_open() {
    let inputs = PlanInputs {
        leisure: vec![LeisureOption {
            action: act("idle_at"),
            params: Canon::Null,
            len: 2,
            weight: 10,
        }],
        open_weight: 10,
        ..PlanInputs::default()
    };
    let s = plan(&inputs);
    let used = 48 - s.free_slots();
    assert!(used > 0 && used < 48, "{used}");
    assert!(s.reservations().all(|r| r.priority == Priority::Leisure));
    s.check_invariants().unwrap();
    let packed = plan(&PlanInputs {
        open_weight: 0,
        ..inputs
    });
    assert_eq!(packed.free_slots(), 0);
}

#[test]
fn replanning_keeps_started_reservations_and_replaces_the_rest() {
    let inputs = PlanInputs {
        duties: vec![duty(4, 4), duty(30, 4)],
        chores: vec![chore(1, 2, 3, None)],
        ..PlanInputs::default()
    };
    let mut s = plan(&inputs);
    let before: Vec<Reservation> = s.reservations().filter(|r| r.start < 20).cloned().collect();
    let mut changed = inputs.clone();
    changed.urgent.push(urgent("hunger", 40, 2));
    replan_from(
        &mut s,
        seed(),
        pawn(),
        20,
        &changed,
        "hunger became critical",
    );
    let after: Vec<Reservation> = s.reservations().filter(|r| r.start < 20).cloned().collect();
    assert_eq!(before, after);
    assert!(s
        .reservations()
        .any(|r| r.priority == Priority::UrgentNeed && r.start >= 20));
    assert!(s.notes[0]
        .explain()
        .contains("replanned from slot 20: hunger became critical"));
    s.check_invariants().unwrap();
}

#[test]
fn an_urgent_insert_displaces_the_weakest_that_is_in_the_way_and_nothing_stronger() {
    let mut s = DaySchedule::new(0, 8);
    put(&mut s, Priority::Duty, 0, 2, 0);
    put(&mut s, Priority::Commitment, 2, 2, 0);
    let chore_id = put(&mut s, Priority::Chore, 4, 2, 3);
    let leisure_id = put(&mut s, Priority::Leisure, 6, 2, 0);
    let got = insert_urgent(&mut s, 0, &urgent("energy", 6, 2)).unwrap();
    assert_eq!(
        s.reservation(got).unwrap().start,
        4,
        "took the chore's slots"
    );
    assert!(s
        .reservation(got)
        .unwrap()
        .reason
        .explain()
        .starts_with("The urgent energy activity"));
    // Leisure was removed first but was not in the way, so it is back where it was.
    assert_eq!(s.reservation(leisure_id).map(|r| r.start), Some(6));
    // The chore found no later room and was dropped with a reason.
    assert!(s.reservation(chore_id).is_none());
    assert!(s.dropped.iter().any(|d| d.reason.code == "no_free_slot"));
    s.check_invariants().unwrap();
}

#[test]
fn a_displaced_chore_moves_to_a_later_free_slot_when_one_exists() {
    let mut s = DaySchedule::new(0, 8);
    put(&mut s, Priority::Chore, 0, 2, 2);
    let got = insert_urgent(&mut s, 0, &urgent("energy", 2, 2)).unwrap();
    assert_eq!(s.reservation(got).unwrap().start, 0);
    let chore = s
        .reservations()
        .find(|r| r.priority == Priority::Chore)
        .unwrap();
    assert_eq!(chore.start, 2);
    assert_eq!(chore.reason.code, "moved_later");
    s.check_invariants().unwrap();
}

#[test]
fn an_urgent_insert_that_cannot_fit_changes_nothing() {
    let mut full = DaySchedule::new(0, 4);
    put(&mut full, Priority::Duty, 0, 2, 0);
    put(&mut full, Priority::Commitment, 2, 2, 0);
    let before = full.clone();
    assert_eq!(insert_urgent(&mut full, 0, &urgent("energy", 4, 2)), None);
    assert_eq!(
        full.reservations().collect::<Vec<_>>(),
        before.reservations().collect::<Vec<_>>()
    );
    full.check_invariants().unwrap();
}

#[test]
fn place_rejects_bad_ranges_and_overlaps() {
    let mut s = DaySchedule::new(0, 4);
    let mk = |start, len| {
        blank(
            Priority::Chore,
            &act("idle_at"),
            &Canon::Null,
            start,
            len,
            ReasonCode::simple("slots_open"),
        )
    };
    assert_eq!(s.place(mk(0, 0)), Err(PlaceError::OutOfRange));
    assert_eq!(s.place(mk(3, 2)), Err(PlaceError::OutOfRange));
    assert_eq!(s.place(mk(u32::MAX, 2)), Err(PlaceError::OutOfRange));
    s.place(mk(1, 2)).unwrap();
    assert_eq!(s.place(mk(2, 2)), Err(PlaceError::Occupied));
    assert!(s.remove(1).is_some());
    assert_eq!(s.free_slots(), 4);
    assert!(s.remove(1).is_none());
}

// ---- properties ------------------------------------------------------------------------------------

fn arb_inputs() -> impl Strategy<Value = PlanInputs> {
    let duties =
        prop::collection::vec((0u32..48, 1u32..10, 0u32..3, 0u32..3), 0..4).prop_map(|v| {
            v.into_iter()
                .map(|(start, len, e, l)| DutyTemplate {
                    min_len: 1.max(len / 2),
                    shift_earlier: e,
                    shift_later: l,
                    ..duty(start, len)
                })
                .collect::<Vec<_>>()
        });
    let urgent_ = prop::collection::vec((0u32..60, 1u32..5, 0usize..3), 0..4).prop_map(|v| {
        v.into_iter()
            .map(|(p, len, i)| urgent(["hunger", "energy", "bladder"][i], p, len))
            .collect::<Vec<_>>()
    });
    let commitments = prop::collection::vec((0u32..48, 1u32..6, any::<bool>(), 0u64..20), 0..4)
        .prop_map(|v| {
            v.into_iter()
                .enumerate()
                .map(|(i, (s, l, r, t))| commitment(i as u32 + 1, s, l, r, t))
                .collect::<Vec<_>>()
        });
    let chores = prop::collection::vec((1u32..5, 0u32..10, prop::option::of(0u32..60)), 0..6)
        .prop_map(|v| {
            v.into_iter()
                .enumerate()
                .map(|(i, (l, u, d))| chore(i as u32 + 1, l, u, d))
                .collect::<Vec<_>>()
        });
    let leisure = prop::collection::vec((1u32..5, 0u32..10), 0..3).prop_map(|v| {
        v.into_iter()
            .map(|(len, weight)| LeisureOption {
                action: act("idle_at"),
                params: Canon::Null,
                len,
                weight,
            })
            .collect::<Vec<_>>()
    });
    (duties, urgent_, commitments, chores, leisure, 0u32..20).prop_map(
        |(duties, urgent, commitments, chores, leisure, open_weight)| PlanInputs {
            duties,
            urgent,
            commitments,
            chores,
            leisure,
            open_weight,
        },
    )
}

fn of_priority(s: &DaySchedule, p: Priority) -> Vec<Reservation> {
    s.reservations()
        .filter(|r| r.priority == p)
        .cloned()
        .collect()
}

proptest! {
    #[test]
    fn plans_never_overlap_and_stay_consistent(inputs in arb_inputs(), day in 0u64..5) {
        let s = plan_day(seed(), pawn(), day, 48, &inputs);
        prop_assert!(s.check_invariants().is_ok(), "{:?}", s.check_invariants());
    }

    #[test]
    fn planning_is_deterministic(inputs in arb_inputs()) {
        prop_assert_eq!(plan(&inputs), plan(&inputs));
    }

    #[test]
    fn stronger_priorities_never_depend_on_weaker_inputs(inputs in arb_inputs()) {
        let full = plan(&inputs);
        let duties_only = plan(&PlanInputs { duties: inputs.duties.clone(), ..PlanInputs::default() });
        prop_assert_eq!(of_priority(&full, Priority::Duty), of_priority(&duties_only, Priority::Duty));
        let through_urgent = plan(&PlanInputs { duties: inputs.duties.clone(), urgent: inputs.urgent.clone(), ..PlanInputs::default() });
        prop_assert_eq!(of_priority(&full, Priority::UrgentNeed), of_priority(&through_urgent, Priority::UrgentNeed));
        let through_commit = plan(&PlanInputs { chores: vec![], leisure: vec![], ..inputs.clone() });
        prop_assert_eq!(of_priority(&full, Priority::Commitment), of_priority(&through_commit, Priority::Commitment));
        prop_assert_eq!(of_priority(&full, Priority::Duty), of_priority(&through_commit, Priority::Duty));
    }

    #[test]
    fn replanning_never_touches_started_reservations(inputs in arb_inputs(), from in 0u32..48) {
        let mut s = plan(&inputs);
        let before: Vec<Reservation> = s.reservations().filter(|r| r.start < from).cloned().collect();
        replan_from(&mut s, seed(), pawn(), from, &inputs, "test");
        let after: Vec<Reservation> = s.reservations().filter(|r| r.start < from).cloned().collect();
        prop_assert_eq!(before, after);
        prop_assert!(s.check_invariants().is_ok());
    }

    #[test]
    fn urgent_inserts_keep_the_schedule_valid_and_protect_duties_and_commitments(
        inputs in arb_inputs(), predicted in 0u32..60, len in 1u32..6, from in 0u32..48
    ) {
        let mut s = plan(&inputs);
        let strong: Vec<Reservation> = s.reservations().filter(|r| r.priority <= Priority::Commitment).cloned().collect();
        let _ = insert_urgent(&mut s, from, &urgent("hunger", predicted, len));
        prop_assert!(s.check_invariants().is_ok(), "{:?}", s.check_invariants());
        for r in strong {
            prop_assert_eq!(s.reservation(r.id), Some(&r));
        }
    }
}
