//! Save -> load round trips and corruption handling for `WorldState::from_canon`.

use crate::canon::{json, Canon, ToCanon};
use crate::commands::Command;
use crate::id::{EntityId, Kind};
use crate::input::SimInput;
use crate::map::Tile;
use crate::sim::Sim;
use crate::world::{RestoreError, WorldState, SCHEMA_VERSION};
use pg_content::{load_pack, ComponentRegistry, ContentSet, Limits, MemoryPack};
use proptest::prelude::*;
use std::sync::Arc;

fn content() -> Arc<ContentSet> {
    let pack = MemoryPack::new()
        .with(
            "pack.json",
            r#"{"id":"base","name":"Base","version":"0.1.0"}"#,
        )
        .with(
            "data/templates/t.json",
            r#"[{"id":"base.object","schema":1},{"id":"item.coin","schema":1,"extends":"base.object"},
                {"id":"base.item","schema":1,"extends":"base.object","tags":["item"]},
                {"id":"furniture.drawer","schema":1,"extends":"base.item",
                 "components":{"container":{"containers":[{"id":"contents","capacity":4}]}}}]"#,
        );
    Arc::new(
        ContentSet::build(
            vec![load_pack(&pack, &Limits::default()).unwrap()],
            ComponentRegistry::builtin(),
        )
        .unwrap(),
    )
}

fn cmd(c: Command) -> SimInput {
    SimInput::Command { actor: None, cmd: c }
}

/// A town with pawns, objects, a commitment and a mid-run slot change: every table is populated.
fn busy_sim() -> Sim {
    let mut sim = Sim::with_dev_systems(WorldState::new("Save Town", "save-seed")).with_content(content());
    sim.submit(0, cmd(Command::DevCreateMap { w: 32, h: 24, style: 1 })).unwrap();
    let map = EntityId::new(Kind::Map, 1);
    for i in 0..5 {
        sim.submit(0, cmd(Command::DevSpawnPawn { map, at: None, name: format!("P{i}") })).unwrap();
    }
    sim.submit(1, cmd(Command::DevSpawnObject { map, at: Tile::new(5, 5), template: "furniture.drawer".into() })).unwrap();
    sim.submit(1, cmd(Command::DevSpawnObject { map, at: Tile::new(6, 5), template: "item.coin".into() })).unwrap();
    sim.submit(
        2,
        cmd(Command::DevPutInContainer {
            child: EntityId::new(Kind::Object, 2),
            owner: EntityId::new(Kind::Object, 1),
            container: "contents".into(),
        }),
    )
    .unwrap();
    sim.submit(
        400,
        cmd(Command::DevPropose {
            proposer: EntityId::new(Kind::Pawn, 1),
            invitee: EntityId::new(Kind::Pawn, 2),
            start: 12,
            len: 2,
            at: Tile::new(8, 8),
            expires_in: 10_000,
            reschedulable: false,
        }),
    )
    .unwrap();
    sim
}

fn round_trip(w: &WorldState) -> WorldState {
    let text = w.to_canon().to_canonical_string();
    let parsed = json::parse(&text).unwrap();
    WorldState::from_canon(&parsed).unwrap()
}

#[test]
fn a_saved_world_loads_back_identical_at_every_stage_of_a_run() {
    let mut sim = busy_sim();
    let mut done = 0u64;
    for stop in [0u64, 1, 350, 1_900, 3_700, 9_000, 14_400, 30_000] {
        sim.run_ticks(stop - done).unwrap();
        done = stop;
        let w = sim.world();
        let back = round_trip(w);
        // Compared canonically: derived fields (map edit versions, dirty chunks) are not saved.
        assert_eq!(back.to_canon(), w.to_canon(), "tick {stop}");
        assert_eq!(back.occupancy, w.occupancy, "tick {stop}");
        assert_eq!(back.state_hash(), w.state_hash(), "tick {stop}");
    }
    // The run actually exercised the rich state.
    let w = sim.world();
    assert!(w.pawns.iter().any(|(_, p)| p.schedule.is_some()));
    assert!(w.commitments.len() == 1 && w.objects.len() == 2);
}

#[test]
fn a_restored_sim_continues_exactly_like_the_original() {
    let mut original = busy_sim();
    original.run_ticks(5_000).unwrap();
    let snap = original.snapshot();
    let mut restored_snap = snap.clone();
    restored_snap.world = round_trip(&snap.world);
    let mut pipeline = crate::pipeline::Pipeline::new();
    crate::dev::install(&mut pipeline);
    let mut resumed = Sim::restore(restored_snap, pipeline).with_content(content());
    original.run_ticks(20_000).unwrap();
    resumed.run_ticks(20_000).unwrap();
    assert_eq!(original.world().state_hash(), resumed.world().state_hash());
}

fn world_json() -> Canon {
    let mut sim = busy_sim();
    sim.run_ticks(3_700).unwrap();
    sim.world().to_canon()
}

fn load(c: &Canon) -> Result<WorldState, RestoreError> {
    WorldState::from_canon(c)
}

fn set(c: &mut Canon, path: &[&str], value: Canon) {
    let mut cur = c;
    for (i, key) in path.iter().enumerate() {
        let Canon::Map(m) = cur else { panic!("not a map at {key}") };
        if i + 1 == path.len() {
            m.insert((*key).to_owned(), value);
            return;
        }
        cur = m.get_mut(*key).unwrap();
    }
}

fn remove(c: &mut Canon, path: &[&str]) {
    let mut cur = c;
    for (i, key) in path.iter().enumerate() {
        let Canon::Map(m) = cur else { panic!("not a map at {key}") };
        if i + 1 == path.len() {
            m.remove(*key);
            return;
        }
        cur = m.get_mut(*key).unwrap();
    }
}

#[test]
fn wrong_schema_is_refused_with_both_numbers() {
    let mut c = world_json();
    set(&mut c, &["schema"], Canon::Int(i128::from(SCHEMA_VERSION) + 1));
    assert_eq!(
        load(&c).unwrap_err(),
        RestoreError::Schema { found: SCHEMA_VERSION + 1, expected: SCHEMA_VERSION }
    );
}

#[test]
fn missing_unknown_and_mistyped_fields_report_their_path() {
    let mut c = world_json();
    remove(&mut c, &["settings", "movement", "max_repaths"]);
    let e = load(&c).unwrap_err().to_string();
    assert!(e.contains("settings.movement") && e.contains("max_repaths"), "{e}");

    let mut c = world_json();
    set(&mut c, &["extra"], Canon::Int(1));
    assert!(load(&c).unwrap_err().to_string().contains("unknown field 'extra'"));

    let mut c = world_json();
    set(&mut c, &["clock", "tick"], Canon::str("soon"));
    assert!(load(&c).unwrap_err().to_string().starts_with("clock.tick"));

    let mut c = world_json();
    set(&mut c, &["settings", "slot_minutes"], Canon::Int(7));
    assert!(load(&c).unwrap_err().to_string().contains("slot_minutes"));
}

#[test]
fn table_keys_must_match_row_ids() {
    let mut c = world_json();
    let Canon::Map(top) = &mut c else { panic!() };
    let Some(Canon::Map(pawns)) = top.get_mut("pawns") else { panic!() };
    let row = pawns.remove("pawn_1").unwrap();
    pawns.insert("pawn_9".into(), row);
    assert!(matches!(load(&c).unwrap_err(), RestoreError::KeyMismatch { table: "pawns", .. }));
}

#[test]
fn a_corrupt_schedule_is_refused_not_loaded() {
    let mut c = world_json();
    let Canon::Map(top) = &mut c else { panic!() };
    let Some(Canon::Map(pawns)) = top.get_mut("pawns") else { panic!() };
    let pawn = pawns.values_mut().find(|p| !matches!(p.get("schedule"), Some(Canon::Null) | None)).unwrap();
    let Some(Canon::Map(sched)) = pawn.get("schedule").cloned().map(|s| s) else { panic!() };
    let mut sched = Canon::Map(sched);
    // Two reservations on the same slots.
    let Canon::Map(m) = &mut sched else { panic!() };
    let Some(Canon::List(rs)) = m.get_mut("reservations") else { panic!() };
    assert!(!rs.is_empty());
    let mut dup = rs[0].clone();
    if let Canon::Map(d) = &mut dup {
        d.insert("id".into(), Canon::Int(9_999));
    }
    rs.push(dup);
    m.insert("next_id".into(), Canon::Int(10_000));
    let Canon::Map(p) = pawn else { panic!() };
    p.insert("schedule".into(), sched);
    let e = load(&c).unwrap_err().to_string();
    assert!(e.contains("cannot be placed"), "{e}");
}

#[test]
fn map_arrays_must_match_the_declared_size() {
    let mut c = world_json();
    set(&mut c, &["maps", "map_1", "terrain"], Canon::str("00"));
    let e = load(&c).unwrap_err().to_string();
    assert!(e.contains("maps.map_1.terrain") && e.contains("hex characters"), "{e}");
    // A huge declared size is refused before anything is allocated.
    let mut c = world_json();
    set(&mut c, &["maps", "map_1", "w"], Canon::Int(1_000_000));
    assert!(load(&c).is_err());
}

#[test]
fn an_empty_world_round_trips() {
    let w = WorldState::new("Empty", "s");
    assert_eq!(round_trip(&w).to_canon(), w.to_canon());
}

proptest! {
    /// Saves are untrusted input: flipping any byte of a valid save must give Ok or Err, never a panic.
    #[test]
    fn mutated_saves_never_panic(pos in 0usize..200_000, byte in any::<u8>()) {
        let text = world_json_text();
        let mut bytes = text.into_bytes();
        let i = pos % bytes.len();
        bytes[i] = byte;
        if let Ok(s) = String::from_utf8(bytes) {
            if let Ok(parsed) = json::parse(&s) {
                let _ = WorldState::from_canon(&parsed);
            }
        }
    }
}

fn world_json_text() -> String {
    use std::sync::OnceLock;
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| world_json().to_canonical_string()).clone()
}
