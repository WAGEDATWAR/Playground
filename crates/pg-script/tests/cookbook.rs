//! The cookbook packs and the host, end to end: real packs loaded through the pack loader, driven by the
//! real tick pipeline (Blueprint §18 items 11 and 12, Roadmap 0.9 "one component, system and hook").

use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits, MemoryPack};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::map::Tile;
use pg_core::pipeline::Pipeline;
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use pg_script::host::{attach, ScriptHost, ScriptLimits, ScriptPackInput};
use std::sync::{Arc, Mutex};

const DAY: u64 = 14_400;

fn root(p: &str) -> String {
    format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))
}

fn base() -> pg_content::LoadedPack {
    load_pack(&DirPack::new(root("data/base")), &Limits::default()).unwrap()
}

fn cookbook(name: &str) -> pg_content::LoadedPack {
    load_pack(
        &DirPack::new(root(&format!("packs/cookbook/{name}"))),
        &Limits::default(),
    )
    .unwrap_or_else(|r| panic!("{name}: {r}"))
}

fn build_set(packs: Vec<pg_content::LoadedPack>) -> Arc<ContentSet> {
    Arc::new(
        ContentSet::build(packs, ComponentRegistry::builtin()).unwrap_or_else(|r| panic!("{r}")),
    )
}

fn content(names: &[&str]) -> Arc<ContentSet> {
    let mut packs = vec![base()];
    packs.extend(names.iter().map(|n| cookbook(n)));
    build_set(packs)
}

fn inline_pack(manifest_caps: &str, script: &str, id: &str) -> pg_content::LoadedPack {
    let p = MemoryPack::new()
        .with(
            "pack.json",
            &format!(
                r#"{{"id":"{id}","name":"X","version":"1","capabilities":{manifest_caps},"entry":"scripts/main.luau","depends":[{{"id":"base","version":">=0.1"}}]}}"#
            ),
        )
        .with("scripts/main.luau", script);
    load_pack(&p, &Limits::default()).unwrap()
}

struct Rig {
    sim: Sim,
    host: Arc<Mutex<ScriptHost>>,
}

fn inputs(c: &ContentSet) -> Vec<ScriptPackInput> {
    c.script_packs()
        .iter()
        .map(ScriptPackInput::from_content)
        .collect()
}

fn rebuild(c: &Arc<ContentSet>, snap: pg_core::sim::SimSnapshot) -> Rig {
    rebuild_with(c, snap, true)
}

/// `dev` installs the dev scaffolding (a plan source that keeps pawns busy); without it pawns only move
/// when told to.
fn rebuild_with(c: &Arc<ContentSet>, snap: pg_core::sim::SimSnapshot, dev: bool) -> Rig {
    let mut p = Pipeline::new();
    if dev {
        pg_core::dev::install(&mut p);
    }
    let (host, hooks) = attach(
        &inputs(c),
        snap.world.seed().0,
        ScriptLimits::default(),
        &mut p,
    )
    .unwrap();
    let mut sim = Sim::restore(snap, p).with_content(Arc::clone(c));
    sim.set_hooks(Some(Box::new(hooks)));
    Rig { sim, host }
}

fn fresh(c: &Arc<ContentSet>, world: WorldState) -> Rig {
    rebuild(
        c,
        pg_core::sim::SimSnapshot {
            world,
            pending: Vec::new(),
            next_seq: 0,
        },
    )
}

fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

fn town(c: &Arc<ContentSet>, pawns: u32) -> Rig {
    let mut r = fresh(c, WorldState::new("Cookbook", "cookbook-seed"));
    r.sim
        .submit(
            0,
            cmd(Command::DevCreateMap {
                w: 40,
                h: 30,
                style: 1,
            }),
        )
        .unwrap();
    for i in 0..pawns {
        r.sim
            .submit(
                0,
                cmd(Command::DevSpawnPawn {
                    map: EntityId::new(Kind::Map, 1),
                    at: None,
                    name: format!("P{i}"),
                }),
            )
            .unwrap();
    }
    r
}

fn lock(h: &Mutex<ScriptHost>) -> std::sync::MutexGuard<'_, ScriptHost> {
    h.lock().unwrap()
}

#[test]
fn the_cookbook_packs_load_and_register_what_they_say() {
    let c = content(&["caffeine", "evening_legs", "birthdays"]);
    let host = ScriptHost::new(&inputs(&c), 1, ScriptLimits::default());
    assert!(host.failures().is_empty(), "{:?}", host.failures());
    assert_eq!(
        host.active_packs(),
        ["birthdays", "caffeine", "evening_legs"]
    );
    let names: Vec<&str> = host.schemas().iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["birthdays.town_age", "caffeine.caffeine"]);
}

#[test]
fn a_pack_with_scripts_changes_the_world_through_validated_writes() {
    let c = content(&["caffeine", "birthdays"]);
    let mut r = town(&c, 8);
    r.sim.run_ticks(3 * DAY).unwrap();
    let w = r.sim.world();
    assert!(!w.ext.is_empty());
    let host = lock(&r.host);
    let age = host.schemas().get("birthdays.town_age").unwrap();
    for (id, _) in w.pawns.iter() {
        assert_eq!(
            w.ext.fields_of(age, id)["days"],
            3,
            "{id} has lived three days"
        );
    }
    let caf = host.schemas().get("caffeine.caffeine").unwrap();
    let levels: Vec<i64> = w
        .pawns
        .iter()
        .map(|(id, _)| w.ext.fields_of(caf, id)["level"])
        .collect();
    assert!(levels.iter().all(|l| (0..=600).contains(l)), "{levels:?}");
    assert!(w.table_hashes().iter().any(|(n, _)| *n == "ext"));
}

#[test]
fn the_same_run_gives_the_same_hash_and_the_world_round_trips() {
    let c = content(&["caffeine", "evening_legs", "birthdays"]);
    let run = || {
        let mut r = town(&c, 10);
        r.sim.run_ticks(2 * DAY).unwrap();
        r.sim.world().state_hash()
    };
    assert_eq!(run(), run());
    let mut r = town(&c, 10);
    r.sim.run_ticks(DAY + 777).unwrap();
    let back = WorldState::from_canon(&pg_core::canon::ToCanon::to_canon(r.sim.world())).unwrap();
    assert_eq!(back.state_hash(), r.sim.world().state_hash());
    assert_eq!(back.ext, r.sim.world().ext);
}

#[test]
fn rebuilding_every_vm_at_any_tick_changes_nothing() {
    // The VM-reload variant (Blueprint §18 item 12): snapshot, throw the host and all VMs away, build new
    // ones, carry on. The result must equal a straight run.
    let c = content(&["caffeine", "evening_legs", "birthdays"]);
    let straight = {
        let mut r = town(&c, 10);
        r.sim.run_ticks(3 * DAY).unwrap();
        r.sim.world().state_hash()
    };
    for chunk in [DAY, 1_500, 777] {
        let mut r = town(&c, 10);
        let mut done = 0;
        while done < 3 * DAY {
            let n = chunk.min(3 * DAY - done);
            r.sim.run_ticks(n).unwrap();
            done += n;
            r = rebuild(&c, r.sim.snapshot());
        }
        assert_eq!(
            r.sim.world().state_hash(),
            straight,
            "reloading every {chunk} ticks"
        );
    }
}

fn arrival_ticks(c: &Arc<ContentSet>, start_tick: u64) -> u64 {
    let mut r = rebuild_with(
        c,
        pg_core::sim::SimSnapshot {
            world: WorldState::new("Walk", "walk-seed"),
            pending: Vec::new(),
            next_seq: 0,
        },
        false,
    );
    r.sim
        .submit(
            0,
            cmd(Command::DevCreateMap {
                w: 60,
                h: 8,
                style: 0,
            }),
        )
        .unwrap();
    r.sim
        .submit(
            0,
            cmd(Command::DevSpawnPawn {
                map: EntityId::new(Kind::Map, 1),
                at: Some(Tile::new(1, 4)),
                name: "Walker".into(),
            }),
        )
        .unwrap();
    r.sim.run_ticks(start_tick.max(2)).unwrap();
    let pawn = EntityId::new(Kind::Pawn, 1);
    let now = r.sim.world().clock.tick();
    // The pawn has lived its own life until now, so walk a fixed distance from wherever it stands.
    let here = r.sim.world().pawns.get(pawn).unwrap().position.tile;
    let goal = Tile::new(
        if here.x < 30 {
            here.x + 20
        } else {
            here.x - 20
        },
        here.y,
    );
    r.sim
        .submit(now, cmd(Command::DevMove { pawn, to: goal }))
        .unwrap();
    for _ in 0..5_000 {
        r.sim.step().unwrap();
        if r.sim.world().pawns.get(pawn).unwrap().position.tile == goal {
            return r.sim.world().clock.tick() - now;
        }
    }
    panic!("never arrived");
}

#[test]
fn hooks_bias_walking_speed_within_the_engine_clamp() {
    let none = arrival_ticks(&content(&[]), 0);
    let legs = content(&["evening_legs"]);
    let day = arrival_ticks(&legs, 0);
    let evening = arrival_ticks(&legs, 20 * 60 * 10 + 10);
    assert_eq!(
        day, none,
        "before 20:00 the pack answers 1000, which is no change"
    );
    assert!(
        evening > none,
        "after 20:00 pawns walk slower ({evening} vs {none})"
    );
    assert!(evening <= none * 2, "the slowdown is bounded by the clamp");
}

#[test]
fn a_failing_pack_is_quarantined_and_the_world_carries_on() {
    let bad = inline_pack(
        r#"["data","systems"]"#,
        r#"
            pg.components.register({ name = "c", applies_to = {"pawn"}, fields = { v = pg.field.int(0, 10, 0) } })
            pg.systems.register({ id = "boom", cadence = "minute", query = { kind = "pawn" }, run = function(ctx, p) error("boom") end })
        "#,
        "bad",
    );
    let c = build_set(vec![base(), bad]);
    let mut r = town(&c, 3);
    let mut events = Vec::new();
    for _ in 0..200 {
        events.extend(r.sim.step().unwrap().events);
    }
    assert!(r.sim.world().ext.is_quarantined("bad"));
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "script.quarantined")
            .count(),
        1
    );
    assert!(events.iter().filter(|e| e.kind == "script.error").count() >= 3);
    // Quarantine is world state: it survives a snapshot and a rebuild of every VM, and stays in force.
    let hash = r.sim.world().state_hash();
    let mut r = rebuild(&c, r.sim.snapshot());
    assert_eq!(r.sim.world().state_hash(), hash);
    let before = r.sim.world().ext.health("bad").unwrap().errors.len();
    r.sim.run_ticks(500).unwrap();
    assert_eq!(
        r.sim.world().ext.health("bad").unwrap().errors.len(),
        before,
        "no further errors once quarantined"
    );
}

#[test]
fn a_pack_that_fails_to_load_is_quarantined_at_once_and_others_still_run() {
    let broken = inline_pack(r#"["data"]"#, "error('load failure')", "broken");
    let c = build_set(vec![base(), broken, cookbook("birthdays")]);
    let mut r = town(&c, 2);
    let mut events = Vec::new();
    for _ in 0..(DAY + 10) {
        events.extend(r.sim.step().unwrap().events);
    }
    assert!(r.sim.world().ext.is_quarantined("broken"));
    assert!(!r.sim.world().ext.is_quarantined("birthdays"));
    assert!(events.iter().any(|e| e.kind == "script.quarantined"));
    assert_eq!(lock(&r.host).failures().len(), 1);
    let age = lock(&r.host)
        .schemas()
        .get("birthdays.town_age")
        .unwrap()
        .clone();
    let any = r.sim.world().pawns.iter().next().unwrap().0;
    assert_eq!(r.sim.world().ext.fields_of(&age, any)["days"], 1);
}

#[test]
fn fuel_limits_stop_a_runaway_system_without_stopping_the_tick() {
    let spin = inline_pack(
        r#"["systems"]"#,
        r#"pg.systems.register({ id = "spin", cadence = "tick", query = { kind = "pawn" }, run = function() while true do end end })"#,
        "spin",
    );
    let c = build_set(vec![base(), spin]);
    let mut r = town(&c, 20);
    let mut quarantined = false;
    for _ in 0..50 {
        let report = r.sim.step().unwrap();
        quarantined |= report.events.iter().any(|e| e.kind == "script.quarantined");
    }
    assert!(quarantined);
    let l = ScriptLimits::default();
    assert!(lock(&r.host).fuel_used() <= 3 * l.fuel_per_tick + 3 * 20 * l.fuel_per_call);
}

#[test]
fn a_system_may_not_write_another_packs_component() {
    let thief = inline_pack(
        r#"["data","systems"]"#,
        r#"pg.systems.register({ id = "steal", cadence = "minute", query = { kind = "pawn" }, writes = { "caffeine.caffeine" }, run = function() end })"#,
        "thief",
    );
    let c = build_set(vec![base(), cookbook("caffeine"), thief]);
    let host = ScriptHost::new(&inputs(&c), 1, ScriptLimits::default());
    assert_eq!(host.failures().len(), 1);
    assert!(
        host.failures()[0]
            .message
            .contains("only its own components"),
        "{:?}",
        host.failures()
    );
}

#[test]
fn safe_mode_keeps_pack_data_as_inert_orphans() {
    let c = content(&["caffeine", "birthdays"]);
    let mut r = town(&c, 5);
    r.sim.run_ticks(2 * DAY).unwrap();
    let with_packs = r.sim.world().ext.clone();
    assert!(!with_packs.is_empty());
    // Open the same world with scripts off: a plain pipeline, no host.
    let mut p = Pipeline::new();
    pg_core::dev::install(&mut p);
    let mut safe = Sim::restore(r.sim.snapshot(), p).with_content(Arc::clone(&c));
    safe.run_ticks(DAY).unwrap();
    assert_eq!(
        safe.world().ext,
        with_packs,
        "orphan data is untouched while the packs are off"
    );
    // Turn the packs back on and they carry on from where they were.
    let mut back = rebuild(&c, safe.snapshot());
    back.sim.run_ticks(DAY).unwrap();
    let age = lock(&back.host)
        .schemas()
        .get("birthdays.town_age")
        .unwrap()
        .clone();
    let any = back.sim.world().pawns.iter().next().unwrap().0;
    assert_eq!(
        back.sim.world().ext.fields_of(&age, any)["days"],
        3,
        "the day that passed while the pack was off was not counted"
    );
}

#[test]
fn what_a_pack_prints_reaches_the_developer_console_at_the_level_it_chose() {
    use pg_core::events::EventCatalog;
    let talker = inline_pack(
        r#"["data","systems"]"#,
        r#"
            pg.components.register({ name = "c", applies_to = {"pawn"}, fields = { v = pg.field.int(0, 10, 0) } })
            pg.systems.register({ id = "talk", cadence = "minute", query = { kind = "pawn" }, run = function(ctx, p)
                pg.log.info("hello from the pack")
                pg.log.warn("careful now")
            end })
        "#,
        "talker",
    );
    let c = build_set(vec![base(), talker]);
    let mut r = town(&c, 1);
    let mut events = Vec::new();
    for _ in 0..60 {
        events.extend(r.sim.step().unwrap().events);
    }
    let catalog = EventCatalog::shared();
    let lines: Vec<(&str, String)> = events
        .iter()
        .filter(|e| e.kind == "script.log")
        .map(|e| {
            let (sev, text) = catalog.console_line(e);
            (sev.name(), text)
        })
        .collect();
    assert!(
        lines.contains(&("Info", "[talker] hello from the pack".into())),
        "{lines:?}"
    );
    assert!(lines.contains(&("Warn", "[talker] careful now".into())));
    // The events are declared, so debug builds validated them as they were emitted.
    assert!(catalog.get("script.log").is_some());
}
