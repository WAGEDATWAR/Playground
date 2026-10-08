//! The town journal as part of the world (Stage 1, milestone 1.7; suggestion S-058, decision D-056 and D-058).
//!
//! A short, bounded list of what happened that is worth noticing: a relationship changing label, a resident
//! collapsing or recovering, a need becoming critical, a conversation that clearly brought two people
//! together or pushed them apart. Entries are written by the core as the events happen (so replays make the
//! same journal), are hashed and saved with the world, and hold names rather than ids so they stay readable
//! when a resident is gone. The sentence itself comes from the string table when the journal is shown:
//! `journal.<kind>` filled with the arguments, where an argument named `x_key` holds a string key to look up
//! first.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::pipeline::Event;
use crate::read::{ReadError, Reader};
use crate::time::{TICKS_PER_DAY, TICKS_PER_GAME_MINUTE};
use crate::world::WorldState;
use std::collections::BTreeMap;

/// Entries kept; the oldest are dropped.
pub const KEPT: usize = 120;

/// A conversation moves the relationship at least this much to be written down.
const WARM_DELTA: i64 = 50;
const SOUR_DELTA: i64 = -40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub tick: u64,
    /// Selects the sentence (`journal.<kind>`).
    pub kind: String,
    /// 1 routine, 2 notable, 3 serious.
    pub weight: u8,
    /// Fill-ins, in name order.
    pub args: Vec<(String, String)>,
}

impl JournalEntry {
    pub fn day(&self) -> u64 {
        self.tick / TICKS_PER_DAY
    }

    pub fn minute_of_day(&self) -> u32 {
        u32::try_from((self.tick % TICKS_PER_DAY) / TICKS_PER_GAME_MINUTE).unwrap_or(0)
    }
}

impl ToCanon for JournalEntry {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("tick", self.tick.to_canon()),
            ("kind", Canon::str(self.kind.clone())),
            ("weight", self.weight.to_canon()),
            (
                "args",
                Canon::Map(
                    self.args
                        .iter()
                        .map(|(k, v)| (k.clone(), Canon::str(v.clone())))
                        .collect::<BTreeMap<_, _>>(),
                ),
            ),
        ])
    }
}

impl JournalEntry {
    pub fn from_reader(r: Reader<'_>) -> Result<JournalEntry, ReadError> {
        r.only(&["tick", "kind", "weight", "args"])?;
        let mut args = Vec::new();
        for (k, v) in r.child("args")?.reader().entries()? {
            args.push((k, v.reader().str()?.to_owned()));
        }
        Ok(JournalEntry {
            tick: r.child("tick")?.reader().u64()?,
            kind: r.child("kind")?.reader().str()?.to_owned(),
            weight: r.child("weight")?.reader().u8()?,
            args,
        })
    }
}

/// The journal: oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    entries: Vec<JournalEntry>,
}

impl Journal {
    pub fn new() -> Journal {
        Journal::default()
    }

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn push(&mut self, e: JournalEntry) {
        self.entries.push(e);
        if self.entries.len() > KEPT {
            self.entries.remove(0);
        }
    }
}

impl ToCanon for Journal {
    fn to_canon(&self) -> Canon {
        Canon::List(self.entries.iter().map(ToCanon::to_canon).collect())
    }
}

impl Journal {
    pub fn from_reader(r: Reader<'_>) -> Result<Journal, ReadError> {
        let mut entries = r
            .list()?
            .iter()
            .map(|c| JournalEntry::from_reader(c.reader()))
            .collect::<Result<Vec<_>, _>>()?;
        if entries.len() > KEPT {
            entries.drain(..entries.len() - KEPT);
        }
        Ok(Journal { entries })
    }
}

fn text<'a>(e: &'a Event, key: &str) -> Option<&'a str> {
    e.detail.get(key).and_then(Canon::as_str)
}

fn pawn_id(e: &Event, key: &str) -> Option<EntityId> {
    text(e, key)?.parse().ok()
}

/// Writes the journal entry an event deserves, if any.
pub fn record(world: &mut WorldState, e: &Event) {
    let name = |w: &WorldState, id: Option<EntityId>| {
        id.and_then(|i| w.pawns.get(i))
            .map_or_else(String::new, |p| p.name.clone())
    };
    let entry = |kind: &str, weight: u8, args: &[(&str, String)]| JournalEntry {
        tick: e.tick,
        kind: kind.to_owned(),
        weight,
        args: {
            let mut a: Vec<(String, String)> = args
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect();
            a.sort();
            a
        },
    };
    let made = match e.kind.as_str() {
        "relationship.label_changed" => {
            let label = text(e, "to").unwrap_or_default();
            Some(entry(
                "label",
                2,
                &[
                    ("a", name(world, pawn_id(e, "a"))),
                    ("b", name(world, pawn_id(e, "b"))),
                    ("label_key", format!("relationship.{label}")),
                ],
            ))
        }
        "pawn.collapsed" => Some(entry(
            "collapsed",
            3,
            &[("pawn", name(world, pawn_id(e, "pawn")))],
        )),
        "pawn.recovered" => Some(entry(
            "recovered",
            2,
            &[("pawn", name(world, pawn_id(e, "pawn")))],
        )),
        "need.critical" => {
            let need = text(e, "need").unwrap_or_default();
            Some(entry(
                "critical",
                2,
                &[
                    ("pawn", name(world, pawn_id(e, "pawn"))),
                    ("need_key", format!("need.{need}")),
                ],
            ))
        }
        "conversation.closed" => {
            let delta = e.detail.get("delta").and_then(Canon::as_i64).unwrap_or(0);
            let kind = if delta >= WARM_DELTA {
                Some("talk_warm")
            } else if delta <= SOUR_DELTA {
                Some("talk_sour")
            } else {
                None
            };
            kind.map(|k| {
                let topic = text(e, "topic").unwrap_or_default();
                entry(
                    k,
                    1,
                    &[
                        ("a", name(world, pawn_id(e, "a"))),
                        ("b", name(world, pawn_id(e, "b"))),
                        ("topic_key", format!("memory.phrase.{topic}")),
                    ],
                )
            })
        }
        _ => None,
    };
    if let Some(m) = made {
        world.journal.push(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MapKind, Tile};

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
        let (mut w, a, b) = world();
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
        for e in &events {
            record(&mut w, e);
        }
        let kinds: Vec<&str> = w
            .journal
            .entries()
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
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
        let first = &w.journal.entries()[0];
        assert_eq!(
            first.args,
            vec![
                ("a".to_string(), "Ivan Bauer".to_string()),
                ("b".to_string(), "Tess Fischer".to_string()),
                ("label_key".to_string(), "relationship.friendly".to_string())
            ]
        );
        assert_eq!((first.day(), first.minute_of_day()), (0, 10));
    }

    #[test]
    fn the_journal_keeps_only_the_newest_entries_and_is_part_of_the_hash_and_the_saved_world() {
        let (mut w, a, _) = world();
        let before = w.state_hash();
        for t in 0..(KEPT as u64 + 30) {
            record(&mut w, &ev(t, "pawn.collapsed", &[("pawn", id(a))]));
        }
        assert_eq!(w.journal.len(), KEPT);
        assert_eq!(w.journal.entries()[0].tick, 30);
        assert_ne!(w.state_hash(), before, "the journal is world state");
        // It survives a save and load, bit for bit.
        let text = w.to_canon().to_canonical_string();
        let back = WorldState::from_canon(&crate::canon::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back.journal, w.journal);
        assert_eq!(back.state_hash(), w.state_hash());
    }

    #[test]
    fn a_damaged_entry_is_refused_with_its_path() {
        let c = crate::canon::json::parse(r#"[{"tick":1,"kind":"label","weight":9999,"args":{}}]"#)
            .unwrap();
        assert!(Journal::from_reader(Reader::new(&c, "journal")).is_err());
    }
}
