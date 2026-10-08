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

/// A conversation a pawn is in the middle of (Blueprint §9.2). Both pawns hold a copy; the one with the
/// lower id is the leader and closes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Talk {
    pub partner: EntityId,
    pub topic: String,
    pub tone: String,
    pub started: u64,
    /// The tick it closes.
    pub ends: u64,
    pub turns: u32,
    pub leader: bool,
    /// What was actually said, once known (set by a recorded dialogue; empty until then).
    pub lines: Vec<String>,
}

/// What a conversation memory remembers about the talk itself, so it can be recalled later: when it
/// began, how it went, and, when lines were generated for it, the lines themselves. Without `lines`,
/// the fallback lines are rebuilt from the data (they are a pure function of the talk).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalkRecord {
    pub started: u64,
    pub tone: String,
    pub turns: u32,
    /// The lines as spoken, first speaker (the lower id) first; empty when only the fallback was used.
    pub lines: Vec<String>,
}

impl ToCanon for TalkRecord {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("started", self.started.to_canon()),
            ("tone", Canon::str(self.tone.clone())),
            ("turns", self.turns.to_canon()),
            (
                "lines",
                Canon::List(self.lines.iter().map(|l| Canon::str(l.clone())).collect()),
            ),
        ])
    }
}

impl TalkRecord {
    pub fn from_reader(r: Reader<'_>) -> Result<TalkRecord, ReadError> {
        r.only(&["started", "tone", "turns", "lines"])?;
        Ok(TalkRecord {
            started: r.child("started")?.reader().u64()?,
            tone: r.child("tone")?.reader().str()?.to_owned(),
            turns: r.child("turns")?.reader().u32()?,
            lines: r
                .child("lines")?
                .reader()
                .list()?
                .iter()
                .map(|c| Ok(c.reader().str()?.to_owned()))
                .collect::<Result<Vec<_>, ReadError>>()?,
        })
    }
}

impl ToCanon for Talk {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("partner", self.partner.to_canon()),
            ("topic", Canon::str(self.topic.clone())),
            ("tone", Canon::str(self.tone.clone())),
            ("started", self.started.to_canon()),
            ("ends", self.ends.to_canon()),
            ("turns", self.turns.to_canon()),
            ("leader", Canon::Bool(self.leader)),
            (
                "lines",
                Canon::List(self.lines.iter().map(|l| Canon::str(l.clone())).collect()),
            ),
        ])
    }
}

impl Talk {
    pub fn from_reader(r: Reader<'_>) -> Result<Talk, ReadError> {
        r.only(&[
            "partner", "topic", "tone", "started", "ends", "turns", "leader", "lines",
        ])?;
        Ok(Talk {
            partner: r.child("partner")?.reader().parse()?,
            topic: r.child("topic")?.reader().str()?.to_owned(),
            tone: r.child("tone")?.reader().str()?.to_owned(),
            started: r.child("started")?.reader().u64()?,
            ends: r.child("ends")?.reader().u64()?,
            turns: r.child("turns")?.reader().u32()?,
            leader: r.child("leader")?.reader().bool()?,
            lines: r
                .child("lines")?
                .reader()
                .list()?
                .iter()
                .map(|c| Ok(c.reader().str()?.to_owned()))
                .collect::<Result<Vec<_>, ReadError>>()?,
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
    /// For a conversation: the talk itself, so it can be recalled (lines included when they were generated).
    pub talk: Option<TalkRecord>,
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
            talk: None,
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
            (
                "talk",
                self.talk.as_ref().map_or(Canon::Null, ToCanon::to_canon),
            ),
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
            "talk",
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
            talk: match r.maybe("talk")? {
                Some(c) => Some(TalkRecord::from_reader(c.reader())?),
                None => None,
            },
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
    /// The topic of the latest conversation, kept even after its memory fades.
    pub last_topic: Option<String>,
    /// How many of this pair's memories have been forgotten (rolled up from the pawns' memories).
    pub forgotten: u32,
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
            (
                "last_topic",
                self.last_topic
                    .as_ref()
                    .map_or(Canon::Null, |t| Canon::str(t.clone())),
            ),
            ("forgotten", self.forgotten.to_canon()),
        ])
    }
}

/// What applying an affinity change did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaResult {
    /// The change that was actually made (after the daily cap and the -1000..=1000 clamp).
    pub applied: i32,
    /// `(from, to)` when the label changed.
    pub label_change: Option<(String, String)>,
}

/// The label for `affinity` given the label the pair has now: it moves up only once affinity is `hysteresis`
/// past the next label's threshold, and down only once it is that far below its own, so a pair hovering at
/// a threshold does not flicker.
pub fn label_with_hysteresis(
    params: &pg_content::gamedata::RelationshipParams,
    current: &str,
    affinity: i32,
) -> String {
    let labels = &params.labels;
    let Some(mut idx) = labels.iter().position(|l| l.id == current) else {
        return params
            .label_for(affinity)
            .map(|l| l.id.clone())
            .unwrap_or_default();
    };
    let h = params.hysteresis;
    while let Some(next) = labels.get(idx + 1) {
        if affinity >= next.min.saturating_add(h) {
            idx += 1;
        } else {
            break;
        }
    }
    while idx > 0 {
        let here = labels.get(idx).map_or(i32::MIN, |l| l.min);
        if affinity < here.saturating_sub(h) {
            idx -= 1;
        } else {
            break;
        }
    }
    labels.get(idx).map(|l| l.id.clone()).unwrap_or_default()
}

impl Relationship {
    /// Adds `requested` to the affinity, held to the pair's daily cap (diminishing returns: the net change
    /// since the start of `day` stays within `+-daily_cap`) and to -1000..=1000, and updates the label.
    pub fn apply_delta(
        &mut self,
        params: &pg_content::gamedata::RelationshipParams,
        requested: i32,
        day: u64,
        tick: u64,
    ) -> DeltaResult {
        if self.day != day {
            self.day = day;
            self.day_change = 0;
        }
        let cap = params.daily_cap;
        let target = self.day_change.saturating_add(requested).clamp(-cap, cap);
        let mut applied = target - self.day_change;
        let affinity = self.affinity.saturating_add(applied).clamp(-1000, 1000);
        applied = affinity - self.affinity;
        self.affinity = affinity;
        self.day_change = self.day_change.saturating_add(applied);
        self.last_interaction_tick = tick;
        let label = label_with_hysteresis(params, &self.label, affinity);
        let label_change = (label != self.label).then(|| (self.label.clone(), label.clone()));
        self.label = label;
        DeltaResult {
            applied,
            label_change,
        }
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
            "last_topic",
            "forgotten",
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
            last_topic: match r.maybe("last_topic")? {
                Some(c) => Some(c.reader().str()?.to_owned()),
                None => None,
            },
            forgotten: r.child("forgotten")?.reader().u32()?,
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
            last_topic: None,
            forgotten: 0,
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

    fn rel_params() -> pg_content::gamedata::RelationshipParams {
        use pg_content::gamedata::{RelationshipLabel, RelationshipParams};
        let label = |id: &str, min: i32| RelationshipLabel {
            id: id.into(),
            label_key: format!("relationship.{id}"),
            min,
        };
        RelationshipParams {
            start_affinity: 0,
            daily_cap: 120,
            hysteresis: 40,
            labels: vec![
                label("enemy", -1000),
                label("stranger", -100),
                label("acquaintance", 100),
                label("friendly", 300),
            ],
        }
    }

    #[test]
    fn labels_move_only_after_passing_a_threshold_by_the_margin() {
        let p = rel_params();
        // Up: friendly starts at 300, so 300..339 stays acquaintance, 340 is friendly.
        assert_eq!(
            label_with_hysteresis(&p, "acquaintance", 339),
            "acquaintance"
        );
        assert_eq!(label_with_hysteresis(&p, "acquaintance", 340), "friendly");
        // Down: friendly holds until 260 is passed, so 261 stays friendly, 259 drops.
        assert_eq!(label_with_hysteresis(&p, "friendly", 261), "friendly");
        assert_eq!(label_with_hysteresis(&p, "friendly", 259), "acquaintance");
        // Jumps over several labels at once; an unknown label starts fresh.
        assert_eq!(label_with_hysteresis(&p, "enemy", 500), "friendly");
        assert_eq!(label_with_hysteresis(&p, "nonsense", 150), "acquaintance");
    }

    #[test]
    fn a_pair_hovering_at_a_threshold_does_not_flicker() {
        let p = rel_params();
        let mut r = rel(1, 2, 290);
        r.label = "acquaintance".into();
        let mut changes = 0;
        // Wobble +-30 around the threshold for two hundred conversations.
        for i in 0..200u64 {
            let d = if i % 2 == 0 { 30 } else { -30 };
            // A new day each time so the daily cap is out of the way.
            if r.apply_delta(&p, d, i, i).label_change.is_some() {
                changes += 1;
            }
        }
        assert_eq!(changes, 0, "290 +- 30 never passes 300 + 40 or 300 - 40");
    }

    #[test]
    fn the_daily_cap_and_the_clamp_hold_and_a_new_day_starts_over() {
        let p = rel_params();
        let mut r = rel(1, 2, 0);
        r.label = "stranger".into();
        let first = r.apply_delta(&p, 100, 5, 1);
        let second = r.apply_delta(&p, 100, 5, 2);
        assert_eq!((first.applied, second.applied), (100, 20), "cap 120 a day");
        assert_eq!(r.affinity, 120);
        assert_eq!(r.apply_delta(&p, 50, 5, 3).applied, 0);
        // Bad news the same day is not capped by the good news already counted; it works back toward -cap.
        assert_eq!(r.apply_delta(&p, -50, 5, 4).applied, -50);
        // A new day starts over.
        assert_eq!(r.apply_delta(&p, 100, 6, 5).applied, 100);
        assert_eq!((r.day, r.last_interaction_tick), (6, 5));
        // The clamp.
        let mut top = rel(3, 4, 990);
        let p2 = pg_content::gamedata::RelationshipParams {
            daily_cap: 2000,
            ..p
        };
        assert_eq!(top.apply_delta(&p2, 500, 1, 1).applied, 10);
        assert_eq!(top.affinity, 1000);
    }

    #[test]
    fn a_talk_round_trips() {
        let t = Talk {
            partner: pawn(2),
            topic: "food".into(),
            tone: "warm".into(),
            started: 100,
            ends: 160,
            turns: 3,
            leader: true,
            lines: vec!["Hi.".into()],
        };
        let text = t.to_canon().to_canonical_string();
        let back = Talk::from_reader(Reader::new(&json::parse(&text).unwrap(), "t")).unwrap();
        assert_eq!(back, t);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(200))]

        /// Whatever the conversations ask, affinity stays in range, the day's net change stays within the
        /// cap, applied changes add up, and the label always agrees with the affinity within the margin.
        #[test]
        fn relationship_rules_hold_for_any_sequence_of_conversations(
            steps in proptest::collection::vec((-400i32..400, 0u64..3), 1..120)
        ) {
            let p = rel_params();
            let mut r = rel(1, 2, 0);
            r.label = "stranger".into();
            let mut day = 0u64;
            let mut total = 0i32;
            for (i, (delta, advance)) in steps.into_iter().enumerate() {
                day += advance;
                let before = r.affinity;
                let res = r.apply_delta(&p, delta, day, i as u64);
                total += res.applied;
                proptest::prop_assert_eq!(r.affinity - before, res.applied);
                proptest::prop_assert!((-1000..=1000).contains(&r.affinity));
                proptest::prop_assert!(r.day_change.abs() <= p.daily_cap);
                // The label's range, widened by the margin, contains the affinity.
                let idx = p.labels.iter().position(|l| l.id == r.label).unwrap();
                let low = p.labels[idx].min - p.hysteresis;
                let high = p.labels.get(idx + 1).map_or(i32::MAX, |n| n.min + p.hysteresis);
                proptest::prop_assert!(r.affinity >= low && r.affinity < high || idx == 0 && r.affinity < high,
                    "affinity {} label {} range {low}..{high}", r.affinity, r.label);
            }
            proptest::prop_assert_eq!(r.affinity, total);
        }
    }
}
