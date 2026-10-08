//! Pinned save fixtures (Blueprint §13.5: "every migration ships with a fixture").
//!
//! `fixtures/saves/world-v3.json` is a world written by schema 3 and never changes. Every future build must
//! keep loading it through the migration chain and reach the hash recorded in `world-v3.hash`, which is the
//! hash of the *migrated* world and is re-recorded (with a decision entry) only when a later schema changes
//! what the state hash covers. `world-v4.json` is the same for schema 4, written in that schema directly; if a schema change breaks
//! that, the change needs a migration, and the fixture for the new schema is added next to this one rather
//! than replacing it.
//!
//! Regenerate the v4 fixture deliberately with:
//! `cargo test -p pg-persist regenerate_v4_fixture -- --ignored`

use crate::migrate::Migrations;
use pg_core::canon::{json, ToCanon};
use pg_core::world::WorldState;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/saves")
}

#[test]
fn the_v3_fixture_still_loads_to_its_recorded_hash() {
    let text =
        std::fs::read_to_string(dir().join("world-v3.json")).expect("fixtures/saves/world-v3.json");
    let expected_hash =
        std::fs::read_to_string(dir().join("world-v3.hash")).expect("world-v3.hash");
    let raw = json::parse(&text).unwrap();
    let schema = raw.get("schema").and_then(|s| s.as_i64()).unwrap();
    assert_eq!(schema, 3, "the fixture is a schema 3 save");
    let migrated = Migrations::builtin().migrate(raw, 3).unwrap();
    let world = WorldState::from_canon(&migrated).unwrap();
    assert_eq!(world.state_hash().to_hex(), expected_hash.trim());
    // Writing the migrated world back gives the migrated bytes: the canonical form has not drifted.
    assert_eq!(
        world.to_canon().to_canonical_string(),
        migrated.to_canonical_string()
    );
    // And it is a living world, not an empty one.
    assert!(world.pawns.len() >= 4 && world.pawns.iter().any(|(_, p)| p.schedule.is_some()));
}

#[test]
fn the_v4_fixture_loads_directly_to_its_recorded_hash_and_writes_back_identically() {
    let text =
        std::fs::read_to_string(dir().join("world-v4.json")).expect("fixtures/saves/world-v4.json");
    let expected_hash =
        std::fs::read_to_string(dir().join("world-v4.hash")).expect("world-v4.hash");
    let raw = json::parse(&text).unwrap();
    assert_eq!(raw.get("schema").and_then(|s| s.as_i64()), Some(4));
    let migrated = Migrations::builtin().migrate(raw, 4).unwrap();
    let world = WorldState::from_canon(&migrated).unwrap();
    assert_eq!(world.state_hash().to_hex(), expected_hash.trim());
    assert_eq!(world.to_canon().to_canonical_string(), text.trim_end());
    assert!(world.pawns.len() >= 4);
    assert!(!world.town.buildings.is_empty() && !world.households.is_empty());
    assert!(world.pawns.iter().all(|(_, p)| p.home_tile.is_some()));
}

#[test]
#[ignore = "writes the fixture; run on purpose when the schema changes"]
fn regenerate_v4_fixture() {
    use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
    use pg_core::commands::Command;
    use pg_core::input::SimInput;
    use pg_core::sim::Sim;
    // A generated town that has lived for a day and a half: residents, households, relationships,
    // memories, needs, moods, schedules and the town layout are all in it.
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/base");
    let pack = load_pack(&DirPack::new(base), &Limits::default()).unwrap();
    let content =
        std::sync::Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
    let mut sim =
        Sim::with_dev_systems(WorldState::new("Fixture Town", "fixture-v4")).with_content(content);
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::GenerateTown {
                w: 48,
                h: 36,
                water: 15,
                residents: 10,
                tone: "standard".into(),
            },
        },
    )
    .unwrap();
    sim.run_ticks(21_000).unwrap();
    let world = sim.world().clone();
    std::fs::create_dir_all(dir()).unwrap();
    std::fs::write(
        dir().join("world-v4.json"),
        world.to_canon().to_canonical_string()
            + "
",
    )
    .unwrap();
    std::fs::write(
        dir().join("world-v4.hash"),
        world.state_hash().to_hex()
            + "
",
    )
    .unwrap();
}
