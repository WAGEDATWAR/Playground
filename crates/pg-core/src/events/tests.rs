use super::*;
use crate::canon::{Canon, ToCanon};
use crate::commands::Command;
use crate::id::{EntityId, Kind};
use crate::input::SimInput;
use crate::map::Tile;
use crate::sim::Sim;
use crate::world::WorldState;

fn ev(kind: &str, detail: Canon) -> Event {
    Event {
        tick: 1,
        kind: kind.to_owned(),
        detail,
    }
}

fn pawn(n: u32) -> Canon {
    EntityId::new(Kind::Pawn, n).to_canon()
}

#[test]
fn well_formed_events_validate() {
    let c = EventCatalog::builtin();
    let good = [
        ev(
            "pawn.spawned",
            Canon::map([("pawn", pawn(1)), ("tile", Tile::new(3, 4).to_canon())]),
        ),
        ev(
            "move.failed",
            Canon::map([("pawn", pawn(1)), ("reason", Canon::str("path_blocked"))]),
        ),
        ev(
            "commitment.accepted",
            Canon::map([("commitment", EntityId::new(Kind::Commitment, 1).to_canon())]),
        ),
        ev(
            "input_rejected",
            Canon::map([("reason", Canon::str("no free tile"))]),
        ),
        ev(
            "task.failed",
            Canon::map([
                ("pawn", pawn(2)),
                ("reservation", Canon::Null),
                ("reason", Canon::map([("code", Canon::str("x"))])),
                ("explain", Canon::str("Because.")),
            ]),
        ),
    ];
    for e in &good {
        assert_eq!(c.validate(e), Ok(()), "{e:?}");
    }
    assert!(c.validate_all(&good).is_empty());
}

#[test]
fn unknown_kinds_missing_extra_and_mistyped_fields_are_caught_with_a_hint() {
    let c = EventCatalog::builtin();
    let e = c.validate(&ev("pawn.spawnd", Canon::Null)).unwrap_err();
    assert!(
        e.contains("not in the catalog") && e.contains("pawn.spawned"),
        "{e}"
    );
    let tile = Tile::new(1, 1).to_canon();
    for bad in [
        ev("pawn.spawned", Canon::map([("pawn", pawn(1))])),
        ev(
            "pawn.spawned",
            Canon::map([
                ("pawn", pawn(1)),
                ("tile", tile.clone()),
                ("extra", Canon::Int(1)),
            ]),
        ),
        ev(
            "pawn.spawned",
            Canon::map([("pawn", Canon::str("obj_1")), ("tile", tile.clone())]),
        ),
        ev(
            "pawn.spawned",
            Canon::map([("pawn", pawn(1)), ("tile", Canon::str("3,4"))]),
        ),
        ev(
            "move.failed",
            Canon::map([("pawn", pawn(1)), ("reason", Canon::Int(5))]),
        ),
        ev(
            "schedule.planned",
            Canon::map([
                ("pawn", pawn(1)),
                ("day", Canon::Int(-1)),
                ("reservations", Canon::Int(1)),
                ("dropped", Canon::Int(0)),
            ]),
        ),
        ev("pawn.spawned", Canon::Null),
    ] {
        assert!(c.validate(&bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        c.validate_all(&[ev("x", Canon::Null), ev("y", Canon::Null)])
            .len(),
        2
    );
}

#[test]
fn packs_register_namespaced_events_once() {
    let mut c = EventCatalog::builtin();
    let fields = ParamSchema::new().field("cups", Field::required_int(0, 100));
    c.register(
        "coffee",
        "brewed",
        fields.clone(),
        true,
        "A coffee was brewed.",
    )
    .unwrap();
    assert!(c
        .register("coffee", "brewed", fields.clone(), true, "dup")
        .is_err());
    assert!(c
        .register("Coffee", "x", fields.clone(), true, "bad")
        .is_err());
    assert!(c.register("coffee", "a.b", fields, true, "bad").is_err());
    assert_eq!(
        c.validate(&ev("coffee.brewed", Canon::map([("cups", Canon::Int(2))]))),
        Ok(())
    );
    assert!(c
        .validate(&ev(
            "coffee.brewed",
            Canon::map([("cups", Canon::Int(200))])
        ))
        .is_err());
    assert_eq!(c.get("coffee.brewed").unwrap().category, Category::Pack);
}

#[test]
fn the_catalog_is_well_formed() {
    let c = EventCatalog::builtin();
    assert!(c.iter().count() >= 25);
    for d in c.iter() {
        assert!(
            d.kind
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch == '.'),
            "{}",
            d.kind
        );
        assert!(!d.summary.is_empty());
    }
    assert!(
        !c.get("move.sidestep").unwrap().default_visible,
        "noisy kinds are hidden by default"
    );
    assert!(c.get("task.failed").unwrap().default_visible);
}

fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

/// A rich run exercises almost every emitter; every event it produces must be in the catalog and valid.
/// (`Sim::step` also checks in debug builds, so any emitter that drifts fails the whole test suite.)
#[test]
fn every_event_a_rich_run_emits_is_declared_and_valid() {
    let mut sim = Sim::with_dev_systems(WorldState::new("Cat", "catalog"));
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 30,
            h: 24,
            style: 1,
        }),
    )
    .unwrap();
    let map = EntityId::new(Kind::Map, 1);
    for i in 0..6 {
        sim.submit(
            0,
            cmd(Command::DevSpawnPawn {
                map,
                at: None,
                name: format!("P{i}"),
            }),
        )
        .unwrap();
    }
    sim.submit(
        10,
        cmd(Command::DevSetBlocked {
            map,
            at: Tile::new(2, 2),
            blocked: true,
        }),
    )
    .unwrap();
    sim.submit(
        11,
        cmd(Command::DevMove {
            pawn: EntityId::new(Kind::Pawn, 1),
            to: Tile::new(9, 9),
        }),
    )
    .unwrap();
    sim.submit(100, cmd(Command::DevNudge { amount: 5 }))
        .unwrap();
    sim.submit(
        100,
        cmd(Command::DevPropose {
            proposer: EntityId::new(Kind::Pawn, 1),
            invitee: EntityId::new(Kind::Pawn, 1),
            start: 1,
            len: 1,
            at: Tile::new(5, 5),
            expires_in: 10,
            reschedulable: false,
        }),
    )
    .unwrap();
    for (tick, a, b, start) in [(400u64, 1u32, 2u32, 12u32), (3000, 3, 4, 30)] {
        sim.submit(
            tick,
            cmd(Command::DevPropose {
                proposer: EntityId::new(Kind::Pawn, a),
                invitee: EntityId::new(Kind::Pawn, b),
                start,
                len: 2,
                at: Tile::new(8, 8),
                expires_in: 5000,
                reschedulable: false,
            }),
        )
        .unwrap();
    }
    sim.submit(
        9000,
        SimInput::SettingChange(crate::input::SettingChange::SlotMinutes(60)),
    )
    .unwrap();
    let catalog = EventCatalog::builtin();
    let mut kinds = std::collections::BTreeSet::new();
    for _ in 0..30_000 {
        let report = sim.step().unwrap();
        for e in &report.events {
            kinds.insert(e.kind.clone());
        }
        let problems = catalog.validate_all(&report.events);
        assert!(problems.is_empty(), "{problems:?}");
    }
    assert!(
        kinds.len() >= 17,
        "the run was rich enough to matter: {kinds:?}"
    );
}
