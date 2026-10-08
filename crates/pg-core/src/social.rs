//! Residents' social state (Blueprint §4.6, §8.3, §8.4): memories, relationships, households and the
//! occupation a pawn has. Milestone 1.0 gives them their shape, saved form and hashing; the systems that
//! create and age them arrive in 1.1 to 1.4.

use crate::canon::{Canon, ToCanon};
use crate::id::EntityId;
use crate::read::{ReadError, Reader};
use std::collections::BTreeMap;

/// What a pawn does for a living: an archetype (a template id from the game data) and the seeded
/// variation that makes two barista schedules differ a little.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occupation {
    pub template: String,
    /// A seeded value in `0..=999` the schedule systems derive their small shifts from.
    pub variation: i32,
}

impl ToCanon for Occupation {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("template", Canon::str(self.template.clone())),
            ("variation", self.variation.to_canon()),
        ])
    }
}

impl Occupation {
    pub fn from_reader(r: Reader<'_>) -> Result<Occupation, ReadError> {
        r.only(&["template", "variation"])?;
        Ok(Occupation {
            template: r.child("template")?.reader().str()?.to_owned(),
            variation: r.child("variation")?.reader().i32()?,
        })
    }
}

/// How important a memory is: severity (1 to 5) scaled by how strongly it affected the pawn.
pub fn importance(severity: u8, impact: i32) -> i32 {
    i32::from(severity).saturating_mul(10i32.saturating_add(impact.saturating_abs()))
}

/// Retention lost per day: low-severity memories fade faster. Never less than 1.
pub fn decay_per_day(decay_k: i32, severity: u8) -> i32 {
    (decay_k / i32::from(severity).max(1)).max(1)
}

/// One thing a pawn remembers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    pub id: EntityId,
    /// A memory type from the game data (`conversation`, `shared_event`, ...).
    pub ty: String,
    pub tick: u64,
    /// The other people involved, in id order.
    pub participants: Vec<EntityId>,
    /// 1 to 5.
    pub severity: u8,
    /// Signed: how much better or worse it made the pawn feel.
    pub impact: i32,
    pub importance: i32,
    pub decay_per_day: i32,
    /// 1000 when made; at 0 a minor memory is forgotten.
    pub retention: i32,
    pub topic: Option<String>,
    /// String-table key of the one-line summary the inspector shows (with `{other}` filled in).
    pub summary_key: Option<String>,
}

impl Memory {
    /// A new memory with its derived fields computed.
    pub fn new(
        id: EntityId,
        ty: &str,
        tick: u64,
        participants: Vec<EntityId>,
        severity: u8,
        impact: i32,
        decay_k: i32,
    ) -> Memory {
        let severity = severity.clamp(1, 5);
        let mut participants = participants;
        participants.sort();
        participants.dedup();
        Memory {
            id,
            ty: ty.to_owned(),
            tick,
            participants,
            severity,
            impact,
            importance: importance(severity, impact),
            decay_per_day: decay_per_day(decay_k, severity),
            retention: 1000,
            topic: None,
            summary_key: None,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.impact > 0
    }

    pub fn is_negative(&self) -> bool {
        self.impact < 0
    }
}

impl ToCanon for Memory {
    fn to_canon(&self) -> Canon {
        let opt = |s: &Option<String>| s.as_ref().map_or(Canon::Null, |s| Canon::str(s.clone()));
        Canon::map([
            ("id", self.id.to_canon()),
            ("type", Canon::str(self.ty.clone())),
            ("tick", self.tick.to_canon()),
            (
                "participants",
                Canon::List(self.participants.iter().map(ToCanon::to_canon).collect()),
            ),
            ("severity", self.severity.to_canon()),
            ("impact", self.impact.to_canon()),
            ("importance", self.importance.to_canon()),
            ("decay_per_day", self.decay_per_day.to_canon()),
            ("retention", self.retention.to_canon()),
            ("topic", opt(&self.topic)),
            ("summary_key", opt(&self.summary_key)),
        ])
    }
}

impl Memory {
    pub fn from_reader(r: Reader<'_>) -> Result<Memory, ReadError> {
        r.only(&[
            "id",
            "type",
            "tick",
            "participants",
            "severity",
            "impact",
            "importance",
            "decay_per_day",
            "retention",
            "topic",
            "summary_key",
        ])?;
        let text = |key: &str| -> Result<Option<String>, ReadError> {
            match r.maybe(key)? {
                Some(c) => Ok(Some(c.reader().str()?.to_owned())),
                None => Ok(None),
            }
        };
        let severity = r.child("severity")?.reader().u8()?;
        if !(1..=5).contains(&severity) {
            return Err(r.err("severity must be 1 to 5"));
        }
        Ok(Memory {
            id: r.child("id")?.reader().parse()?,
            ty: r.child("type")?.reader().str()?.to_owned(),
            tick: r.child("tick")?.reader().u64()?,
            participants: r
                .child("participants")?
                .reader()
                .list()?
                .iter()
                .map(|c| c.reader().parse())
                .collect::<Result<Vec<_>, _>>()?,
            severity,
            impact: r.child("impact")?.reader().i32()?,
            importance: r.child("importance")?.reader().i32()?,
            decay_per_day: r.child("decay_per_day")?.reader().i32()?,
            retention: r.child("retention")?.reader().i32()?,
            topic: text("topic")?,
            summary_key: text("summary_key")?,
        })
    }
}

/// The unordered pair a relationship belongs to, stored with the lower id first.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PairKey {
    pub a: EntityId,
    pub b: EntityId,
}

impl PairKey {
    /// `None` for a pawn paired with itself.
    pub fn new(x: EntityId, y: EntityId) -> Option<PairKey> {
        match x.cmp(&y) {
            std::cmp::Ordering::Less => Some(PairKey { a: x, b: y }),
            std::cmp::Ordering::Greater => Some(PairKey { a: y, b: x }),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// The text used as the row key: `pawn_1+pawn_2` (ids never contain `+`).
    pub fn text(&self) -> String {
        format!("{}+{}", self.a, self.b)
    }

    pub fn parse(s: &str) -> Option<PairKey> {
        let (x, y) = s.split_once('+')?;
        PairKey::new(x.parse().ok()?, y.parse().ok()?)
    }

    pub fn contains(&self, id: EntityId) -> bool {
        self.a == id || self.b == id
    }

    /// The other member of the pair.
    pub fn other(&self, id: EntityId) -> Option<EntityId> {
        if self.a == id {
            Some(self.b)
        } else if self.b == id {
            Some(self.a)
        } else {
            None
        }
    }
}

/// How two residents feel about each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub key: PairKey,
    /// -1000 to 1000.
    pub affinity: i32,
    /// Id of the label in the game data; changes only when affinity passes a threshold by the margin.
    pub label: String,
    pub last_interaction_tick: u64,
    /// The game day `day_change` was counted for.
    pub day: u64,
    /// Total affinity change so far on `day` (for the daily cap).
    pub day_change: i32,
}

impl ToCanon for Relationship {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("a", self.key.a.to_canon()),
            ("b", self.key.b.to_canon()),
            ("affinity", self.affinity.to_canon()),
            ("label", Canon::str(self.label.clone())),
            (
                "last_interaction_tick",
                self.last_interaction_tick.to_canon(),
            ),
            ("day", self.day.to_canon()),
            ("day_change", self.day_change.to_canon()),
        ])
    }
}

impl Relationship {
    pub fn from_reader(r: Reader<'_>) -> Result<Relationship, ReadError> {
        r.only(&[
            "a",
            "b",
            "affinity",
            "label",
            "last_interaction_tick",
            "day",
            "day_change",
        ])?;
        let key = PairKey::new(
            r.child("a")?.reader().parse()?,
            r.child("b")?.reader().parse()?,
        )
        .ok_or_else(|| r.err("a relationship needs two different pawns"))?;
        if key.a.to_string() != r.child("a")?.reader().str()? {
            return Err(r.err("the pair must list the lower id first"));
        }
        let affinity = r.child("affinity")?.reader().i32()?;
        if !(-1000..=1000).contains(&affinity) {
            return Err(r.err("affinity must be between -1000 and 1000"));
        }
        Ok(Relationship {
            key,
            affinity,
            label: r.child("label")?.reader().str()?.to_owned(),
            last_interaction_tick: r.child("last_interaction_tick")?.reader().u64()?,
            day: r.child("day")?.reader().u64()?,
            day_change: r.child("day_change")?.reader().i32()?,
        })
    }
}

/// Every relationship, keyed by pair, iterated in key order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Relationships {
    rows: BTreeMap<PairKey, Relationship>,
}

impl Relationships {
    pub fn new() -> Relationships {
        Relationships::default()
    }

    pub fn get(&self, x: EntityId, y: EntityId) -> Option<&Relationship> {
        self.rows.get(&PairKey::new(x, y)?)
    }

    pub fn get_mut(&mut self, x: EntityId, y: EntityId) -> Option<&mut Relationship> {
        self.rows.get_mut(&PairKey::new(x, y)?)
    }

    /// Inserts or replaces the relationship for its pair.
    pub fn set(&mut self, rel: Relationship) {
        self.rows.insert(rel.key, rel);
    }

    pub fn remove_pawn(&mut self, id: EntityId) {
        self.rows.retain(|k, _| !k.contains(id));
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Relationship> {
        self.rows.values()
    }

    /// The relationships `id` has, in pair order.
    pub fn of(&self, id: EntityId) -> impl Iterator<Item = &Relationship> {
        self.rows.values().filter(move |r| r.key.contains(id))
    }
}

impl ToCanon for Relationships {
    fn to_canon(&self) -> Canon {
        Canon::Map(
            self.rows
                .iter()
                .map(|(k, r)| (k.text(), r.to_canon()))
                .collect(),
        )
    }
}

/// People who live together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Household {
    pub id: EntityId,
    /// The family name the members share.
    pub name: String,
    /// Member pawn ids, ascending.
    pub members: Vec<EntityId>,
    /// The home object or plot, once worldgen assigns one (1.2).
    pub home: Option<EntityId>,
}

impl ToCanon for Household {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            ("name", Canon::str(self.name.clone())),
            (
                "members",
                Canon::List(self.members.iter().map(ToCanon::to_canon).collect()),
            ),
            ("home", self.home.map_or(Canon::Null, |h| h.to_canon())),
        ])
    }
}

impl Household {
    pub fn from_reader(r: Reader<'_>) -> Result<Household, ReadError> {
        r.only(&["id", "name", "members", "home"])?;
        let members: Vec<EntityId> = r
            .child("members")?
            .reader()
            .list()?
            .iter()
            .map(|c| c.reader().parse())
            .collect::<Result<_, _>>()?;
        if members.windows(2).any(|w| matches!(w, [a, b] if a >= b)) {
            return Err(r.err("members must be listed in ascending order without repeats"));
        }
        Ok(Household {
            id: r.child("id")?.reader().parse()?,
            name: r.child("name")?.reader().str()?.to_owned(),
            members,
            home: match r.maybe("home")? {
                Some(c) => Some(c.reader().parse()?),
                None => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::json;
    use crate::id::Kind;

    fn pawn(n: u32) -> EntityId {
        EntityId::new(Kind::Pawn, n)
    }

    #[test]
    fn importance_scales_with_severity_and_impact_and_decay_never_reaches_zero() {
        assert_eq!(importance(2, 0), 20);
        assert_eq!(importance(3, -40), 150);
        assert_eq!(importance(5, 100), 550);
        assert_eq!(decay_per_day(60, 2), 30);
        assert_eq!(decay_per_day(60, 5), 12);
        assert_eq!(decay_per_day(3, 5), 1, "a fade rate is never zero");
        assert_eq!(
            importance(5, i32::MIN),
            importance(5, i32::MAX),
            "no overflow"
        );
    }

    #[test]
    fn a_memory_derives_its_fields_sorts_participants_and_round_trips() {
        let mut m = Memory::new(
            EntityId::new(Kind::Memory, 1),
            "conversation",
            120,
            vec![pawn(5), pawn(2), pawn(5)],
            9,
            -30,
            60,
        );
        assert_eq!(m.severity, 5, "clamped");
        assert_eq!(m.participants, vec![pawn(2), pawn(5)]);
        assert_eq!(
            (m.importance, m.decay_per_day, m.retention),
            (200, 12, 1000)
        );
        assert!(m.is_negative() && !m.is_positive());
        m.topic = Some("weather".into());
        let text = m.to_canon().to_canonical_string();
        let back = Memory::from_reader(Reader::new(&json::parse(&text).unwrap(), "m")).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn a_damaged_memory_is_refused_with_a_path() {
        let c = json::parse(r#"{"id":"mem_1","type":"x","tick":0,"participants":[],"severity":9,"impact":0,"importance":0,"decay_per_day":1,"retention":1000,"topic":null,"summary_key":null}"#).unwrap();
        let e = Memory::from_reader(Reader::new(&c, "pawns.pawn_1.memories[0]")).unwrap_err();
        assert!(e.to_string().contains("severity must be 1 to 5"), "{e}");
    }

    #[test]
    fn pairs_are_unordered_keyed_low_id_first_and_never_with_oneself() {
        assert!(PairKey::new(pawn(1), pawn(1)).is_none());
        let k = PairKey::new(pawn(7), pawn(3)).unwrap();
        assert_eq!((k.a, k.b), (pawn(3), pawn(7)));
        assert_eq!(k.text(), "pawn_3+pawn_7");
        assert_eq!(PairKey::parse("pawn_3+pawn_7"), Some(k));
        assert_eq!(
            PairKey::parse("pawn_7+pawn_3"),
            Some(k),
            "either order parses"
        );
        assert_eq!(PairKey::parse("nonsense"), None);
        assert_eq!(k.other(pawn(3)), Some(pawn(7)));
        assert_eq!(k.other(pawn(9)), None);
    }

    fn rel(x: u32, y: u32, affinity: i32) -> Relationship {
        Relationship {
            key: PairKey::new(pawn(x), pawn(y)).unwrap(),
            affinity,
            label: "friend".into(),
            last_interaction_tick: 10,
            day: 0,
            day_change: 0,
        }
    }

    #[test]
    fn relationships_are_found_either_way_listed_in_key_order_and_dropped_with_a_pawn() {
        let mut t = Relationships::new();
        t.set(rel(2, 3, 100));
        t.set(rel(1, 2, 500));
        t.set(rel(1, 3, -20));
        assert_eq!(t.get(pawn(3), pawn(1)).map(|r| r.affinity), Some(-20));
        assert_eq!(
            t.iter().map(|r| r.key.text()).collect::<Vec<_>>(),
            ["pawn_1+pawn_2", "pawn_1+pawn_3", "pawn_2+pawn_3"]
        );
        assert_eq!(t.of(pawn(2)).count(), 2);
        t.get_mut(pawn(2), pawn(1)).unwrap().affinity = 600;
        assert_eq!(t.get(pawn(1), pawn(2)).unwrap().affinity, 600);
        t.remove_pawn(pawn(1));
        assert_eq!(t.len(), 1);
        assert!(t.to_canon().to_canonical_string().contains("pawn_2+pawn_3"));
    }

    #[test]
    fn a_relationship_and_a_household_round_trip_and_reject_nonsense() {
        let r = rel(1, 2, 250);
        let c = json::parse(&r.to_canon().to_canonical_string()).unwrap();
        assert_eq!(Relationship::from_reader(Reader::new(&c, "r")).unwrap(), r);
        let bad = json::parse(
            r#"{"a":"pawn_2","b":"pawn_1","affinity":5,"label":"x","last_interaction_tick":0,"day":0,"day_change":0}"#,
        )
        .unwrap();
        assert!(Relationship::from_reader(Reader::new(&bad, "r")).is_err());
        let bad = json::parse(
            r#"{"a":"pawn_1","b":"pawn_2","affinity":5000,"label":"x","last_interaction_tick":0,"day":0,"day_change":0}"#,
        )
        .unwrap();
        assert!(Relationship::from_reader(Reader::new(&bad, "r")).is_err());

        let h = Household {
            id: EntityId::new(Kind::Household, 1),
            name: "Lee".into(),
            members: vec![pawn(1), pawn(4)],
            home: None,
        };
        let c = json::parse(&h.to_canon().to_canonical_string()).unwrap();
        assert_eq!(Household::from_reader(Reader::new(&c, "h")).unwrap(), h);
        let unsorted =
            json::parse(r#"{"id":"hh_1","name":"Lee","members":["pawn_4","pawn_1"],"home":null}"#)
                .unwrap();
        assert!(Household::from_reader(Reader::new(&unsorted, "h")).is_err());
    }
}
