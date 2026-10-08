//! `WorldState` (Blueprint §4.5). Every table has a canonical form, and the state hash is the combination
//! of per-table hashes, so a divergence between two runs can be localized to the first table that differs.
//!
//! Milestone 0.4 adds the first real entity tables (maps, objects, pawns). Needs, memories,
//! relationships and the rest arrive with Stage 1.

use crate::canon::{Canon, ToCanon};
use crate::commitment::Commitment;
use crate::hash::{combine_row_hashes, combine_table_hashes, hash_value, StateHash};
use crate::id::{EntityId, IdCounters, IdsExhausted, Kind};
use crate::map::{MapData, MapError, MapKind, MoveCosts, Tile};
use crate::object::ObjectInstance;
use crate::occupancy::{Occupancy, OccupancyError};
use crate::pawn::{Pawn, Position};
use crate::read::{ReadError, Reader};
use crate::rng::Seed;
use crate::social::{Household, Relationship, Relationships};
use crate::table::Table;
use crate::time::{Clock, SlotMinutes};
use crate::town::Town;
use std::collections::BTreeMap;
use std::fmt;

/// Bumped whenever the saved shape of `WorldState` changes (migrations hang off this, §13.5).
pub const SCHEMA_VERSION: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldMeta {
    pub name: String,
    /// The text the player typed (or was generated); the numeric seed derives from it.
    pub seed_text: String,
}

/// Movement tuning (Blueprint §6.3, §7.4). These are world settings, not derived values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovementSettings {
    /// Ticks to enter one tile (at least 1). Typical in-town trips should take roughly 5–20 game minutes.
    pub move_ticks_per_tile: u32,
    /// Ticks to wait on an occupied tile before trying a sidestep.
    pub max_wait_ticks: u32,
    /// Re-solves allowed before a route fails with `path_blocked`.
    pub max_repaths: u32,
    /// Node-expansion cap per path request.
    pub path_expansion_cap: u32,
}

impl Default for MovementSettings {
    fn default() -> MovementSettings {
        MovementSettings {
            move_ticks_per_tile: 2,
            max_wait_ticks: 20,
            max_repaths: 3,
            path_expansion_cap: 20_000,
        }
    }
}

/// The world's tone preset (Design Document section 9): how likely serious events are. It never changes what an
/// event does, only how often it happens, and it is separate from the graphic-content filter, which is a
/// device setting and only changes how things are described and drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TonePreset {
    Cozy,
    #[default]
    Standard,
    Mature,
}

impl TonePreset {
    pub const ALL: [TonePreset; 3] = [TonePreset::Cozy, TonePreset::Standard, TonePreset::Mature];

    pub const fn name(self) -> &'static str {
        match self {
            TonePreset::Cozy => "cozy",
            TonePreset::Standard => "standard",
            TonePreset::Mature => "mature",
        }
    }

    pub fn parse(s: &str) -> Option<TonePreset> {
        TonePreset::ALL.into_iter().find(|t| t.name() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldSettings {
    pub slot_minutes: SlotMinutes,
    pub movement: MovementSettings,
    pub tone: TonePreset,
}

impl Default for WorldSettings {
    fn default() -> Self {
        WorldSettings {
            slot_minutes: SlotMinutes::DEFAULT,
            movement: MovementSettings::default(),
            tone: TonePreset::Standard,
        }
    }
}

/// **Dev scaffolding.** A tiny piece of state mutated by the dev systems and the `DevNudge` command
/// so replay, hashing and snapshot tests are meaningful before real simulation systems exist.
/// Removed when the Stage 1 systems land (see `docs/TODO.md`, D-009).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub value: i64,
    pub minutes: u64,
    pub days: u64,
}

/// The entities whose rows differ between two worlds in `table`: present in only one, or different in
/// both. Empty for tables without rows or when the table is identical.
pub fn differing_rows(a: &WorldState, b: &WorldState, table: &str) -> Vec<String> {
    let (Some(ra), Some(rb)) = (a.row_hashes(table), b.row_hashes(table)) else {
        return Vec::new();
    };
    let mb: BTreeMap<&String, &StateHash> = rb.iter().map(|(k, h)| (k, h)).collect();
    let ma: BTreeMap<&String, &StateHash> = ra.iter().map(|(k, h)| (k, h)).collect();
    let mut out: Vec<String> = ra
        .iter()
        .filter(|(k, h)| mb.get(k) != Some(&h))
        .map(|(k, _)| k.clone())
        .collect();
    out.extend(
        rb.iter()
            .filter(|(k, _)| !ma.contains_key(k))
            .map(|(k, _)| k.clone()),
    );
    out.sort();
    out
}

/// Why a world mutation was refused. A refused mutation changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldError {
    Map(MapError),
    Ids(IdsExhausted),
    Occupancy(OccupancyError),
    UnknownMap(EntityId),
    UnknownPawn(EntityId),
    /// The tile cannot be stood on (blocked, impassable terrain or out of bounds).
    NotPassable(Tile),
    BadName(String),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldError::Map(e) => e.fmt(f),
            WorldError::Ids(e) => e.fmt(f),
            WorldError::Occupancy(e) => e.fmt(f),
            WorldError::UnknownMap(m) => write!(f, "no map {m}"),
            WorldError::UnknownPawn(p) => write!(f, "no pawn {p}"),
            WorldError::NotPassable(t) => write!(f, "tile {t} cannot be stood on"),
            WorldError::BadName(n) => write!(
                f,
                "'{n}' is not a valid name (1-32 characters, no control characters)"
            ),
        }
    }
}

impl std::error::Error for WorldError {}

impl From<MapError> for WorldError {
    fn from(e: MapError) -> Self {
        WorldError::Map(e)
    }
}

impl From<IdsExhausted> for WorldError {
    fn from(e: IdsExhausted) -> Self {
        WorldError::Ids(e)
    }
}

impl From<OccupancyError> for WorldError {
    fn from(e: OccupancyError) -> Self {
        WorldError::Occupancy(e)
    }
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
    pub maps: Table<MapData>,
    pub objects: Table<ObjectInstance>,
    pub pawns: Table<Pawn>,
    pub households: Table<Household>,
    pub relationships: Relationships,
    pub commitments: Table<Commitment>,
    pub town: Town,
    pub probe: Probe,
    /// Pack-defined component values and pack health (empty without packs, and then not hashed).
    pub ext: crate::ext::ExtStore,
    /// Derived from `pawns` and `maps`; never hashed or saved. Call [`WorldState::rebuild_derived`] after
    /// loading.
    pub occupancy: Occupancy,
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
            maps: Table::new(),
            objects: Table::new(),
            pawns: Table::new(),
            households: Table::new(),
            relationships: Relationships::new(),
            commitments: Table::new(),
            town: Town::default(),
            probe: Probe::default(),
            ext: crate::ext::ExtStore::new(),
            occupancy: Occupancy::new(),
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

    /// Rebuilds everything derived from authoritative state (currently the occupancy grids).
    pub fn rebuild_derived(&mut self) {
        let mut occ = Occupancy::new();
        for (_, map) in self.maps.iter() {
            occ.add_map(map);
        }
        for (id, pawn) in self.pawns.iter() {
            // A corrupt save could stack pawns; the first (lowest id) keeps the tile, validation reports the rest.
            let _ = occ.place(pawn.position.map, pawn.position.tile, id);
        }
        self.occupancy = occ;
    }

    /// Creates an empty (all-grass) map and returns its id.
    pub fn create_map(&mut self, kind: MapKind, w: i32, h: i32) -> Result<EntityId, WorldError> {
        // Validate the size before consuming an id.
        MapData::new(EntityId::new(Kind::Map, 0), kind, w, h)?;
        let id = self.id_counters.allocate(Kind::Map)?;
        let map = MapData::new(id, kind, w, h)?;
        self.occupancy.add_map(&map);
        // Fresh ids are unique, so this cannot collide.
        let _ = self.maps.insert(id, map);
        Ok(id)
    }

    /// Whether a pawn could stand on `tile` of `map` (in bounds, not blocked, passable terrain).
    pub fn is_passable(&self, map: EntityId, tile: Tile) -> bool {
        self.maps
            .get(map)
            .is_some_and(|m| MoveCosts::default().step_cost(m, tile).is_some())
    }

    /// Creates a pawn standing on `tile`. Refused if the tile is impassable, taken, or the name is invalid.
    pub fn spawn_pawn(
        &mut self,
        name: &str,
        map: EntityId,
        tile: Tile,
    ) -> Result<EntityId, WorldError> {
        if name.trim().is_empty() || name.chars().count() > 32 || name.chars().any(char::is_control)
        {
            return Err(WorldError::BadName(name.to_owned()));
        }
        if !self.maps.contains(map) {
            return Err(WorldError::UnknownMap(map));
        }
        if !self.is_passable(map, tile) {
            return Err(WorldError::NotPassable(tile));
        }
        if let Some(by) = self.occupancy.occupant(map, tile) {
            return Err(OccupancyError::Occupied { tile, by }.into());
        }
        let id = self.id_counters.allocate(Kind::Pawn)?;
        self.occupancy.place(map, tile, id)?;
        let pawn = Pawn::new(id, name, Position { map, tile });
        let _ = self.pawns.insert(id, pawn);
        Ok(id)
    }

    /// Per-row hashes of an entity table (`maps`, `objects`, `pawns`, `commitments`), in id order, or
    /// `None` for a table that has no rows (S-023).
    pub fn row_hashes(&self, table: &str) -> Option<Vec<(String, StateHash)>> {
        fn rows<T: ToCanon>(t: &Table<T>) -> Vec<(String, StateHash)> {
            t.iter()
                .map(|(id, row)| (id.to_string(), hash_value(row)))
                .collect()
        }
        match table {
            "maps" => Some(rows(&self.maps)),
            "objects" => Some(rows(&self.objects)),
            "pawns" => Some(rows(&self.pawns)),
            "households" => Some(rows(&self.households)),
            "relationships" => Some(
                self.relationships
                    .iter()
                    .map(|r| (r.key.text(), hash_value(r)))
                    .collect(),
            ),
            "commitments" => Some(rows(&self.commitments)),
            "ext" if !self.ext.is_empty() => Some(self.ext.row_hashes()),
            _ => None,
        }
    }

    /// Per-table hashes in a fixed order. Entity tables hash as the combination of their rows.
    pub fn table_hashes(&self) -> Vec<(&'static str, StateHash)> {
        let table = |name: &str| combine_row_hashes(&self.row_hashes(name).unwrap_or_default());
        let mut tables = vec![
            ("clock", hash_value(&self.clock)),
            ("commitments", table("commitments")),
            ("households", table("households")),
            ("id_counters", hash_value(&self.id_counters)),
            ("maps", table("maps")),
            ("meta", hash_value(&self.meta)),
            ("objects", table("objects")),
            ("pawns", table("pawns")),
            ("probe", hash_value(&self.probe)),
            ("relationships", table("relationships")),
            ("rng_counters", hash_value(&self.rng_counters)),
            ("settings", hash_value(&self.settings)),
            ("town", hash_value(&self.town)),
        ];
        // Extension data joins the hash only when there is some, so worlds without packs keep their hashes.
        if !self.ext.is_empty() {
            tables.insert(3, ("ext", table("ext")));
        }
        tables
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

impl ToCanon for MovementSettings {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("max_repaths", self.max_repaths.to_canon()),
            ("max_wait_ticks", self.max_wait_ticks.to_canon()),
            ("move_ticks_per_tile", self.move_ticks_per_tile.to_canon()),
            ("path_expansion_cap", self.path_expansion_cap.to_canon()),
        ])
    }
}

impl ToCanon for WorldSettings {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("movement", self.movement.to_canon()),
            ("slot_minutes", self.slot_minutes.get().to_canon()),
            ("tone", Canon::str(self.tone.name())),
        ])
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

impl ToCanon for WorldState {
    /// The authoritative state. `occupancy` is derived and deliberately absent.
    fn to_canon(&self) -> Canon {
        let mut doc = Canon::map([
            ("schema", self.schema.to_canon()),
            ("meta", self.meta.to_canon()),
            ("settings", self.settings.to_canon()),
            ("clock", self.clock.to_canon()),
            ("id_counters", self.id_counters.to_canon()),
            ("rng_counters", self.rng_counters.to_canon()),
            ("maps", self.maps.to_canon()),
            ("objects", self.objects.to_canon()),
            ("pawns", self.pawns.to_canon()),
            ("households", self.households.to_canon()),
            ("relationships", self.relationships.to_canon()),
            ("commitments", self.commitments.to_canon()),
            ("town", self.town.to_canon()),
            ("probe", self.probe.to_canon()),
        ]);
        if let (false, Canon::Map(m)) = (self.ext.is_empty(), &mut doc) {
            m.insert("ext".to_owned(), self.ext.to_canon());
        }
        doc
    }
}

/// Why a saved world could not be restored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// The save was written by a different schema than this build reads (migrate it first).
    Schema {
        found: u32,
        expected: u32,
    },
    Read(ReadError),
    /// A table key does not match the id inside the row.
    KeyMismatch {
        table: &'static str,
        key: String,
    },
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreError::Schema { found, expected } => write!(
                f,
                "save schema {found} cannot be read directly (this build reads {expected})"
            ),
            RestoreError::Read(e) => write!(f, "{e}"),
            RestoreError::KeyMismatch { table, key } => {
                write!(f, "{table}: key '{key}' does not match the row's id")
            }
        }
    }
}

impl std::error::Error for RestoreError {}

impl From<ReadError> for RestoreError {
    fn from(e: ReadError) -> Self {
        RestoreError::Read(e)
    }
}

fn read_table<T>(
    r: &Reader<'_>,
    name: &'static str,
    decode: impl Fn(Reader<'_>) -> Result<T, ReadError>,
    id_of: impl Fn(&T) -> EntityId,
) -> Result<Table<T>, RestoreError> {
    let mut table = Table::new();
    for (key, child) in r.child(name)?.reader().entries()? {
        let row = decode(child.reader())?;
        if id_of(&row).to_string() != key {
            return Err(RestoreError::KeyMismatch { table: name, key });
        }
        // Keys are unique in an object, so the row ids are too.
        let _ = table.insert(id_of(&row), row);
    }
    Ok(table)
}

impl WorldState {
    /// Decodes a world from its canonical form and rebuilds derived state. Unknown fields, bad ranges,
    /// inconsistent tables and wrong schemas are refused with a path to the problem; the caller (the
    /// persistence layer) runs migrations first and `validate_containment` afterwards.
    pub fn from_canon(c: &Canon) -> Result<WorldState, RestoreError> {
        let r = Reader::new(c, "");
        r.only(&[
            "schema",
            "meta",
            "settings",
            "clock",
            "id_counters",
            "rng_counters",
            "maps",
            "objects",
            "pawns",
            "households",
            "relationships",
            "commitments",
            "town",
            "probe",
            "ext",
        ])?;
        let found = r.child("schema")?.reader().u32()?;
        if found != SCHEMA_VERSION {
            return Err(RestoreError::Schema {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        let meta_c = r.child("meta")?;
        let meta_r = meta_c.reader();
        meta_r.only(&["name", "seed_text"])?;
        let meta = WorldMeta {
            name: meta_r.child("name")?.reader().str()?.to_owned(),
            seed_text: meta_r.child("seed_text")?.reader().str()?.to_owned(),
        };
        let settings_c = r.child("settings")?;
        let s = settings_c.reader();
        s.only(&["movement", "slot_minutes", "tone"])?;
        let m_c = s.child("movement")?;
        let m = m_c.reader();
        m.only(&[
            "max_repaths",
            "max_wait_ticks",
            "move_ticks_per_tile",
            "path_expansion_cap",
        ])?;
        let slot_c = s.child("slot_minutes")?;
        let tone_c = s.child("tone")?;
        let settings = WorldSettings {
            tone: TonePreset::parse(tone_c.reader().str()?)
                .ok_or_else(|| tone_c.reader().err("expected cozy, standard or mature"))?,
            slot_minutes: SlotMinutes::new(slot_c.reader().u32()?)
                .map_err(|e| slot_c.reader().err(e.to_string()))?,
            movement: MovementSettings {
                max_repaths: m.child("max_repaths")?.reader().u32()?,
                max_wait_ticks: m.child("max_wait_ticks")?.reader().u32()?,
                move_ticks_per_tile: m.child("move_ticks_per_tile")?.reader().u32()?,
                path_expansion_cap: m.child("path_expansion_cap")?.reader().u32()?,
            },
        };
        let clock_c = r.child("clock")?;
        clock_c.reader().only(&["tick"])?;
        let clock = Clock::from_tick(clock_c.reader().child("tick")?.reader().u64()?);
        let mut rng_counters = BTreeMap::new();
        for (name, child) in r.child("rng_counters")?.reader().entries()? {
            rng_counters.insert(name, child.reader().u32()?);
        }
        let probe_c = r.child("probe")?;
        let p = probe_c.reader();
        p.only(&["days", "minutes", "value"])?;
        let probe = Probe {
            days: p.child("days")?.reader().u64()?,
            minutes: p.child("minutes")?.reader().u64()?,
            value: p.child("value")?.reader().i64()?,
        };
        let mut world = WorldState {
            schema: found,
            meta,
            settings,
            clock,
            id_counters: IdCounters::from_reader(r.child("id_counters")?.reader())?,
            rng_counters,
            maps: read_table(&r, "maps", MapData::from_reader, |m| m.id)?,
            objects: read_table(&r, "objects", ObjectInstance::from_reader, |o| o.id)?,
            pawns: read_table(&r, "pawns", Pawn::from_reader, |p| p.id)?,
            households: read_table(&r, "households", Household::from_reader, |h| h.id)?,
            relationships: {
                let mut rels = Relationships::new();
                for (key, child) in r.child("relationships")?.reader().entries()? {
                    let rel = Relationship::from_reader(child.reader())?;
                    if rel.key.text() != key {
                        return Err(RestoreError::KeyMismatch {
                            table: "relationships",
                            key,
                        });
                    }
                    rels.set(rel);
                }
                rels
            },
            commitments: read_table(&r, "commitments", Commitment::from_reader, |c| c.id)?,
            town: Town::from_reader(r.child("town")?.reader())?,
            probe,
            ext: match r.maybe("ext")? {
                Some(c) => crate::ext::ExtStore::from_reader(c.reader())?,
                None => crate::ext::ExtStore::new(),
            },
            occupancy: Occupancy::new(),
        };
        world.rebuild_derived();
        Ok(world)
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
            r#"{"clock":{"tick":0},"commitments":{},"households":{},"id_counters":{},"maps":{},"meta":{"name":"Town","seed_text":"seed"},"objects":{},"pawns":{},"probe":{"days":0,"minutes":0,"value":0},"relationships":{},"rng_counters":{},"schema":4,"settings":{"movement":{"max_repaths":3,"max_wait_ticks":20,"move_ticks_per_tile":2,"path_expansion_cap":20000},"slot_minutes":30,"tone":"standard"},"town":{"buildings":[],"districts":[],"gathering":[],"starting_hash":null}}"#
        );
    }

    #[test]
    fn maps_and_pawns_are_created_with_fresh_ids_and_occupancy() {
        let mut w = WorldState::new("Town", "seed");
        let m = w.create_map(MapKind::Overworld, 8, 8).unwrap();
        assert_eq!(m.to_string(), "map_1");
        let p = w.spawn_pawn("Ann", m, Tile::new(2, 3)).unwrap();
        assert_eq!(p.to_string(), "pawn_1");
        assert_eq!(w.occupancy.occupant(m, Tile::new(2, 3)), Some(p));
        assert_eq!(w.pawns.get(p).unwrap().position.tile, Tile::new(2, 3));
    }

    #[test]
    fn creation_failures_change_nothing_and_do_not_burn_ids() {
        let mut w = WorldState::new("Town", "seed");
        assert!(w.create_map(MapKind::Overworld, 0, 5).is_err());
        let m = w.create_map(MapKind::Overworld, 4, 4).unwrap();
        assert_eq!(m.counter(), 1, "a refused map must not consume an id");
        let before = w.state_hash();
        assert!(matches!(
            w.spawn_pawn("", m, Tile::new(0, 0)),
            Err(WorldError::BadName(_))
        ));
        assert!(matches!(
            w.spawn_pawn("x\ny", m, Tile::new(0, 0)),
            Err(WorldError::BadName(_))
        ));
        assert!(matches!(
            w.spawn_pawn("Ann", m, Tile::new(9, 9)),
            Err(WorldError::NotPassable(_))
        ));
        assert!(matches!(
            w.spawn_pawn("Ann", EntityId::new(Kind::Map, 7), Tile::new(0, 0)),
            Err(WorldError::UnknownMap(_))
        ));
        w.maps
            .get_mut(m)
            .unwrap()
            .set_blocked(Tile::new(1, 1), true)
            .unwrap();
        assert!(matches!(
            w.spawn_pawn("Ann", m, Tile::new(1, 1)),
            Err(WorldError::NotPassable(_))
        ));
        let a = w.spawn_pawn("Ann", m, Tile::new(0, 0)).unwrap();
        assert!(matches!(
            w.spawn_pawn("Bob", m, Tile::new(0, 0)),
            Err(WorldError::Occupancy(_))
        ));
        assert_eq!(a.counter(), 1, "failed spawns must not consume pawn ids");
        assert_ne!(before, w.state_hash(), "the successful spawn changed state");
    }

    #[test]
    fn occupancy_is_derived_and_rebuilds_from_the_tables() {
        let mut w = WorldState::new("Town", "seed");
        let m = w.create_map(MapKind::Overworld, 6, 6).unwrap();
        let a = w.spawn_pawn("Ann", m, Tile::new(1, 1)).unwrap();
        let b = w.spawn_pawn("Bob", m, Tile::new(4, 4)).unwrap();
        let hash = w.state_hash();
        let expected = w.occupancy.clone();
        w.occupancy = Occupancy::new();
        w.rebuild_derived();
        assert_eq!(w.occupancy, expected);
        assert_eq!(w.occupancy.occupant(m, Tile::new(1, 1)), Some(a));
        assert_eq!(w.occupancy.occupant(m, Tile::new(4, 4)), Some(b));
        assert_eq!(w.state_hash(), hash, "occupancy is not part of the hash");
    }

    fn world_with_pawns() -> WorldState {
        let mut w = WorldState::new("Rows", "seed");
        let m = w.create_map(MapKind::Overworld, 8, 8).unwrap();
        for (i, name) in ["Ann", "Bob", "Cy"].iter().enumerate() {
            w.spawn_pawn(name, m, Tile::new(i as i32, 0)).unwrap();
        }
        w
    }

    #[test]
    fn row_hashes_name_the_entity_that_changed() {
        let a = world_with_pawns();
        let mut b = a.clone();
        assert!(differing_rows(&a, &b, "pawns").is_empty());
        assert_eq!(a.state_hash(), b.state_hash());
        let id = EntityId::new(Kind::Pawn, 2);
        b.pawns.get_mut(id).unwrap().name = "Robert".into();
        assert_ne!(a.state_hash(), b.state_hash());
        assert_eq!(differing_rows(&a, &b, "pawns"), ["pawn_2"]);
        assert!(differing_rows(&a, &b, "maps").is_empty());
        // Added and removed rows are reported too, in id order.
        b.pawns.remove(EntityId::new(Kind::Pawn, 3));
        let m = EntityId::new(Kind::Map, 1);
        b.spawn_pawn("Dee", m, Tile::new(5, 5)).unwrap();
        assert_eq!(
            differing_rows(&a, &b, "pawns"),
            ["pawn_2", "pawn_3", "pawn_4"]
        );
        // Tables without rows have no row hashes.
        assert!(a.row_hashes("clock").is_none() && differing_rows(&a, &b, "clock").is_empty());
        assert_eq!(a.row_hashes("pawns").unwrap().len(), 3);
    }

    #[test]
    fn households_relationships_and_resident_fields_are_hashed_and_localised() {
        use crate::social::{Household, PairKey, Relationship};
        let a = world_with_pawns();
        let mut b = a.clone();
        let (p1, p2) = (EntityId::new(Kind::Pawn, 1), EntityId::new(Kind::Pawn, 2));
        let hh = b.id_counters.allocate(Kind::Household).unwrap();
        b.households
            .insert(
                hh,
                Household {
                    id: hh,
                    name: "Lee".into(),
                    members: vec![p1, p2],
                    home: None,
                },
            )
            .unwrap();
        let changed = |x: &WorldState, y: &WorldState| -> Vec<&'static str> {
            x.table_hashes()
                .into_iter()
                .zip(y.table_hashes())
                .filter(|(m, n)| m.1 != n.1)
                .map(|(m, _)| m.0)
                .collect()
        };
        assert_eq!(changed(&a, &b), ["households", "id_counters"]);
        assert_eq!(differing_rows(&a, &b, "households"), ["hh_1"]);
        let mut c = b.clone();
        c.relationships.set(Relationship {
            key: PairKey::new(p1, p2).unwrap(),
            affinity: 300,
            label: "friendly".into(),
            last_interaction_tick: 0,
            day: 0,
            day_change: 0,
        });
        assert_eq!(changed(&b, &c), ["relationships"]);
        assert_eq!(differing_rows(&b, &c, "relationships"), ["pawn_1+pawn_2"]);
        // Fields on a pawn change that pawn's row and nothing else.
        let mut d = c.clone();
        d.pawns.get_mut(p1).unwrap().mood = "cheerful".into();
        d.pawns
            .get_mut(p1)
            .unwrap()
            .needs
            .insert("hunger".into(), 640);
        assert_eq!(changed(&c, &d), ["pawns"]);
        assert_eq!(differing_rows(&c, &d, "pawns"), ["pawn_1"]);
        // And all of it survives a save and load.
        let back = WorldState::from_canon(
            &crate::canon::json::parse(&d.to_canon().to_canonical_string()).unwrap(),
        )
        .unwrap();
        assert_eq!(back.state_hash(), d.state_hash());
        assert_eq!(back.households.len(), 1);
    }

    #[test]
    fn a_relationship_row_under_the_wrong_key_is_refused_on_load() {
        let mut w = world_with_pawns();
        w.relationships.set(crate::social::Relationship {
            key: crate::social::PairKey::new(
                EntityId::new(Kind::Pawn, 1),
                EntityId::new(Kind::Pawn, 2),
            )
            .unwrap(),
            affinity: 0,
            label: "stranger".into(),
            last_interaction_tick: 0,
            day: 0,
            day_change: 0,
        });
        let text = w
            .to_canon()
            .to_canonical_string()
            .replace("\"pawn_1+pawn_2\":{", "\"pawn_1+pawn_3\":{");
        let err = WorldState::from_canon(&crate::canon::json::parse(&text).unwrap()).unwrap_err();
        assert!(
            matches!(
                err,
                RestoreError::KeyMismatch {
                    table: "relationships",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_table_hash_depends_on_its_rows_and_their_ids_not_on_incidental_order() {
        let a = world_with_pawns();
        let b = world_with_pawns();
        assert_eq!(a.table_hashes(), b.table_hashes());
        // Renaming an id (same content) changes the table hash.
        let mut c = a.clone();
        let row = c.pawns.remove(EntityId::new(Kind::Pawn, 1)).unwrap();
        let mut moved = row.clone();
        moved.id = EntityId::new(Kind::Pawn, 9);
        c.pawns.insert(moved.id, moved).unwrap();
        assert_ne!(
            a.table_hashes().iter().find(|(n, _)| *n == "pawns"),
            c.table_hashes().iter().find(|(n, _)| *n == "pawns")
        );
    }
}
