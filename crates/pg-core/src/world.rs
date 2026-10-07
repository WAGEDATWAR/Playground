//! `WorldState` (Blueprint §4.5). Milestone 0.2 holds only what the clock, the input log and the
//! tick pipeline need; the entity tables (pawns, objects, maps, …) arrive with later milestones.
//!
//! Every table has a canonical form, and the state hash is the combination of per-table hashes, so a
//! divergence between two runs can be localized to the first table that differs.

use crate::canon::{Canon, ToCanon};
use crate::hash::{combine_table_hashes, hash_value, StateHash};
use crate::id::IdCounters;
use crate::rng::Seed;
use crate::time::{Clock, SlotMinutes};
use std::collections::BTreeMap;

/// Bumped whenever the saved shape of `WorldState` changes (migrations hang off this, §13.5).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldMeta {
    pub name: String,
    /// The text the player typed (or was generated); the numeric seed derives from it.
    pub seed_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldSettings {
    pub slot_minutes: SlotMinutes,
}

impl Default for WorldSettings {
    fn default() -> Self {
        WorldSettings {
            slot_minutes: SlotMinutes::DEFAULT,
        }
    }
}

/// **Dev scaffolding.** A tiny piece of state mutated by the dev systems and the `DevNudge` command
/// so replay, hashing and snapshot tests are meaningful before real simulation systems exist.
/// Removed when the Stage 1 systems land (see `docs/TODO.md`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub value: i64,
    pub minutes: u64,
    pub days: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldState {
    pub schema: u32,
    pub meta: WorldMeta,
    pub settings: WorldSettings,
    pub clock: Clock,
    pub id_counters: IdCounters,
    /// Sequentially consumed RNG streams: stream name -> next counter (§5.1).
    pub rng_counters: BTreeMap<String, u32>,
    pub probe: Probe,
}

impl WorldState {
    pub fn new(name: impl Into<String>, seed_text: impl Into<String>) -> WorldState {
        WorldState {
            schema: SCHEMA_VERSION,
            meta: WorldMeta {
                name: name.into(),
                seed_text: seed_text.into(),
            },
            settings: WorldSettings::default(),
            clock: Clock::new(),
            id_counters: IdCounters::new(),
            rng_counters: BTreeMap::new(),
            probe: Probe::default(),
        }
    }

    /// The numeric world seed.
    pub fn seed(&self) -> Seed {
        Seed::from_text(&self.meta.seed_text)
    }

    /// Takes the next counter for a sequentially consumed stream, or `None` if it is exhausted.
    pub fn next_stream_counter(&mut self, stream: &str) -> Option<u32> {
        let slot = self.rng_counters.entry(stream.to_owned()).or_insert(0);
        let current = *slot;
        *slot = slot.checked_add(1)?;
        Some(current)
    }

    /// Per-table hashes in a fixed order.
    pub fn table_hashes(&self) -> Vec<(&'static str, StateHash)> {
        vec![
            ("clock", hash_value(&self.clock)),
            ("id_counters", hash_value(&self.id_counters)),
            ("meta", hash_value(&self.meta)),
            ("probe", hash_value(&self.probe)),
            ("rng_counters", hash_value(&self.rng_counters)),
            ("settings", hash_value(&self.settings)),
        ]
    }

    /// The combined state hash (Blueprint §5.4).
    pub fn state_hash(&self) -> StateHash {
        combine_table_hashes(self.table_hashes())
    }
}

impl ToCanon for WorldMeta {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("name", self.name.to_canon()),
            ("seed_text", self.seed_text.to_canon()),
        ])
    }
}

impl ToCanon for WorldSettings {
    fn to_canon(&self) -> Canon {
        Canon::map([("slot_minutes", self.slot_minutes.get().to_canon())])
    }
}

impl ToCanon for Probe {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("days", self.days.to_canon()),
            ("minutes", self.minutes.to_canon()),
            ("value", self.value.to_canon()),
        ])
    }
}

impl ToCanon for BTreeMap<String, u32> {
    fn to_canon(&self) -> Canon {
        Canon::Map(
            self.iter()
                .map(|(k, v)| (k.clone(), v.to_canon()))
                .collect(),
        )
    }
}

impl ToCanon for WorldState {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("schema", self.schema.to_canon()),
            ("meta", self.meta.to_canon()),
            ("settings", self.settings.to_canon()),
            ("clock", self.clock.to_canon()),
            ("id_counters", self.id_counters.to_canon()),
            ("rng_counters", self.rng_counters.to_canon()),
            ("probe", self.probe.to_canon()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_worlds_hash_identically_and_any_change_shows() {
        let a = WorldState::new("Town", "seed");
        let mut b = WorldState::new("Town", "seed");
        assert_eq!(a.state_hash(), b.state_hash());
        b.probe.value = 1;
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn the_table_hash_localizes_a_change() {
        let a = WorldState::new("Town", "seed");
        let mut b = a.clone();
        b.settings.slot_minutes = SlotMinutes::new(60).unwrap();
        let diff: Vec<_> = a
            .table_hashes()
            .into_iter()
            .zip(b.table_hashes())
            .filter(|(x, y)| x.1 != y.1)
            .map(|(x, _)| x.0)
            .collect();
        assert_eq!(diff, vec!["settings"]);
    }

    #[test]
    fn seed_comes_from_the_text() {
        assert_eq!(WorldState::new("a", "x").seed(), Seed::from_text("x"));
        assert_ne!(
            WorldState::new("a", "x").seed(),
            WorldState::new("a", "y").seed()
        );
    }

    #[test]
    fn stream_counters_advance_per_stream() {
        let mut w = WorldState::new("a", "x");
        assert_eq!(w.next_stream_counter("s1"), Some(0));
        assert_eq!(w.next_stream_counter("s1"), Some(1));
        assert_eq!(w.next_stream_counter("s2"), Some(0));
        w.rng_counters.insert("full".into(), u32::MAX);
        assert_eq!(w.next_stream_counter("full"), None);
    }

    #[test]
    fn canonical_form_is_stable() {
        let w = WorldState::new("Town", "seed");
        assert_eq!(
            w.to_canon().to_canonical_string(),
            r#"{"clock":{"tick":0},"id_counters":{},"meta":{"name":"Town","seed_text":"seed"},"probe":{"days":0,"minutes":0,"value":0},"rng_counters":{},"schema":1,"settings":{"slot_minutes":30}}"#
        );
    }
}
