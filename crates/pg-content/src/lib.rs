//! Content: templates, schemas, the component registry, packs and the resolved content set.
//! Blueprint §4.2–4.3, §13.6, §17, §23.3.
//!
//! Everything here is data. Nothing executes code; pack scripts are loaded by `pg-script` (milestone
//! 0.9). `pg-core` depends on this crate for types only.
#![forbid(unsafe_code)]

pub mod component;
pub mod content_set;
pub mod diff;
pub mod hints;
pub mod ids;
pub mod manifest;
pub mod pack;
pub mod report;
pub mod resolve;
pub mod schema;
pub mod settings;
pub mod strings;
pub mod template;

pub use component::{ComponentDef, ComponentRegistry, Origin};
pub use content_set::{ContentRef, ContentSet, ScriptPack, BASE_PACK};
pub use ids::{ActionId, ComponentName, IdError, PackId, Tag, TemplateId};
pub use manifest::{Capability, PackManifest, Version, VersionReq, API_VERSION, ENGINE_VERSION};
pub use pack::{load_pack, DirPack, Limits, LoadedPack, MemoryPack, PackFiles};
pub use report::{Issue, Severity, ValidationReport};
pub use resolve::{resolve_template, ResolvedTemplate};
pub use strings::Strings;
pub use template::ObjectTemplate;
