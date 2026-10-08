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

// ---- the developer console (milestone 1.3a) -------------------------------------------------------------

use pg_host::console::Severity;

#[test]
fn every_builtin_event_kind_has_a_listed_console_severity_and_nothing_extra_is_listed() {
    let c = EventCatalog::builtin();
    let listed: std::collections::BTreeSet<&str> = SEVERITIES.iter().map(|(k, _)| *k).collect();
    let declared: std::collections::BTreeSet<&str> = c.iter().map(|d| d.kind.as_str()).collect();
    assert_eq!(
        declared.difference(&listed).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "an event kind has no console severity: add it to SEVERITIES"
    );
    assert_eq!(
        listed.difference(&declared).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "SEVERITIES lists a kind the catalog does not declare"
    );
    assert_eq!(SEVERITIES.len(), listed.len(), "no kind is listed twice");
    for d in c.iter() {
        assert_eq!(
            SEVERITIES
                .iter()
                .find(|(k, _)| *k == d.kind)
                .map(|(_, s)| *s),
            Some(d.severity)
        );
    }
}

#[test]
fn what_a_developer_should_notice_is_not_buried_at_debug() {
    let c = EventCatalog::builtin();
    for kind in [
        "input_rejected",
        "move.failed",
        "task.failed",
        "need.critical",
        "pawn.collapsed",
        "commitment.failed",
    ] {
        assert_eq!(c.severity_of(kind), Severity::Warn, "{kind}");
    }
    for kind in ["script.error", "script.quarantined"] {
        assert_eq!(c.severity_of(kind), Severity::Error, "{kind}");
    }
    for kind in [
        "task.started",
        "move.arrived",
        "schedule.planned",
        "mood.changed",
    ] {
        assert_eq!(c.severity_of(kind), Severity::Debug, "{kind}");
    }
    assert_eq!(c.severity_of("town.generated"), Severity::Info);
    assert_eq!(
        c.severity_of("somepack.thing"),
        Severity::Info,
        "a pack's own kinds"
    );
}

#[test]
fn console_lines_read_as_kind_and_fields_with_the_explanation_in_place_of_the_reason() {
    let c = EventCatalog::builtin();
    let (sev, line) = c.console_line(&ev(
        "task.failed",
        Canon::map([
            ("pawn", pawn(3)),
            ("reservation", Canon::Int(7)),
            ("reason", Canon::map([("code", Canon::str("path_blocked"))])),
            (
                "explain",
                Canon::str("The route stayed blocked by other pawns."),
            ),
        ]),
    ));
    assert_eq!(sev, Severity::Warn);
    assert_eq!(
        line,
        "task.failed pawn=pawn_3 reservation=7: The route stayed blocked by other pawns."
    );
    let (sev, line) = c.console_line(&ev(
        "need.urgent",
        Canon::map([
            ("pawn", pawn(1)),
            ("need", Canon::str("hunger")),
            ("value", Canon::Int(290)),
        ]),
    ));
    assert_eq!(
        (sev, line.as_str()),
        (
            Severity::Info,
            "need.urgent need=hunger pawn=pawn_1 value=290"
        )
    );
    // A script pack's own line shows at the level it chose, under its name.
    let log = |level: &str| {
        ev(
            "script.log",
            Canon::map([
                ("pack", Canon::str("caffeine")),
                ("level", Canon::str(level)),
                ("text", Canon::str("hello")),
            ]),
        )
    };
    assert_eq!(
        c.console_line(&log("info")),
        (Severity::Info, "[caffeine] hello".into())
    );
    assert_eq!(c.console_line(&log("warn")).0, Severity::Warn);
    // An event this catalog does not know (a pack's) is shown, not lost.
    let (sev, line) = c.console_line(&ev("mypack.party", Canon::map([("n", Canon::Int(3))])));
    assert_eq!(sev, Severity::Info);
    assert!(line.starts_with("mypack.party "), "{line}");
}
