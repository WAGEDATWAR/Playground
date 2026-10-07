//! Shared fixtures for this crate's tests.

use crate::compat::ContentRefRecord;
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::sim::Sim;
use pg_core::world::WorldState;

pub fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

/// A sim with a small generated town and four pawns, not yet run.
pub fn town_sim() -> Sim {
    let mut sim = Sim::with_dev_systems(WorldState::new("Kit Town", "kit-seed"));
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 24,
            h: 18,
            style: 1,
        }),
    )
    .unwrap();
    let map = EntityId::new(Kind::Map, 1);
    for i in 0..4 {
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
    sim
}

pub fn world_at(ticks: u64) -> WorldState {
    let mut sim = town_sim();
    sim.run_ticks(ticks).unwrap();
    sim.world().clone()
}

pub fn refs() -> Vec<ContentRefRecord> {
    vec![ContentRefRecord {
        pack_id: "base".into(),
        version: "0.1.0".into(),
        hash: "abcdef0123456789".into(),
    }]
}
