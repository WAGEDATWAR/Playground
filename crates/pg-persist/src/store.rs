//! World slots: the manifest, two generations, the atomic save procedure and recovery on load
//! (Blueprint §13.1, §13.2, §13.4).
//!
//! ```text
//! worlds/<world_id>/manifest.json         names the generations; written LAST (the commit point)
//! worlds/<world_id>/state.<gen>.pgsave    one file per generation
//! worlds/<world_id>/damaged.json          present only after every generation failed to load
//! ```
//!
//! **Save:** write generation `g+1`, then the manifest pointing at it, then delete generation `g-1`. A
//! failure before the manifest write leaves the previous manifest and generation untouched; a failure after
//! it leaves a complete new save plus, at worst, one extra old file that the next save removes.
//!
//! **Load:** try the manifest's generations newest first, then any other generation file found, newest
//! first. The first that passes checksum, schema, decode and validation wins; falling back is reported.
//! If none works the slot is marked damaged and its files are kept.

use crate::codec::{self, CodecError};
use crate::compat::{self, refs_from_reader, refs_to_canon, CompatReport, ContentRefRecord};
use crate::migrate::{MigrateError, Migrations};
use pg_content::ContentSet;
use pg_core::canon::{json, Canon, ToCanon};
use pg_core::containment::validate_containment;
use pg_core::read::{ReadError, Reader, Root};
use pg_core::world::WorldState;
use pg_host::Storage;
use std::fmt;

pub const MANIFEST_FORMAT: &str = "playground-world";

/// A world id: lowercase letters, digits, `_` and `-`, 1 to 40 characters.
pub fn valid_world_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn dir(id: &str) -> String {
    format!("worlds/{id}")
}

fn manifest_name(id: &str) -> String {
    format!("worlds/{id}/manifest.json")
}

fn state_name(id: &str, generation: u64) -> String {
    format!("worlds/{id}/state.{generation}.pgsave")
}

fn damaged_name(id: &str) -> String {
    format!("worlds/{id}/damaged.json")
}

/// What the manifest knows about one generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationInfo {
    pub generation: u64,
    pub state_hash: String,
    pub play_ticks: u64,
    pub day: u64,
    pub saved_iso: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub seed_text: String,
    pub schema: u32,
    pub content_refs: Vec<ContentRefRecord>,
    /// Newest first; at most two.
    pub generations: Vec<GenerationInfo>,
}

impl Manifest {
    pub fn current(&self) -> Option<&GenerationInfo> {
        self.generations.first()
    }
}

impl ToCanon for Manifest {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("format", Canon::str(MANIFEST_FORMAT)),
            ("name", Canon::str(self.name.clone())),
            ("seed_text", Canon::str(self.seed_text.clone())),
            ("schema", self.schema.to_canon()),
            ("content_refs", refs_to_canon(&self.content_refs)),
            (
                "generations",
                Canon::List(
                    self.generations
                        .iter()
                        .map(|g| {
                            Canon::map([
                                ("generation", g.generation.to_canon()),
                                ("state_hash", Canon::str(g.state_hash.clone())),
                                ("play_ticks", g.play_ticks.to_canon()),
                                ("day", g.day.to_canon()),
                                ("saved_iso", Canon::str(g.saved_iso.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

impl Manifest {
    pub fn from_reader(r: Reader<'_>) -> Result<Manifest, ReadError> {
        r.only(&[
            "format",
            "name",
            "seed_text",
            "schema",
            "content_refs",
            "generations",
        ])?;
        if r.child("format")?.reader().str()? != MANIFEST_FORMAT {
            return Err(r.err("not a world manifest"));
        }
        let generations = r
            .child("generations")?
            .reader()
            .list()?
            .iter()
            .map(|g| {
                let g = g.reader();
                g.only(&["generation", "state_hash", "play_ticks", "day", "saved_iso"])?;
                Ok(GenerationInfo {
                    generation: g.child("generation")?.reader().u64()?,
                    state_hash: g.child("state_hash")?.reader().str()?.to_owned(),
                    play_ticks: g.child("play_ticks")?.reader().u64()?,
                    day: g.child("day")?.reader().u64()?,
                    saved_iso: g.child("saved_iso")?.reader().str()?.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, ReadError>>()?;
        if generations.is_empty() || generations.len() > 2 {
            return Err(r.err("a manifest lists one or two generations"));
        }
        Ok(Manifest {
            name: r.child("name")?.reader().str()?.to_owned(),
            seed_text: r.child("seed_text")?.reader().str()?.to_owned(),
            schema: r.child("schema")?.reader().u32()?,
            content_refs: refs_from_reader(r.child("content_refs")?.reader())?,
            generations,
        })
    }
}

#[derive(Debug)]
pub enum SaveError {
    BadWorldId(String),
    Io(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::BadWorldId(id) => {
                write!(f, "'{id}' is not a valid world id (a-z, 0-9, _ and -)")
            }
            SaveError::Io(e) => write!(f, "could not save: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReport {
    pub generation: u64,
    pub compressed_bytes: usize,
    pub uncompressed_bytes: usize,
    /// Old files that could not be deleted after the commit (harmless; the next save retries).
    pub leftovers: Vec<String>,
}

/// How the loaded state was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The newest generation named by the manifest loaded cleanly.
    Clean,
    /// A newer generation failed; an older one was loaded instead.
    FellBack {
        wanted: u64,
        used: u64,
        reason: String,
        /// Simulated ticks lost, when the manifest knows both.
        ticks_lost: Option<u64>,
    },
    /// The manifest was missing or unreadable; the newest valid generation file was used.
    NoManifest { used: u64 },
}

#[derive(Debug)]
pub struct Loaded {
    pub world: WorldState,
    pub manifest: Option<Manifest>,
    pub generation: u64,
    pub recovery: Recovery,
    /// The schema the file was written with, if it had to be migrated.
    pub migrated_from: Option<u32>,
    pub compat: Option<CompatReport>,
}

#[derive(Debug)]
pub enum LoadError {
    /// No such world.
    NotFound,
    /// Written by a newer version: refused, and the slot is not marked damaged.
    Newer(MigrateError),
    /// Every generation failed; the slot was marked damaged and its files kept. One line per attempt.
    Damaged(Vec<String>),
    Io(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotFound => write!(f, "no such world"),
            LoadError::Newer(e) => write!(f, "{e}"),
            LoadError::Damaged(a) => write!(
                f,
                "this world's save is damaged and could not be recovered ({}). Its files were kept; export them for support.",
                a.join("; ")
            ),
            LoadError::Io(e) => write!(f, "could not read the save: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// The outcome of checking one generation file: its number and either success or the reason it failed.
pub type GenerationCheck = (u64, Result<(), String>);

/// What loading needs besides the storage.
pub struct LoadOptions<'a> {
    pub migrations: Migrations,
    /// With content, the load also runs `validate_containment` and compares content refs.
    pub content: Option<&'a ContentSet>,
    /// The content refs of the installed packs; with `Some`, the load reports how they compare with the
    /// refs recorded in the manifest.
    pub installed_refs: Option<Vec<ContentRefRecord>>,
    pub max_uncompressed: u64,
}

impl Default for LoadOptions<'_> {
    fn default() -> Self {
        LoadOptions {
            migrations: Migrations::builtin(),
            content: None,
            installed_refs: None,
            max_uncompressed: codec::DEFAULT_MAX_UNCOMPRESSED,
        }
    }
}

pub struct SlotStore<'a> {
    storage: &'a dyn Storage,
}

fn io<T>(r: std::io::Result<T>) -> Result<T, SaveError> {
    r.map_err(|e| SaveError::Io(e.to_string()))
}

/// Generation numbers of the state files present, newest first.
fn generations_on_disk(storage: &dyn Storage, id: &str) -> std::io::Result<Vec<u64>> {
    let prefix = format!("{}/state.", dir(id));
    let mut gens: Vec<u64> = storage
        .list(&prefix)?
        .into_iter()
        .filter_map(|b| {
            b.name
                .strip_prefix(&prefix)?
                .strip_suffix(".pgsave")?
                .parse()
                .ok()
        })
        .collect();
    gens.sort_unstable_by(|a, b| b.cmp(a));
    Ok(gens)
}

fn read_manifest(storage: &dyn Storage, id: &str) -> Option<Manifest> {
    let bytes = storage.read(&manifest_name(id)).ok()??;
    let text = String::from_utf8(bytes).ok()?;
    let root = Root::new(json::parse(&text).ok()?);
    Manifest::from_reader(root.reader()).ok()
}

impl<'a> SlotStore<'a> {
    pub fn new(storage: &'a dyn Storage) -> SlotStore<'a> {
        SlotStore { storage }
    }

    /// Saves `world` as the next generation of `world_id`.
    pub fn save(
        &self,
        world_id: &str,
        world: &WorldState,
        content_refs: &[ContentRefRecord],
        saved_iso: &str,
    ) -> Result<SaveReport, SaveError> {
        if !valid_world_id(world_id) {
            return Err(SaveError::BadWorldId(world_id.to_owned()));
        }
        let previous = read_manifest(self.storage, world_id);
        let on_disk = io(generations_on_disk(self.storage, world_id))?;
        let newest = previous
            .as_ref()
            .and_then(|m| m.current().map(|g| g.generation))
            .into_iter()
            .chain(on_disk.first().copied())
            .max()
            .unwrap_or(0);
        let generation = newest + 1;

        let payload = world.to_canon().to_canonical_string().into_bytes();
        let blob = codec::encode(world.schema, &payload);
        // 1. The new generation, atomically.
        io(self
            .storage
            .write_atomic(&state_name(world_id, generation), &blob))?;
        // 2. The manifest: the commit point. It keeps the previous generation as the fallback.
        let info = GenerationInfo {
            generation,
            state_hash: world.state_hash().to_hex(),
            play_ticks: world.clock.tick(),
            day: world.clock.day(),
            saved_iso: saved_iso.to_owned(),
        };
        let mut generations = vec![info];
        if let Some(prev) = previous.as_ref().and_then(|m| m.current()) {
            generations.push(prev.clone());
        } else if let Some(g) = on_disk.first() {
            // A save without a (valid) manifest: keep the newest existing file as the fallback.
            generations.push(GenerationInfo {
                generation: *g,
                state_hash: String::new(),
                play_ticks: 0,
                day: 0,
                saved_iso: String::new(),
            });
        }
        let manifest = Manifest {
            name: world.meta.name.clone(),
            seed_text: world.meta.seed_text.clone(),
            schema: world.schema,
            content_refs: content_refs.to_vec(),
            generations,
        };
        io(self.storage.write_atomic(
            &manifest_name(world_id),
            manifest.to_canon().to_canonical_string().as_bytes(),
        ))?;
        // 3. Only now remove generations the manifest no longer names, and any stale damage marker.
        let keep: Vec<u64> = manifest.generations.iter().map(|g| g.generation).collect();
        let mut leftovers = Vec::new();
        for g in on_disk.into_iter().filter(|g| !keep.contains(g)) {
            let name = state_name(world_id, g);
            if self.storage.delete(&name).is_err() {
                leftovers.push(name);
            }
        }
        let _ = self.storage.delete(&damaged_name(world_id));
        Ok(SaveReport {
            generation,
            compressed_bytes: blob.len(),
            uncompressed_bytes: payload.len(),
            leftovers,
        })
    }

    /// The manifest, if there is a valid one.
    pub fn manifest(&self, world_id: &str) -> Option<Manifest> {
        read_manifest(self.storage, world_id)
    }

    /// World ids that have any files, sorted.
    pub fn list_worlds(&self) -> Result<Vec<String>, std::io::Error> {
        let mut ids: Vec<String> = self
            .storage
            .list("worlds/")?
            .into_iter()
            .filter_map(|b| {
                Some(
                    b.name
                        .strip_prefix("worlds/")?
                        .split('/')
                        .next()?
                        .to_owned(),
                )
            })
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub fn is_marked_damaged(&self, world_id: &str) -> bool {
        matches!(self.storage.read(&damaged_name(world_id)), Ok(Some(_)))
    }

    /// Loads a world, recovering from damage where it can.
    pub fn load(&self, world_id: &str, opts: &LoadOptions<'_>) -> Result<Loaded, LoadError> {
        if !valid_world_id(world_id) {
            return Err(LoadError::NotFound);
        }
        let manifest = read_manifest(self.storage, world_id);
        let on_disk = generations_on_disk(self.storage, world_id)
            .map_err(|e| LoadError::Io(e.to_string()))?;
        if manifest.is_none() && on_disk.is_empty() {
            return Err(LoadError::NotFound);
        }
        // Candidates: the manifest's, in order, then every other file newest first.
        let mut candidates: Vec<u64> = manifest
            .iter()
            .flat_map(|m| m.generations.iter().map(|g| g.generation))
            .collect();
        for g in &on_disk {
            if !candidates.contains(g) {
                candidates.push(*g);
            }
        }
        let wanted = candidates.first().copied();
        let mut attempts = Vec::new();
        for generation in candidates {
            match self.try_generation(world_id, generation, manifest.as_ref(), opts) {
                Ok((world, migrated_from)) => {
                    let recovery = match (&manifest, wanted) {
                        (None, _) => Recovery::NoManifest { used: generation },
                        (Some(_), Some(w)) if w == generation => Recovery::Clean,
                        (Some(m), Some(w)) => Recovery::FellBack {
                            wanted: w,
                            used: generation,
                            reason: attempts.last().cloned().unwrap_or_default(),
                            ticks_lost: match (
                                m.generations.iter().find(|g| g.generation == w),
                                m.generations.iter().find(|g| g.generation == generation),
                            ) {
                                (Some(a), Some(b)) if a.play_ticks >= b.play_ticks => {
                                    Some(a.play_ticks - b.play_ticks)
                                }
                                _ => None,
                            },
                        },
                        (Some(_), None) => Recovery::Clean,
                    };
                    let compat = match (&manifest, &opts.installed_refs) {
                        (Some(m), Some(installed)) => {
                            Some(compat::compare(&m.content_refs, installed))
                        }
                        _ => None,
                    };
                    return Ok(Loaded {
                        world,
                        manifest,
                        generation,
                        recovery,
                        migrated_from,
                        compat,
                    });
                }
                Err(Attempt::Newer(e)) => return Err(LoadError::Newer(e)),
                Err(Attempt::Failed(why)) => {
                    attempts.push(format!("generation {generation}: {why}"))
                }
            }
        }
        // Nothing worked: keep every file, note the damage, never touch other slots.
        let marker = Canon::map([(
            "attempts",
            Canon::List(attempts.iter().cloned().map(Canon::Str).collect()),
        )]);
        let _ = self.storage.write_atomic(
            &damaged_name(world_id),
            marker.to_canonical_string().as_bytes(),
        );
        Err(LoadError::Damaged(attempts))
    }

    /// Checks every generation file the slot has, without loading anything into a sim: for each (newest
    /// first), whether it decodes, migrates, validates and matches its manifest hash.
    pub fn verify(
        &self,
        world_id: &str,
        opts: &LoadOptions<'_>,
    ) -> Result<Vec<GenerationCheck>, LoadError> {
        if !valid_world_id(world_id) {
            return Err(LoadError::NotFound);
        }
        let manifest = read_manifest(self.storage, world_id);
        let on_disk = generations_on_disk(self.storage, world_id)
            .map_err(|e| LoadError::Io(e.to_string()))?;
        if manifest.is_none() && on_disk.is_empty() {
            return Err(LoadError::NotFound);
        }
        Ok(on_disk
            .into_iter()
            .map(|g| {
                let result = match self.try_generation(world_id, g, manifest.as_ref(), opts) {
                    Ok(_) => Ok(()),
                    Err(Attempt::Failed(why)) => Err(why),
                    Err(Attempt::Newer(e)) => Err(e.to_string()),
                };
                (g, result)
            })
            .collect())
    }

    fn try_generation(
        &self,
        world_id: &str,
        generation: u64,
        manifest: Option<&Manifest>,
        opts: &LoadOptions<'_>,
    ) -> Result<(WorldState, Option<u32>), Attempt> {
        let name = state_name(world_id, generation);
        let bytes = self
            .storage
            .read(&name)
            .map_err(|e| Attempt::Failed(e.to_string()))?
            .ok_or_else(|| Attempt::Failed("the file is missing".to_owned()))?;
        let (schema, payload) = codec::decode(&bytes, opts.max_uncompressed)
            .map_err(|e: CodecError| Attempt::Failed(e.to_string()))?;
        let text = String::from_utf8(payload)
            .map_err(|_| Attempt::Failed("the data is not UTF-8".to_owned()))?;
        let raw =
            json::parse(&text).map_err(|e| Attempt::Failed(format!("not valid JSON: {e}")))?;
        let inner_schema = raw
            .get("schema")
            .and_then(Canon::as_i64)
            .and_then(|s| u32::try_from(s).ok());
        if inner_schema != Some(schema) {
            return Err(Attempt::Failed(
                "the header's schema and the data's schema disagree".to_owned(),
            ));
        }
        let migrated = opts.migrations.migrate(raw, schema).map_err(|e| match e {
            e @ MigrateError::Newer { .. } => Attempt::Newer(e),
            other => Attempt::Failed(other.to_string()),
        })?;
        let world =
            WorldState::from_canon(&migrated).map_err(|e| Attempt::Failed(e.to_string()))?;
        if schema == opts.migrations.current() {
            // The recorded hash must match what we decoded (catches decoder bugs and wrong-file mixups).
            if let Some(info) =
                manifest.and_then(|m| m.generations.iter().find(|g| g.generation == generation))
            {
                if !info.state_hash.is_empty() && info.state_hash != world.state_hash().to_hex() {
                    return Err(Attempt::Failed(
                        "the loaded state's hash does not match the manifest".to_owned(),
                    ));
                }
            }
        }
        if let Some(content) = opts.content {
            let report = validate_containment(&world, content);
            if !report.is_ok() {
                return Err(Attempt::Failed(format!(
                    "containment check failed: {} violation(s)",
                    report.error_count()
                )));
            }
        }
        Ok((
            world,
            (schema != opts.migrations.current()).then_some(schema),
        ))
    }
}

enum Attempt {
    Failed(String),
    Newer(MigrateError),
}

#[cfg(test)]
mod tests;
