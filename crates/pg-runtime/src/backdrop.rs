//! The small, paused town behind the main menu (Stage 1, milestone 1.7; suggestion S-039).
//!
//! A tiny seeded town is generated and run for a day and a bit, so the first screen already shows the game:
//! streets, buildings and residents out and about. It is built once, on a background thread, from the loaded
//! content; it never touches a saved world, and it is dropped when a real world opens.

use crate::control::RunState;
use crate::snapshot::RenderSnapshot;
use pg_content::ContentSet;
use pg_core::commands::Command;
use pg_core::input::SimInput;
use pg_core::pipeline::Pipeline;
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use std::sync::Arc;

/// The seed of the menu town: the same picture every launch.
pub const SEED: &str = "main-menu";

/// Ticks to run before the picture is taken: a day and a bit, so it is mid-morning.
pub const TICKS: u64 = 14_400 + 5_400;

/// Builds the backdrop's snapshot, or `None` if the content has no town generator.
pub fn build(content: &Arc<ContentSet>) -> Option<RenderSnapshot> {
    content.game().worldgen.as_ref()?;
    let mut sim = Sim::new(WorldState::new("Backdrop", SEED), Pipeline::new())
        .with_content(Arc::clone(content));
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::GenerateTown {
                w: 40,
                h: 28,
                water: 18,
                residents: 10,
                tone: "standard".into(),
            },
        },
    )
    .ok()?;
    sim.run_ticks(TICKS).ok()?;
    let snap = RenderSnapshot::build(sim.world(), RunState::Paused, None, &[], "");
    (!snap.maps.is_empty()).then_some(snap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_content::{load_pack, ComponentRegistry, DirPack, Limits};

    #[test]
    fn the_backdrop_is_a_populated_town_and_the_same_every_time() {
        let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
        let content =
            Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
        let a = build(&content).expect("the base content can make a town");
        assert_eq!(a.run, RunState::Paused);
        assert_eq!(a.maps.len(), 1);
        assert_eq!((a.maps[0].width(), a.maps[0].height()), (40, 28));
        assert_eq!(a.pawns.len(), 10);
        assert_eq!(a.tick, TICKS);
        let b = build(&content).unwrap();
        let spots = |s: &RenderSnapshot| s.pawns.iter().map(|p| (p.id, p.tile)).collect::<Vec<_>>();
        assert_eq!(spots(&a), spots(&b));
    }
}
