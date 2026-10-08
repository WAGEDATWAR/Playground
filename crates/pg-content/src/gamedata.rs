//! Game data tables (Stage 1, milestone 1.0): the numbers and lists the simulation systems read, as data.
//!
//! A pack may carry any of these files under `data/game/`:
//!
//! | file | holds |
//! | --- | --- |
//! | `needs.json` | a list of need definitions (decay, thresholds) |
//! | `mood.json` | `{ "moods": [...], "rules": [...] }`: the emotional moods and the ordered rule table |
//! | `memory.json` | memory parameters (persistence threshold, bounds, decay, severity by type) |
//! | `relationships.json` | affinity labels, hysteresis and the daily cap |
//! | `occupations.json` | occupation schedule templates (identity and schedule archetypes) |
//! | `names.json` | name pools for generated residents |
//! | `residents.json` | authored resident definitions |
//!
//! Everything is integers (Blueprint §5.2). Entries are identified by `id`; a later pack's entry with an
//! existing id replaces it and says so in the report, the same way templates do. Cross references (a rule
//! naming a mood, a resident naming an occupation) are checked on the merged data, with "did you mean"
//! hints (S-012).

use crate::hints::hint;
use crate::report::ValidationReport;
use pg_canon::Canon;
use std::collections::{BTreeMap, BTreeSet};

pub const GAME_DIR: &str = "data/game/";
pub const NEEDS_FILE: &str = "data/game/needs.json";
pub const MOOD_FILE: &str = "data/game/mood.json";
pub const MEMORY_FILE: &str = "data/game/memory.json";
pub const RELATIONSHIPS_FILE: &str = "data/game/relationships.json";
pub const OCCUPATIONS_FILE: &str = "data/game/occupations.json";
pub const NAMES_FILE: &str = "data/game/names.json";
pub const RESIDENTS_FILE: &str = "data/game/residents.json";

pub const KNOWN_FILES: [&str; 7] = [
    NEEDS_FILE,
    MOOD_FILE,
    MEMORY_FILE,
    RELATIONSHIPS_FILE,
    OCCUPATIONS_FILE,
    NAMES_FILE,
    RESIDENTS_FILE,
];

/// Needs and affinity use this scale (Blueprint §8.1).
pub const SCALE: i32 = 1000;
pub const MINUTES_PER_DAY: u32 = 1440;

// ---- types ----------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedDef {
    pub id: String,
    pub label_key: String,
    /// Value a new resident starts with.
    pub start: i32,
    /// Points lost per game hour at rest.
    pub decay_per_hour: u32,
    /// Multiplier (permille) on decay while the pawn is active; 1000 means the same as at rest.
    pub active_permille: u32,
    pub urgent_below: i32,
    pub critical_below: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodDef {
    pub id: String,
    pub label_key: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tone {
    Positive,
    Negative,
}

impl Tone {
    pub fn name(self) -> &'static str {
        match self {
            Tone::Positive => "positive",
            Tone::Negative => "negative",
        }
    }
}

/// One condition of a mood rule; a rule holds when all of its conditions do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoodCond {
    NeedCritical {
        need: String,
    },
    AnyNeedUrgent,
    AllNeedsAbove {
        value: i32,
    },
    Memory {
        tone: Tone,
        within_hours: u32,
        min_importance: i32,
    },
    IdleHours {
        hours: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodRule {
    pub id: String,
    /// Rules are tried in ascending priority, then id; the first that holds decides.
    pub priority: u32,
    pub mood: String,
    pub when: Vec<MoodCond>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryType {
    pub id: String,
    /// Base severity, 1 to 5.
    pub severity: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryParams {
    /// Memories at or above this importance never fade.
    pub persist_threshold: i32,
    pub max_memories: u32,
    /// Daily retention loss is `max(1, decay_k / severity)`.
    pub decay_k: i32,
    pub types: Vec<MemoryType>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationshipLabel {
    pub id: String,
    pub label_key: String,
    /// The label applies from this affinity upward, until the next label's minimum.
    pub min: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationshipParams {
    pub start_affinity: i32,
    /// Largest total change per pair per day (diminishing returns).
    pub daily_cap: i32,
    /// A label changes only after affinity passes its threshold by this margin.
    pub hysteresis: i32,
    /// Ascending by `min`; the first starts at -1000.
    pub labels: Vec<RelationshipLabel>,
}

impl RelationshipParams {
    /// The label for `affinity` with no history (no hysteresis).
    pub fn label_for(&self, affinity: i32) -> Option<&RelationshipLabel> {
        self.labels.iter().rev().find(|l| affinity >= l.min)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Workplace,
    Gathering,
    Anywhere,
}

impl PlaceKind {
    pub const ALL: [PlaceKind; 4] = [
        PlaceKind::Home,
        PlaceKind::Workplace,
        PlaceKind::Gathering,
        PlaceKind::Anywhere,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PlaceKind::Home => "home",
            PlaceKind::Workplace => "workplace",
            PlaceKind::Gathering => "gathering",
            PlaceKind::Anywhere => "anywhere",
        }
    }

    pub fn parse(s: &str) -> Option<PlaceKind> {
        PlaceKind::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// A fixed block of the day, with the bounds its seeded variation may use (minutes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duty {
    /// The action (an `ActionId` in text form, checked against the action registry by the core).
    pub activity: String,
    pub place: PlaceKind,
    /// Minute of the day the duty starts.
    pub start: u32,
    pub minutes: u32,
    pub min_minutes: u32,
    pub shift_earlier: u32,
    pub shift_later: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeisureDef {
    pub activity: String,
    pub place: PlaceKind,
    pub minutes: u32,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationDef {
    pub id: String,
    pub name_key: String,
    /// How often generated towns pick it.
    pub weight: u32,
    pub duties: Vec<Duty>,
    pub leisure: Vec<LeisureDef>,
    /// Weight of leaving a free slot open against the leisure weights.
    pub open_weight: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamePools {
    pub given: Vec<String>,
    pub family: Vec<String>,
}

/// A resident written by hand (a scenario, a sample pack, a scripted town).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentDef {
    pub id: String,
    pub given: String,
    pub family: String,
    pub occupation: String,
    /// Residents with the same household key live together.
    pub household: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameData {
    pub needs: BTreeMap<String, NeedDef>,
    pub moods: BTreeMap<String, MoodDef>,
    pub mood_rules: Vec<MoodRule>,
    pub memory: Option<MemoryParams>,
    pub relationships: Option<RelationshipParams>,
    pub occupations: BTreeMap<String, OccupationDef>,
    pub names: NamePools,
    pub residents: BTreeMap<String, ResidentDef>,
}

impl GameData {
    pub fn is_empty(&self) -> bool {
        self == &GameData::default()
    }

    /// Needs in id order.
    pub fn need_ids(&self) -> Vec<&str> {
        self.needs.keys().map(String::as_str).collect()
    }

    /// The mood rules in evaluation order.
    pub fn ordered_rules(&self) -> Vec<&MoodRule> {
        let mut v: Vec<&MoodRule> = self.mood_rules.iter().collect();
        v.sort_by(|a, b| (a.priority, &a.id).cmp(&(b.priority, &b.id)));
        v
    }

    /// Adds `other` (a later pack): same-id entries replace, lists and pools extend.
    pub fn merge(&mut self, other: GameData, from: &str, report: &mut ValidationReport) {
        fn put<T>(
            into: &mut BTreeMap<String, T>,
            items: BTreeMap<String, T>,
            what: &str,
            from: &str,
            report: &mut ValidationReport,
        ) {
            for (id, v) in items {
                if into.insert(id.clone(), v).is_some() {
                    report.warn(
                        "overrides",
                        from.to_owned(),
                        format!("pack '{from}' replaces the earlier {what} '{id}'"),
                    );
                }
            }
        }
        put(&mut self.needs, other.needs, "need", from, report);
        put(&mut self.moods, other.moods, "mood", from, report);
        put(
            &mut self.occupations,
            other.occupations,
            "occupation",
            from,
            report,
        );
        put(
            &mut self.residents,
            other.residents,
            "resident",
            from,
            report,
        );
        for rule in other.mood_rules {
            match self.mood_rules.iter_mut().find(|r| r.id == rule.id) {
                Some(slot) => {
                    report.warn(
                        "overrides",
                        from.to_owned(),
                        format!("pack '{from}' replaces the earlier mood rule '{}'", rule.id),
                    );
                    *slot = rule;
                }
                None => self.mood_rules.push(rule),
            }
        }
        for (mine, theirs, what) in [
            (
                self.memory.is_some(),
                other.memory.is_some(),
                "memory parameters",
            ),
            (
                self.relationships.is_some(),
                other.relationships.is_some(),
                "relationship parameters",
            ),
        ] {
            if mine && theirs {
                report.warn(
                    "overrides",
                    from.to_owned(),
                    format!("pack '{from}' replaces the earlier {what}"),
                );
            }
        }
        if other.memory.is_some() {
            self.memory = other.memory;
        }
        if other.relationships.is_some() {
            self.relationships = other.relationships;
        }
        for g in other.names.given {
            if !self.names.given.contains(&g) {
                self.names.given.push(g);
            }
        }
        for f in other.names.family {
            if !self.names.family.contains(&f) {
                self.names.family.push(f);
            }
        }
    }

    /// Cross-reference and consistency checks on the merged data. Quiet when there is no data.
    pub fn validate(&self, report: &mut ValidationReport) {
        let need_ids = self.needs.keys().map(String::as_str);
        let mood_ids = self.moods.keys().map(String::as_str);
        for rule in &self.mood_rules {
            let path = format!("{MOOD_FILE}.rules.{}", rule.id);
            if !self.moods.contains_key(&rule.mood) {
                report.error(
                    "unknown_mood",
                    path.clone(),
                    format!(
                        "rule '{}' gives mood '{}', which is not defined{}",
                        rule.id,
                        rule.mood,
                        hint(&rule.mood, mood_ids.clone())
                    ),
                );
            }
            for c in &rule.when {
                if let MoodCond::NeedCritical { need } = c {
                    if !self.needs.contains_key(need) {
                        report.error(
                            "unknown_need",
                            path.clone(),
                            format!(
                                "rule '{}' tests need '{need}', which is not defined{}",
                                rule.id,
                                hint(need, need_ids.clone())
                            ),
                        );
                    }
                }
            }
        }
        if !self.mood_rules.is_empty() && !self.mood_rules.iter().any(|r| r.when.is_empty()) {
            report.warn(
                "no_default_mood",
                MOOD_FILE,
                "no rule without conditions, so a pawn that matches nothing has no mood; add a final catch-all rule",
            );
        }
        if let Some(rel) = &self.relationships {
            if rel.labels.first().map(|l| l.min) != Some(-SCALE) {
                report.error(
                    "labels_do_not_cover",
                    RELATIONSHIPS_FILE,
                    "the first label must start at -1000 so every affinity has a label",
                );
            }
            if !(-SCALE..=SCALE).contains(&rel.start_affinity) {
                report.error(
                    "bad_range",
                    format!("{RELATIONSHIPS_FILE}.start_affinity"),
                    "starting affinity must be between -1000 and 1000",
                );
            }
        }
        for occ in self.occupations.values() {
            let path = format!("{OCCUPATIONS_FILE}.{}", occ.id);
            for (i, d) in occ.duties.iter().enumerate() {
                let p = format!("{path}.duties[{i}]");
                if d.start + d.minutes > MINUTES_PER_DAY {
                    report.error("bad_range", p.clone(), "a duty must end before midnight");
                }
                if d.min_minutes == 0 || d.min_minutes > d.minutes {
                    report.error(
                        "bad_range",
                        p,
                        "min_minutes must be at least 1 and at most minutes",
                    );
                }
            }
            let mut spans: Vec<(u32, u32)> = occ
                .duties
                .iter()
                .map(|d| (d.start, d.start + d.minutes))
                .collect();
            spans.sort_unstable();
            if spans.windows(2).any(|w| matches!(w, [a, b] if b.0 < a.1)) {
                report.error("overlapping_duties", path, "two duties overlap in time");
            }
        }
        let occ_ids = self.occupations.keys().map(String::as_str);
        for r in self.residents.values() {
            if !self.occupations.contains_key(&r.occupation) {
                report.error(
                    "unknown_occupation",
                    format!("{RESIDENTS_FILE}.{}", r.id),
                    format!(
                        "resident '{}' has occupation '{}', which is not defined{}",
                        r.id,
                        r.occupation,
                        hint(&r.occupation, occ_ids.clone())
                    ),
                );
            }
        }
    }

    /// Every string-table key the data refers to (for the missing-key lint).
    pub fn string_keys(&self) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        keys.extend(self.needs.values().map(|n| n.label_key.clone()));
        keys.extend(self.moods.values().map(|m| m.label_key.clone()));
        keys.extend(self.occupations.values().map(|o| o.name_key.clone()));
        if let Some(r) = &self.relationships {
            keys.extend(r.labels.iter().map(|l| l.label_key.clone()));
        }
        keys
    }
}

// ---- parsing --------------------------------------------------------------------------------------------

/// A JSON object being read: unknown and missing fields, wrong types and out-of-range numbers are
/// reported with the path, and reading carries on so one pass shows every problem.
struct Obj<'a> {
    map: &'a BTreeMap<String, Canon>,
    path: String,
}

impl<'a> Obj<'a> {
    fn new(
        value: &'a Canon,
        path: impl Into<String>,
        allowed: &[&str],
        report: &mut ValidationReport,
    ) -> Option<Obj<'a>> {
        let path = path.into();
        let Canon::Map(map) = value else {
            report.error("type_mismatch", path, "expected an object");
            return None;
        };
        for key in map.keys() {
            if !allowed.contains(&key.as_str()) {
                report.error(
                    "unknown_field",
                    format!("{path}.{key}"),
                    format!(
                        "unknown field{} (allowed: {})",
                        hint(key, allowed.iter().copied()),
                        allowed.join(", ")
                    ),
                );
            }
        }
        Some(Obj { map, path })
    }

    fn at(&self, key: &str) -> String {
        format!("{}.{key}", self.path)
    }

    fn text(&self, key: &str, max: usize, report: &mut ValidationReport) -> Option<String> {
        match self.map.get(key) {
            Some(Canon::Str(s))
                if !s.trim().is_empty()
                    && s.chars().count() <= max
                    && !s.chars().any(char::is_control) =>
            {
                Some(s.clone())
            }
            Some(Canon::Str(_)) => {
                report.error(
                    "bad_text",
                    self.at(key),
                    format!("must be 1 to {max} characters with no control characters"),
                );
                None
            }
            Some(_) => {
                report.error("type_mismatch", self.at(key), "expected text");
                None
            }
            None => {
                report.error("missing_field", self.at(key), "this field is required");
                None
            }
        }
    }

    fn id(&self, key: &str, report: &mut ValidationReport) -> Option<String> {
        let t = self.text(key, 48, report)?;
        let ok = t
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.'))
            && t.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        if !ok {
            report.error(
                "bad_id",
                self.at(key),
                format!(
                    "'{t}' must be lowercase letters, digits, '_' or '.', starting with a letter"
                ),
            );
            return None;
        }
        Some(t)
    }

    fn int(
        &self,
        key: &str,
        min: i64,
        max: i64,
        default: Option<i64>,
        report: &mut ValidationReport,
    ) -> Option<i64> {
        match self.map.get(key) {
            Some(Canon::Int(i)) => match i64::try_from(*i) {
                Ok(v) if (min..=max).contains(&v) => Some(v),
                _ => {
                    report.error(
                        "bad_range",
                        self.at(key),
                        format!("must be between {min} and {max}"),
                    );
                    None
                }
            },
            Some(_) => {
                report.error("type_mismatch", self.at(key), "expected a whole number");
                None
            }
            None => match default {
                Some(d) => Some(d),
                None => {
                    report.error("missing_field", self.at(key), "this field is required");
                    None
                }
            },
        }
    }

    fn list(&self, key: &str, report: &mut ValidationReport) -> Option<&'a Vec<Canon>> {
        match self.map.get(key) {
            Some(Canon::List(l)) => Some(l),
            Some(_) => {
                report.error("type_mismatch", self.at(key), "expected a list");
                None
            }
            None => {
                report.error("missing_field", self.at(key), "this field is required");
                None
            }
        }
    }
}

fn i32_of(v: Option<i64>) -> Option<i32> {
    v.and_then(|v| i32::try_from(v).ok())
}

fn u32_of(v: Option<i64>) -> Option<u32> {
    v.and_then(|v| u32::try_from(v).ok())
}

/// A file holding a list of objects: each is read by `item`.
fn each_item<T>(
    value: &Canon,
    path: &str,
    report: &mut ValidationReport,
    mut item: impl FnMut(&Canon, String, &mut ValidationReport) -> Option<T>,
) -> Vec<T> {
    let Canon::List(list) = value else {
        report.error("type_mismatch", path, "expected a list");
        return Vec::new();
    };
    list.iter()
        .enumerate()
        .filter_map(|(i, v)| item(v, format!("{path}[{i}]"), report))
        .collect()
}

fn unique<T>(
    items: Vec<(String, T)>,
    path: &str,
    what: &str,
    report: &mut ValidationReport,
) -> BTreeMap<String, T> {
    let mut out = BTreeMap::new();
    for (id, v) in items {
        if out.insert(id.clone(), v).is_some() {
            report.error(
                "duplicate_id",
                path,
                format!("{what} '{id}' is defined twice in this file"),
            );
        }
    }
    out
}

pub fn parse_needs(value: &Canon, report: &mut ValidationReport) -> BTreeMap<String, NeedDef> {
    let items = each_item(value, NEEDS_FILE, report, |v, path, r| {
        let o = Obj::new(
            v,
            path,
            &[
                "id",
                "label_key",
                "start",
                "decay_per_hour",
                "active_permille",
                "urgent_below",
                "critical_below",
            ],
            r,
        )?;
        let def = NeedDef {
            id: o.id("id", r)?,
            label_key: o.text("label_key", 80, r)?,
            start: i32_of(o.int("start", 0, 1000, Some(1000), r))?,
            decay_per_hour: u32_of(o.int("decay_per_hour", 0, 1000, None, r))?,
            active_permille: u32_of(o.int("active_permille", 0, 5000, Some(1000), r))?,
            urgent_below: i32_of(o.int("urgent_below", 1, 999, None, r))?,
            critical_below: i32_of(o.int("critical_below", 0, 998, None, r))?,
        };
        if def.critical_below >= def.urgent_below {
            r.error(
                "bad_range",
                o.at("critical_below"),
                "critical_below must be lower than urgent_below",
            );
            return None;
        }
        Some((def.id.clone(), def))
    });
    unique(items, NEEDS_FILE, "need", report)
}

fn parse_cond(v: &Canon, path: String, r: &mut ValidationReport) -> Option<MoodCond> {
    let Canon::Map(m) = v else {
        r.error("type_mismatch", path, "expected an object");
        return None;
    };
    let kind = match m.get("type") {
        Some(Canon::Str(s)) => s.clone(),
        _ => {
            r.error(
                "missing_field",
                format!("{path}.type"),
                "a condition needs a text 'type'",
            );
            return None;
        }
    };
    const KINDS: [&str; 5] = [
        "need_critical",
        "any_need_urgent",
        "all_needs_above",
        "memory",
        "idle_hours",
    ];
    match kind.as_str() {
        "need_critical" => {
            let o = Obj::new(v, path, &["type", "need"], r)?;
            Some(MoodCond::NeedCritical {
                need: o.id("need", r)?,
            })
        }
        "any_need_urgent" => {
            Obj::new(v, path, &["type"], r)?;
            Some(MoodCond::AnyNeedUrgent)
        }
        "all_needs_above" => {
            let o = Obj::new(v, path, &["type", "value"], r)?;
            Some(MoodCond::AllNeedsAbove {
                value: i32_of(o.int("value", 0, 1000, None, r))?,
            })
        }
        "memory" => {
            let o = Obj::new(
                v,
                path,
                &["type", "tone", "within_hours", "min_importance"],
                r,
            )?;
            let tone = match m.get("tone") {
                Some(Canon::Str(s)) if s == "positive" => Tone::Positive,
                Some(Canon::Str(s)) if s == "negative" => Tone::Negative,
                _ => {
                    r.error(
                        "bad_enum",
                        o.at("tone"),
                        "tone must be \"positive\" or \"negative\"",
                    );
                    return None;
                }
            };
            Some(MoodCond::Memory {
                tone,
                within_hours: u32_of(o.int("within_hours", 1, 24 * 30, None, r))?,
                min_importance: i32_of(o.int("min_importance", 0, 100_000, Some(0), r))?,
            })
        }
        "idle_hours" => {
            let o = Obj::new(v, path, &["type", "hours"], r)?;
            Some(MoodCond::IdleHours {
                hours: u32_of(o.int("hours", 1, 24 * 7, None, r))?,
            })
        }
        other => {
            r.error(
                "bad_enum",
                format!("{path}.type"),
                format!(
                    "unknown condition '{other}'{} (allowed: {})",
                    hint(other, KINDS.iter().copied()),
                    KINDS.join(", ")
                ),
            );
            None
        }
    }
}

pub fn parse_mood(
    value: &Canon,
    report: &mut ValidationReport,
) -> (BTreeMap<String, MoodDef>, Vec<MoodRule>) {
    let Some(top) = Obj::new(value, MOOD_FILE, &["moods", "rules"], report) else {
        return (BTreeMap::new(), Vec::new());
    };
    let moods = match top.map.get("moods") {
        Some(l) => {
            let items = each_item(l, &top.at("moods"), report, |v, path, r| {
                let o = Obj::new(v, path, &["id", "label_key"], r)?;
                let d = MoodDef {
                    id: o.id("id", r)?,
                    label_key: o.text("label_key", 80, r)?,
                };
                Some((d.id.clone(), d))
            });
            unique(items, MOOD_FILE, "mood", report)
        }
        None => BTreeMap::new(),
    };
    let mut rules = Vec::new();
    if let Some(l) = top.map.get("rules") {
        let items = each_item(l, &top.at("rules"), report, |v, path, r| {
            let o = Obj::new(v, path.clone(), &["id", "priority", "mood", "when"], r)?;
            let when = match o.map.get("when") {
                None => Vec::new(),
                Some(Canon::List(list)) => list
                    .iter()
                    .enumerate()
                    .map(|(i, c)| parse_cond(c, format!("{path}.when[{i}]"), r))
                    .collect::<Option<Vec<_>>>()?,
                Some(_) => {
                    r.error(
                        "type_mismatch",
                        o.at("when"),
                        "expected a list of conditions",
                    );
                    return None;
                }
            };
            let rule = MoodRule {
                id: o.id("id", r)?,
                priority: u32_of(o.int("priority", 0, 10_000, None, r))?,
                mood: o.id("mood", r)?,
                when,
            };
            Some((rule.id.clone(), rule))
        });
        rules = unique(items, MOOD_FILE, "mood rule", report)
            .into_values()
            .collect();
    }
    (moods, rules)
}

pub fn parse_memory(value: &Canon, report: &mut ValidationReport) -> Option<MemoryParams> {
    let o = Obj::new(
        value,
        MEMORY_FILE,
        &["persist_threshold", "max_memories", "decay_k", "types"],
        report,
    )?;
    let items = o.list("types", report).map(|l| {
        l.iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let t = Obj::new(
                    v,
                    format!("{MEMORY_FILE}.types[{i}]"),
                    &["id", "severity"],
                    report,
                )?;
                let ty = MemoryType {
                    id: t.id("id", report)?,
                    severity: u8::try_from(t.int("severity", 1, 5, None, report)?).ok()?,
                };
                Some((ty.id.clone(), ty))
            })
            .collect::<Vec<_>>()
    })?;
    let types = unique(items, MEMORY_FILE, "memory type", report);
    Some(MemoryParams {
        persist_threshold: i32_of(o.int("persist_threshold", 1, 1_000_000, None, report))?,
        max_memories: u32_of(o.int("max_memories", 1, 5000, Some(200), report))?,
        decay_k: i32_of(o.int("decay_k", 1, 10_000, None, report))?,
        types: types.into_values().collect(),
    })
}

pub fn parse_relationships(
    value: &Canon,
    report: &mut ValidationReport,
) -> Option<RelationshipParams> {
    let o = Obj::new(
        value,
        RELATIONSHIPS_FILE,
        &["start_affinity", "daily_cap", "hysteresis", "labels"],
        report,
    )?;
    let labels = o.list("labels", report).map(|l| {
        l.iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let t = Obj::new(
                    v,
                    format!("{RELATIONSHIPS_FILE}.labels[{i}]"),
                    &["id", "label_key", "min"],
                    report,
                )?;
                Some(RelationshipLabel {
                    id: t.id("id", report)?,
                    label_key: t.text("label_key", 80, report)?,
                    min: i32_of(t.int("min", -1000, 1000, None, report))?,
                })
            })
            .collect::<Vec<_>>()
    })?;
    let mut labels = labels;
    labels.sort_by_key(|l| l.min);
    if labels
        .windows(2)
        .any(|w| matches!(w, [a, b] if a.min == b.min))
    {
        report.error(
            "duplicate_id",
            RELATIONSHIPS_FILE,
            "two labels start at the same affinity",
        );
    }
    if labels.is_empty() {
        report.error(
            "missing_field",
            RELATIONSHIPS_FILE,
            "at least one label is needed",
        );
        return None;
    }
    Some(RelationshipParams {
        start_affinity: i32_of(o.int("start_affinity", -1000, 1000, Some(0), report))?,
        daily_cap: i32_of(o.int("daily_cap", 1, 2000, None, report))?,
        hysteresis: i32_of(o.int("hysteresis", 0, 500, Some(0), report))?,
        labels,
    })
}

fn parse_place(o: &Obj<'_>, key: &str, r: &mut ValidationReport) -> Option<PlaceKind> {
    match o.map.get(key) {
        Some(Canon::Str(s)) => match PlaceKind::parse(s) {
            Some(p) => Some(p),
            None => {
                r.error(
                    "bad_enum",
                    o.at(key),
                    format!(
                        "unknown place '{s}'{} (allowed: home, workplace, gathering, anywhere)",
                        hint(s, PlaceKind::ALL.iter().map(|p| p.name()))
                    ),
                );
                None
            }
        },
        _ => {
            r.error("missing_field", o.at(key), "a place is required");
            None
        }
    }
}

pub fn parse_occupations(
    value: &Canon,
    report: &mut ValidationReport,
) -> BTreeMap<String, OccupationDef> {
    let items = each_item(value, OCCUPATIONS_FILE, report, |v, path, r| {
        let o = Obj::new(
            v,
            path.clone(),
            &[
                "id",
                "name_key",
                "weight",
                "duties",
                "leisure",
                "open_weight",
            ],
            r,
        )?;
        let mut duties = Vec::new();
        let mut ok = true;
        if let Some(Canon::List(l)) = o.map.get("duties") {
            for (i, d) in l.iter().enumerate() {
                let p = format!("{path}.duties[{i}]");
                let Some(d) = Obj::new(
                    d,
                    p,
                    &[
                        "activity",
                        "place",
                        "start",
                        "minutes",
                        "min_minutes",
                        "shift_earlier",
                        "shift_later",
                    ],
                    r,
                ) else {
                    ok = false;
                    continue;
                };
                let duty = (|| {
                    let minutes = u32_of(d.int("minutes", 5, 1440, None, r))?;
                    Some(Duty {
                        activity: d.text("activity", 48, r)?,
                        place: parse_place(&d, "place", r)?,
                        start: u32_of(d.int("start", 0, 1439, None, r))?,
                        minutes,
                        min_minutes: u32_of(d.int(
                            "min_minutes",
                            1,
                            1440,
                            Some(i64::from(minutes)),
                            r,
                        ))?,
                        shift_earlier: u32_of(d.int("shift_earlier", 0, 720, Some(0), r))?,
                        shift_later: u32_of(d.int("shift_later", 0, 720, Some(0), r))?,
                    })
                })();
                match duty {
                    Some(x) => duties.push(x),
                    None => ok = false,
                }
            }
        } else if o.map.contains_key("duties") {
            r.error("type_mismatch", o.at("duties"), "expected a list");
            ok = false;
        }
        let mut leisure = Vec::new();
        if let Some(Canon::List(l)) = o.map.get("leisure") {
            for (i, d) in l.iter().enumerate() {
                let p = format!("{path}.leisure[{i}]");
                let Some(d) = Obj::new(d, p, &["activity", "place", "minutes", "weight"], r) else {
                    ok = false;
                    continue;
                };
                let item = (|| {
                    Some(LeisureDef {
                        activity: d.text("activity", 48, r)?,
                        place: parse_place(&d, "place", r)?,
                        minutes: u32_of(d.int("minutes", 5, 720, None, r))?,
                        weight: u32_of(d.int("weight", 1, 1000, None, r))?,
                    })
                })();
                match item {
                    Some(x) => leisure.push(x),
                    None => ok = false,
                }
            }
        } else if o.map.contains_key("leisure") {
            r.error("type_mismatch", o.at("leisure"), "expected a list");
            ok = false;
        }
        let def = OccupationDef {
            id: o.id("id", r)?,
            name_key: o.text("name_key", 80, r)?,
            weight: u32_of(o.int("weight", 0, 1000, Some(10), r))?,
            duties,
            leisure,
            open_weight: u32_of(o.int("open_weight", 0, 1000, Some(15), r))?,
        };
        ok.then(|| (def.id.clone(), def))
    });
    unique(items, OCCUPATIONS_FILE, "occupation", report)
}

pub fn parse_names(value: &Canon, report: &mut ValidationReport) -> NamePools {
    let Some(o) = Obj::new(value, NAMES_FILE, &["given", "family"], report) else {
        return NamePools::default();
    };
    let mut pool = |key: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(Canon::List(l)) = o.map.get(key) {
            for (i, v) in l.iter().enumerate() {
                match v {
                    Canon::Str(s)
                        if !s.trim().is_empty()
                            && s.chars().count() <= 24
                            && !s.chars().any(char::is_control) =>
                    {
                        if out.contains(s) {
                            report.warn(
                                "duplicate_name",
                                format!("{}[{i}]", o.at(key)),
                                format!("'{s}' is listed twice"),
                            );
                        } else {
                            out.push(s.clone());
                        }
                    }
                    _ => report.error(
                        "bad_text",
                        format!("{}[{i}]", o.at(key)),
                        "a name is 1 to 24 characters with no control characters",
                    ),
                }
            }
        } else if o.map.contains_key(key) {
            report.error("type_mismatch", o.at(key), "expected a list of names");
        }
        out
    };
    NamePools {
        given: pool("given"),
        family: pool("family"),
    }
}

pub fn parse_residents(
    value: &Canon,
    report: &mut ValidationReport,
) -> BTreeMap<String, ResidentDef> {
    let items = each_item(value, RESIDENTS_FILE, report, |v, path, r| {
        let o = Obj::new(
            v,
            path,
            &["id", "given", "family", "occupation", "household"],
            r,
        )?;
        let household = match o.map.get("household") {
            None | Some(Canon::Null) => None,
            Some(_) => Some(o.id("household", r)?),
        };
        let def = ResidentDef {
            id: o.id("id", r)?,
            given: o.text("given", 24, r)?,
            family: o.text("family", 24, r)?,
            occupation: o.id("occupation", r)?,
            household,
        };
        Some((def.id.clone(), def))
    });
    unique(items, RESIDENTS_FILE, "resident", report)
}

/// Parses whichever of the known files exist in `files` (path to parsed JSON). Files under
/// `data/game/` that are not known produce a warning.
pub fn parse_files(files: &BTreeMap<String, Canon>, report: &mut ValidationReport) -> GameData {
    let mut data = GameData::default();
    for (path, value) in files {
        match path.as_str() {
            NEEDS_FILE => data.needs = parse_needs(value, report),
            MOOD_FILE => {
                let (m, r) = parse_mood(value, report);
                data.moods = m;
                data.mood_rules = r;
            }
            MEMORY_FILE => data.memory = parse_memory(value, report),
            RELATIONSHIPS_FILE => data.relationships = parse_relationships(value, report),
            OCCUPATIONS_FILE => data.occupations = parse_occupations(value, report),
            NAMES_FILE => data.names = parse_names(value, report),
            RESIDENTS_FILE => data.residents = parse_residents(value, report),
            other => report.warn(
                "unknown_game_file",
                other.to_owned(),
                format!(
                    "not a known game data file{} (known: {})",
                    hint(other, KNOWN_FILES.iter().copied()),
                    KNOWN_FILES.join(", ")
                ),
            ),
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json;

    fn parse(path: &str, text: &str) -> (GameData, ValidationReport) {
        let mut report = ValidationReport::new();
        let value = json::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let mut files = BTreeMap::new();
        files.insert(path.to_owned(), value);
        let data = parse_files(&files, &mut report);
        (data, report)
    }

    fn base_files() -> BTreeMap<String, Canon> {
        let root = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        KNOWN_FILES
            .iter()
            .filter_map(|p| {
                let text = std::fs::read_to_string(format!("{root}/{p}")).ok()?;
                Some(((*p).to_owned(), json::parse(&text).ok()?))
            })
            .collect()
    }

    #[test]
    fn the_base_pack_data_parses_validates_and_is_complete() {
        let mut report = ValidationReport::new();
        let data = parse_files(&base_files(), &mut report);
        data.validate(&mut report);
        assert!(report.is_empty(), "{report}");
        assert_eq!(data.need_ids(), ["energy", "hunger", "social"]);
        assert_eq!(data.moods.len(), 8);
        assert_eq!(
            data.ordered_rules().first().map(|r| r.id.as_str()),
            Some("lonely_when_social_critical")
        );
        assert_eq!(
            data.ordered_rules().last().map(|r| r.mood.as_str()),
            Some("neutral")
        );
        assert!(data.occupations.len() >= 5 && data.names.given.len() >= 30);
        let rel = data.relationships.as_ref().unwrap();
        assert_eq!(rel.label_for(0).unwrap().id, "stranger");
        assert_eq!(rel.label_for(-1000).unwrap().id, "enemy");
        assert_eq!(rel.label_for(1000).unwrap().id, "close_friend");
        assert_eq!(rel.label_for(299).unwrap().id, "acquaintance");
        assert_eq!(rel.label_for(300).unwrap().id, "friendly");
    }

    #[test]
    fn mistakes_are_reported_with_paths_and_hints_and_reading_carries_on() {
        let (data, r) = parse(
            NEEDS_FILE,
            r#"[{"id":"hunger","label_key":"k","dekay_per_hour":3,"decay_per_hour":50,"urgent_below":300,"critical_below":100},
                {"id":"Energy","label_key":"k","decay_per_hour":50,"urgent_below":300,"critical_below":100},
                {"id":"social","label_key":"k","decay_per_hour":50,"urgent_below":100,"critical_below":300},
                {"id":"ok","label_key":"k","decay_per_hour":50,"urgent_below":300,"critical_below":100}]"#,
        );
        assert_eq!(
            data.needs.len(),
            2,
            "the entries with bad ids or ranges are dropped, the rest kept: {r}"
        );
        assert!(
            r.has_code("unknown_field") && r.has_code("bad_id") && r.has_code("bad_range"),
            "{r}"
        );
        let text = r.to_string();
        assert!(text.contains("did you mean 'decay_per_hour'"), "{text}");
        assert!(
            text.contains("data/game/needs.json[0].dekay_per_hour"),
            "{text}"
        );
        let (_, r) = parse(NEEDS_FILE, r#"{"not":"a list"}"#);
        assert!(r.has_code("type_mismatch"));
        let (_, r) = parse(
            NEEDS_FILE,
            r#"[{"id":"a","label_key":"k","decay_per_hour":"fast","urgent_below":3,"critical_below":1}]"#,
        );
        assert!(r.has_code("type_mismatch"), "{r}");
        let (_, r) = parse("data/game/needz.json", "[]");
        assert!(
            r.has_code("unknown_game_file") && r.to_string().contains("did you mean"),
            "{r}"
        );
    }

    #[test]
    fn rules_moods_and_residents_must_refer_to_things_that_exist() {
        let mut data = GameData::default();
        let mut r = ValidationReport::new();
        let (m, rules) = parse_mood(
            &json::parse(
                r#"{"moods":[{"id":"cheerful","label_key":"k"}],
                    "rules":[{"id":"a","priority":1,"mood":"cheerfull","when":[{"type":"need_critical","need":"hungr"}]}]}"#,
            )
            .unwrap(),
            &mut r,
        );
        assert!(r.is_ok(), "{r}");
        data.moods = m;
        data.mood_rules = rules;
        data.needs = parse_needs(
            &json::parse(r#"[{"id":"hunger","label_key":"k","decay_per_hour":50,"urgent_below":300,"critical_below":100}]"#).unwrap(),
            &mut r,
        );
        data.occupations = parse_occupations(
            &json::parse(r#"[{"id":"barista","name_key":"k"}]"#).unwrap(),
            &mut r,
        );
        data.residents = parse_residents(
            &json::parse(r#"[{"id":"r1","given":"Ann","family":"Lee","occupation":"barrista"}]"#)
                .unwrap(),
            &mut r,
        );
        assert!(r.is_ok(), "{r}");
        data.validate(&mut r);
        let text = r.to_string();
        assert!(
            text.contains("unknown_mood") && text.contains("did you mean 'cheerful'"),
            "{text}"
        );
        assert!(
            text.contains("unknown_need") && text.contains("did you mean 'hunger'"),
            "{text}"
        );
        assert!(
            text.contains("unknown_occupation") && text.contains("did you mean 'barista'"),
            "{text}"
        );
        assert!(
            r.has_code("no_default_mood"),
            "a table without a catch-all warns: {text}"
        );
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn duties_and_labels_are_checked_for_sense() {
        let mut r = ValidationReport::new();
        let mut data = GameData::default();
        data.occupations = parse_occupations(
            &json::parse(
                r#"[{"id":"a","name_key":"k","duties":[
                    {"activity":"idle_at","place":"workplace","start":600,"minutes":120},
                    {"activity":"idle_at","place":"workplace","start":660,"minutes":120},
                    {"activity":"idle_at","place":"workplace","start":1400,"minutes":120}]}]"#,
            )
            .unwrap(),
            &mut r,
        );
        data.relationships = parse_relationships(
            &json::parse(r#"{"daily_cap":100,"labels":[{"id":"x","label_key":"k","min":-500}]}"#)
                .unwrap(),
            &mut r,
        );
        data.validate(&mut r);
        assert!(
            r.has_code("overlapping_duties") && r.has_code("labels_do_not_cover"),
            "{r}"
        );
        assert!(r.to_string().contains("end before midnight"), "{r}");
        let mut r = ValidationReport::new();
        let o = parse_occupations(
            &json::parse(r#"[{"id":"a","name_key":"k","duties":[{"activity":"idle_at","place":"nowhere","start":0,"minutes":60}]}]"#).unwrap(),
            &mut r,
        );
        assert!(o.is_empty() && r.has_code("bad_enum"), "{r}");
    }

    #[test]
    fn a_later_pack_replaces_by_id_with_a_warning_and_name_pools_extend() {
        let (mut a, _) = parse(NAMES_FILE, r#"{"given":["Ann"],"family":["Lee"]}"#);
        let (b, _) = parse(NAMES_FILE, r#"{"given":["Ann","Bo"],"family":[]}"#);
        let (mut needs_a, _) = parse(
            NEEDS_FILE,
            r#"[{"id":"hunger","label_key":"k","decay_per_hour":50,"urgent_below":300,"critical_below":100}]"#,
        );
        let (needs_b, _) = parse(
            NEEDS_FILE,
            r#"[{"id":"hunger","label_key":"k2","decay_per_hour":10,"urgent_below":300,"critical_below":100}]"#,
        );
        let mut r = ValidationReport::new();
        a.merge(b, "extra", &mut r);
        needs_a.merge(needs_b, "extra", &mut r);
        assert_eq!(a.names.given, ["Ann", "Bo"]);
        assert_eq!(needs_a.needs["hunger"].decay_per_hour, 10);
        assert!(r.has_code("overrides") && r.is_ok(), "{r}");
    }
}
