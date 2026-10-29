//! The New world screen's sample town (milestone 1.8b).
//!
//! Makes the town the chosen settings would make, with the same generator the real world uses, and draws it as a
//! small picture. Nothing is saved and no world is opened; it runs on a background thread.

use crate::thumbnail::thumbnail_rgb;
use pg_content::ContentSet;
use pg_core::commands::Command;
use pg_core::input::SimInput;
use pg_core::pipeline::Pipeline;
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use std::sync::Arc;

/// The picture of a town made from these settings as `(width, height, rgb)`, or `None` if the content has no
/// town generator or the generator refused the settings.
pub fn build(
    content: &Arc<ContentSet>,
    seed: &str,
    size: (i32, i32),
    residents: u32,
    water: u32,
    tone: &str,
) -> Option<(u32, u32, Vec<u8>)> {
    content.game().worldgen.as_ref()?;
    let mut sim = Sim::new(WorldState::new("Sample", seed), Pipeline::new())
        .with_content(Arc::clone(content));
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::GenerateTown {
                w: size.0,
                h: size.1,
                water,
                residents,
                tone: tone.to_owned(),
            },
        },
    )
    .ok()?;
    sim.run_ticks(1).ok()?;
    thumbnail_rgb(sim.world())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_content::{load_pack, ComponentRegistry, DirPack, Limits};

    fn content() -> Arc<ContentSet> {
        let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
        Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap())
    }

    #[test]
    fn the_sample_is_a_picture_of_the_town_the_seed_makes_and_the_same_every_time() {
        let c = content();
        let a = build(&c, "abc", (48, 36), 12, 18, "standard").expect("a sample");
        assert_eq!((a.0, a.1), (48, 36));
        assert_eq!(a.2.len(), 48 * 36 * 3);
        assert_eq!(a, build(&c, "abc", (48, 36), 12, 18, "standard").unwrap());
        let other = build(&c, "abd", (48, 36), 12, 18, "standard").unwrap();
        assert_ne!(a.2, other.2, "a different seed makes a different town");
    }
}
