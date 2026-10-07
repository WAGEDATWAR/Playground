//! World export and import (Blueprint §13.6).
//!
//! An export is one canonical-JSON document: a small header, the world's canonical state, and a BLAKE3 hash
//! of that state. It is built from the validated world model only, so anything outside the schema (and
//! therefore any secret) cannot be in it.
//!
//! **Import pipeline:** size check -> parse (strict, integers only) -> format and kind -> hash -> schema
//! detect -> migrate -> decode and validate structure -> validate containment -> a [`ValidationReport`]
//! and a content compatibility report for the player to confirm -> write as a **new slot**. A failure at
//! any step leaves existing data untouched; nothing is written until [`commit_import`]. Entity ids are
//! kept as they are: they only have meaning inside one world, and an imported world is always a new slot.

use crate::compat::{self, refs_from_reader, refs_to_canon, CompatReport, ContentRefRecord};
use crate::migrate::{MigrateError, Migrations};
use crate::store::{valid_world_id, SaveError, SaveReport, SlotStore};
use pg_content::{ContentSet, ValidationReport};
use pg_core::canon::{json, Canon, ToCanon};
use pg_core::containment::validate_containment;
use pg_core::read::{ReadError, Root};
use pg_core::world::WorldState;
use std::fmt;

pub const EXPORT_FORMAT: &str = "playground-export";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BundleKind {
    World,
}

impl BundleKind {
    pub const fn name(self) -> &'static str {
        match self {
            BundleKind::World => "world",
        }
    }
}

/// Builds an export of `world` as canonical JSON bytes.
pub fn export_world(
    world: &WorldState,
    content_refs: &[ContentRefRecord],
    app_version: &str,
    created_iso: &str,
) -> Vec<u8> {
    let payload = world.to_canon();
    let hash = pg_core::hash::hash_canon(&payload).to_hex();
    Canon::map([
        ("format", Canon::str(EXPORT_FORMAT)),
        ("kind", Canon::str(BundleKind::World.name())),
        ("schema", world.schema.to_canon()),
        ("app_version", Canon::str(app_version)),
        ("created_iso", Canon::str(created_iso)),
        ("content_refs", refs_to_canon(content_refs)),
        ("payload", payload),
        ("hash", Canon::str(hash)),
    ])
    .to_canonical_string()
    .into_bytes()
}

#[derive(Debug)]
pub enum ImportError {
    TooLarge {
        bytes: usize,
        limit: usize,
    },
    NotJson(String),
    /// Not an export file, or an export of a kind this build does not import.
    WrongFormat(String),
    /// The payload does not match its hash: damaged or edited after export.
    HashMismatch,
    Migrate(MigrateError),
    /// The state is structurally invalid (with the path to the problem).
    Invalid(String),
    /// The state decoded but failed validation; the report lists why.
    Rejected(ValidationReport),
    SlotExists(String),
    BadWorldId(String),
    Save(SaveError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::TooLarge { bytes, limit } => {
                write!(
                    f,
                    "the file is {bytes} bytes; imports are limited to {limit}"
                )
            }
            ImportError::NotJson(e) => write!(f, "the file is not valid export data: {e}"),
            ImportError::WrongFormat(e) => write!(f, "{e}"),
            ImportError::HashMismatch => write!(
                f,
                "the file's contents do not match its hash; it was damaged or edited"
            ),
            ImportError::Migrate(e) => write!(f, "{e}"),
            ImportError::Invalid(e) => write!(f, "the world data is invalid: {e}"),
            ImportError::Rejected(r) => write!(f, "the world failed validation:\n{r}"),
            ImportError::SlotExists(id) => write!(
                f,
                "a world named '{id}' already exists; choose another name or overwrite explicitly"
            ),
            ImportError::BadWorldId(id) => write!(f, "'{id}' is not a valid world id"),
            ImportError::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {}

pub struct ImportOptions<'a> {
    pub max_bytes: usize,
    pub migrations: Migrations,
    pub content: Option<&'a ContentSet>,
    pub installed_refs: Option<Vec<ContentRefRecord>>,
}

impl Default for ImportOptions<'_> {
    fn default() -> Self {
        ImportOptions {
            max_bytes: 128 * 1024 * 1024,
            migrations: Migrations::builtin(),
            content: None,
            installed_refs: None,
        }
    }
}

/// A validated import waiting for the player's confirmation. Nothing has been written yet.
pub struct ImportPlan {
    pub world: WorldState,
    pub app_version: String,
    pub created_iso: String,
    pub content_refs: Vec<ContentRefRecord>,
    pub migrated_from: Option<u32>,
    /// Warnings only: errors become [`ImportError::Rejected`].
    pub report: ValidationReport,
    pub compat: Option<CompatReport>,
}

fn invalid(e: ReadError) -> ImportError {
    ImportError::Invalid(e.to_string())
}

/// Runs the import pipeline up to (not including) writing.
pub fn import_world(bytes: &[u8], opts: &ImportOptions<'_>) -> Result<ImportPlan, ImportError> {
    if bytes.len() > opts.max_bytes {
        return Err(ImportError::TooLarge {
            bytes: bytes.len(),
            limit: opts.max_bytes,
        });
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| ImportError::NotJson("not UTF-8".to_owned()))?;
    let doc = json::parse(text).map_err(|e| ImportError::NotJson(e.to_string()))?;
    let root = Root::new(doc);
    let r = root.reader();
    r.only(&[
        "format",
        "kind",
        "schema",
        "app_version",
        "created_iso",
        "content_refs",
        "payload",
        "hash",
    ])
    .map_err(|e| ImportError::WrongFormat(e.to_string()))?;
    let field = |name: &str| {
        r.child(name)
            .map_err(|e| ImportError::WrongFormat(e.to_string()))
    };
    if field("format")?.reader().str().map_err(invalid)? != EXPORT_FORMAT {
        return Err(ImportError::WrongFormat(
            "this is not a Playground export file".to_owned(),
        ));
    }
    let kind = field("kind")?;
    if kind.reader().str().map_err(invalid)? != BundleKind::World.name() {
        return Err(ImportError::WrongFormat(format!(
            "this export holds '{}', not a world",
            kind.reader().str().map_err(invalid)?
        )));
    }
    let payload = field("payload")?;
    let expected = field("hash")?;
    if pg_core::hash::hash_canon(payload.reader().value()).to_hex()
        != expected.reader().str().map_err(invalid)?
    {
        return Err(ImportError::HashMismatch);
    }
    let schema = field("schema")?.reader().u32().map_err(invalid)?;
    let migrated = opts
        .migrations
        .migrate(payload.reader().value().clone(), schema)
        .map_err(ImportError::Migrate)?;
    let world =
        WorldState::from_canon(&migrated).map_err(|e| ImportError::Invalid(e.to_string()))?;
    let content_refs = refs_from_reader(field("content_refs")?.reader()).map_err(invalid)?;
    let mut report = ValidationReport::new();
    if let Some(content) = opts.content {
        report.merge(validate_containment(&world, content));
    }
    if !report.is_ok() {
        return Err(ImportError::Rejected(report));
    }
    let compat = opts
        .installed_refs
        .as_ref()
        .map(|installed| compat::compare(&content_refs, installed));
    Ok(ImportPlan {
        world,
        app_version: field("app_version")?
            .reader()
            .str()
            .map_err(invalid)?
            .to_owned(),
        created_iso: field("created_iso")?
            .reader()
            .str()
            .map_err(invalid)?
            .to_owned(),
        content_refs,
        migrated_from: (schema != opts.migrations.current()).then_some(schema),
        report,
        compat,
    })
}

/// Writes a confirmed import as a new slot. Refuses an existing world id unless `overwrite` is set.
pub fn commit_import(
    store: &SlotStore<'_>,
    world_id: &str,
    plan: &ImportPlan,
    saved_iso: &str,
    overwrite: bool,
) -> Result<SaveReport, ImportError> {
    if !valid_world_id(world_id) {
        return Err(ImportError::BadWorldId(world_id.to_owned()));
    }
    if !overwrite
        && store
            .list_worlds()
            .is_ok_and(|w| w.iter().any(|x| x == world_id))
    {
        return Err(ImportError::SlotExists(world_id.to_owned()));
    }
    store
        .save(world_id, &plan.world, &plan.content_refs, saved_iso)
        .map_err(ImportError::Save)
}

#[cfg(test)]
mod tests;
