//! Turning the simulation's events into the town journal (milestone 1.7, suggestion S-058).
//!
//! Only what is worth noticing makes it in: a relationship changing label, a resident collapsing or getting
//! back on their feet, a need becoming critical, a conversation that clearly brought two people together or
//! pushed them apart. Entries are plain data (the sentence comes from the string table when the window is
//! drawn), kept to the newest [`KEPT`]. The journal is derived from events as they happen; it is not saved
//! with the world.

use pg_core::canon::Canon;
use pg_core::id::EntityId;
use pg_core::pipeline::Event;
use pg_core::time::{TICKS_PER_DAY, TICKS_PER_GAME_MINUTE};
use pg_core::world::WorldState;
use pg_ui_model::journal::JournalEntry;
use std::collections::VecDeque;

/// Entries kept.
pub const KEPT: usize = 120;

/// A conversation moves the relationship at least this much to be written down.
const WARM_DELTA: i64 = 50;
const SOUR_DELTA: i64 = -40;

#[derive(Clone, Debug, Default)]
pub struct Journal {
    entries: VecDeque<JournalEntry>,
}

fn text<'a>(e: &'a Event, key: &str) -> Option<&'a str> {
    e.detail.get(key).and_then(Canon::as_str)
}

fn number(e: &Event, key: &str) -> Option<i64> {
    e.detail.get(key).and_then(Canon::as_i64)
}

fn pawn_id(e: &Event, key: &str) -> Option<EntityId> {
    text(e, key)?.parse().ok()
}

impl Journal {
    pub fn new() -> Journal {
        Journal::default()
    }

    pub fn entries(&self) -> Vec<JournalEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn push(&mut self, e: &Event, kind: &str, weight: u8, args: &[(&str, String)]) {
        self.entries.push_back(JournalEntry {
            tick: e.tick,
            day: e.tick / TICKS_PER_DAY,
            minute_of_day: u32::try_from((e.tick % TICKS_PER_DAY) / TICKS_PER_GAME_MINUTE)
                .unwrap_or(0),
            kind: kind.to_owned(),
            args: args
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            weight,
        });
        while self.entries.len() > KEPT {
            self.entries.pop_front();
        }
    }

    /// Looks at one tick's events. Returns whether anything was added.
    pub fn observe(&mut self, events: &[Event], world: &WorldState) -> bool {
        let before = self.entries.len();
        let last_before = self.entries.back().map(|e| e.tick);
        let name = |id: Option<EntityId>| {
            id.and_then(|i| world.pawns.get(i))
                .map_or_else(String::new, |p| p.name.clone())
        };
        for e in events {
            match e.kind.as_str() {
                "relationship.label_changed" => {
                    let (a, b) = (name(pawn_id(e, "a")), name(pawn_id(e, "b")));
                    let label = text(e, "to").unwrap_or_default();
                    self.push(
                        e,
                        "label",
                        2,
                        &[
                            ("a", a),
                            ("b", b),
                            ("label_key", format!("relationship.{label}")),
                        ],
                    );
                }
                "pawn.collapsed" => {
                    let who = name(pawn_id(e, "pawn"));
                    self.push(e, "collapsed", 3, &[("pawn", who)]);
                }
                "pawn.recovered" => {
                    let who = name(pawn_id(e, "pawn"));
                    self.push(e, "recovered", 2, &[("pawn", who)]);
                }
                "need.critical" => {
                    let who = name(pawn_id(e, "pawn"));
                    let need = text(e, "need").unwrap_or_default();
                    self.push(
                        e,
                        "critical",
                        2,
                        &[("pawn", who), ("need_key", format!("need.{need}"))],
                    );
                }
                "conversation.closed" => {
                    let delta = number(e, "delta").unwrap_or(0);
                    let kind = if delta >= WARM_DELTA {
                        "talk_warm"
                    } else if delta <= SOUR_DELTA {
                        "talk_sour"
                    } else {
                        continue;
                    };
                    let (a, b) = (name(pawn_id(e, "a")), name(pawn_id(e, "b")));
                    let topic = text(e, "topic").unwrap_or_default();
                    self.push(
                        e,
                        kind,
                        1,
                        &[
                            ("a", a),
                            ("b", b),
                            ("topic_key", format!("memory.phrase.{topic}")),
                        ],
                    );
                }
                _ => {}
            }
        }
        self.entries.len() != before || self.entries.back().map(|e| e.tick) != last_before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::id::Kind;
    use pg_core::map::{MapKind, Tile};

    fn world() -> (WorldState, EntityId, EntityId) {
        let mut w = WorldState::new("J", "journal");
        let m = w.create_map(MapKind::Overworld, 8, 8).unwrap();
        let a = w.spawn_pawn("Ivan Bauer", m, Tile::new(1, 1)).unwrap();
        let b = w.spawn_pawn("Tess Fischer", m, Tile::new(2, 1)).unwrap();
        (w, a, b)
    }

    fn ev(tick: u64, kind: &str, detail: &[(&str, Canon)]) -> Event {
        Event {
            tick,
            kind: kind.into(),
            detail: Canon::map(detail.iter().map(|(k, v)| (*k, v.clone()))),
        }
    }

    fn id(i: EntityId) -> Canon {
        Canon::str(i.to_string())
    }

    #[test]
    fn notable_events_become_entries_and_routine_ones_do_not() {
        let (w, a, b) = world();
        let mut j = Journal::new();
        let events = [
            ev(
                100,
                "relationship.label_changed",
                &[
                    ("a", id(a)),
                    ("b", id(b)),
                    ("from", Canon::str("acquaintance")),
                    ("to", Canon::str("friendly")),
                    ("affinity", Canon::Int(320)),
                ],
            ),
            ev(110, "pawn.collapsed", &[("pawn", id(a))]),
            ev(
                120,
                "need.critical",
                &[
                    ("pawn", id(b)),
                    ("need", Canon::str("energy")),
                    ("value", Canon::Int(90)),
                ],
            ),
            ev(
                130,
                "conversation.closed",
                &[
                    ("a", id(a)),
                    ("b", id(b)),
                    ("topic", Canon::str("food")),
                    ("tone", Canon::str("warm")),
                    ("delta", Canon::Int(60)),
                    ("affinity", Canon::Int(400)),
                ],
            ),
            ev(
                140,
                "conversation.closed",
                &[
                    ("a", id(a)),
                    ("b", id(b)),
                    ("topic", Canon::str("work")),
                    ("tone", Canon::str("friendly")),
                    ("delta", Canon::Int(20)),
                    ("affinity", Canon::Int(420)),
                ],
            ),
            ev(
                150,
                "conversation.closed",
                &[
                    ("a", id(a)),
                    ("b", id(b)),
                    ("topic", Canon::str("complaint")),
                    ("tone", Canon::str("curt")),
                    ("delta", Canon::Int(-45)),
                    ("affinity", Canon::Int(375)),
                ],
            ),
            ev(160, "move.arrived", &[]),
            ev(170, "pawn.recovered", &[("pawn", id(a))]),
        ];
        assert!(j.observe(&events, &w));
        let kinds: Vec<String> = j.entries().iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                "label",
                "collapsed",
                "critical",
                "talk_warm",
                "talk_sour",
                "recovered"
            ]
        );
        let first = &j.entries()[0];
        assert_eq!(
            first.args,
            vec![
                ("a".to_string(), "Ivan Bauer".to_string()),
                ("b".to_string(), "Tess Fischer".to_string()),
                ("label_key".to_string(), "relationship.friendly".to_string())
            ]
        );
        assert_eq!((first.day, first.minute_of_day), (0, 100 / 10));
        assert!(!j.observe(&[ev(180, "move.arrived", &[])], &w));
    }

    #[test]
    fn the_journal_keeps_only_the_newest_entries() {
        let (w, a, _) = world();
        let mut j = Journal::new();
        for t in 0..(KEPT as u64 + 30) {
            j.observe(&[ev(t, "pawn.collapsed", &[("pawn", id(a))])], &w);
        }
        assert_eq!(j.len(), KEPT);
        assert_eq!(j.entries()[0].tick, 30);
        let _ = Kind::Pawn;
    }

    #[test]
    fn a_real_town_writes_a_readable_journal_over_a_few_days() {
        use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
        use pg_core::commands::Command;
        use pg_core::input::SimInput;
        use pg_core::pipeline::Pipeline;
        use pg_core::sim::Sim;
        use std::sync::Arc;
        let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
        let content =
            Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
        let mut sim = Sim::new(WorldState::new("Town", "journal-real"), Pipeline::new())
            .with_content(content.clone());
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
        let mut j = Journal::new();
        for _ in 0..4 * 14_400 {
            let r = sim.step().unwrap();
            j.observe(&r.events, sim.world());
        }
        assert!(j.len() >= 5, "{} entries in four days", j.len());
        assert!(j.len() <= KEPT);
        // Every entry's sentence exists in the string table and has no unfilled placeholder.
        let strings = content.strings();
        for e in j.entries() {
            let key = format!("journal.{}", e.kind);
            assert!(strings.get("en", &key).is_some(), "{key}");
            let text = pg_ui_model::journal::sentence(&e, &|k, a| strings.text("en", k, a));
            assert!(!text.contains('{') && !text.starts_with('['), "{text}");
            assert!(e.args.iter().all(|(_, v)| !v.is_empty()), "{e:?}");
        }
    }
}
