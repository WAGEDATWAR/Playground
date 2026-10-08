//! The town journal for the UI (milestone 1.7). The journal itself lives in the world (`pg_core::journal`,
//! D-058): the core writes it as events happen, it is saved, hashed and replayed with the world. This turns
//! it into what the journal window shows.

use pg_core::world::WorldState;
use pg_ui_model::journal::JournalEntry;

/// The world's journal as window entries, oldest first.
pub fn view(world: &WorldState) -> Vec<JournalEntry> {
    world
        .journal
        .entries()
        .iter()
        .map(|e| JournalEntry {
            tick: e.tick,
            day: e.day(),
            minute_of_day: e.minute_of_day(),
            kind: e.kind.clone(),
            args: e.args.clone(),
            weight: e.weight,
        })
        .collect()
}

/// A cheap fingerprint of the journal (how many entries and the newest tick), to notice a change.
pub fn stamp(world: &WorldState) -> (usize, Option<u64>) {
    (
        world.journal.len(),
        world.journal.entries().last().map(|e| e.tick),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
    use pg_core::canon::ToCanon;
    use pg_core::commands::Command;
    use pg_core::input::SimInput;
    use pg_core::pipeline::Pipeline;
    use pg_core::sim::Sim;
    use std::sync::Arc;

    fn town(seed: &str) -> (Sim, Arc<ContentSet>) {
        let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
        let content =
            Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
        let mut sim =
            Sim::new(WorldState::new("Town", seed), Pipeline::new()).with_content(content.clone());
        sim.submit(
            0,
            SimInput::Command {
                actor: None,
                cmd: Command::GenerateTown {
                    w: 48,
                    h: 36,
                    water: 15,
                    residents: 12,
                    tone: "standard".into(),
                },
            },
        )
        .unwrap();
        (sim, content)
    }

    #[test]
    fn a_real_town_writes_a_readable_journal_that_survives_saving_and_replay() {
        let (mut sim, content) = town("journal-real");
        sim.run_ticks(4 * 14_400).unwrap();
        let w = sim.world();
        assert!(
            w.journal.len() >= 5,
            "{} entries in four days",
            w.journal.len()
        );
        // Every entry's sentence exists in the string table and has no unfilled placeholder.
        let strings = content.strings();
        for e in view(w) {
            let key = format!("journal.{}", e.kind);
            assert!(strings.get("en", &key).is_some(), "{key}");
            let text = pg_ui_model::journal::sentence(&e, &|k, a| strings.text("en", k, a));
            assert!(!text.contains('{') && !text.starts_with('['), "{text}");
            assert!(e.args.iter().all(|(_, v)| !v.is_empty()), "{e:?}");
        }
        // Saved and loaded, the journal is the same; and a second run of the same town writes the same one.
        let text = w.to_canon().to_canonical_string();
        let back = WorldState::from_canon(&pg_core::canon::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back.journal, w.journal);
        let (mut again, _) = town("journal-real");
        again.run_ticks(4 * 14_400).unwrap();
        assert_eq!(again.world().journal, w.journal);
        assert_eq!(stamp(w), stamp(again.world()));
    }
}
