//! Content packs the player chooses (Stage 1, milestone 1.7; Blueprint section 23): finding installed packs,
//! remembering which are enabled and which capabilities were approved, deciding what to load at launch, and
//! installing a pack from a folder.
//!
//! Installed packs live in `packs/<id>/` under the data folder. What is enabled and approved is kept in
//! `mods.json` in storage. Nothing here loads scripts: it only decides which pack folders go to the content
//! loader, and says why the others did not. The base pack is always loaded. A pack that asks for more than
//! reading and data (systems, world writes, world generation, AI, developer tools) is loaded only after the
//! player has approved each of those capabilities for it. If the chosen packs cannot be loaded together, the
//! game starts with the base pack alone and says so, rather than failing to start.

use pg_content::manifest::Capability;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use pg_core::canon::{json, Canon};
use pg_host::Storage;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where the choices are kept.
pub const CONFIG: &str = "mods.json";

/// The folder (under the data folder) installed packs live in.
pub const PACKS_DIR: &str = "packs";

/// The base game's pack id; it cannot be disabled.
pub const BASE: &str = "base";

/// Capabilities that need the player's approval before a pack that asks for them is loaded.
pub fn needs_approval(c: Capability) -> bool {
    !matches!(c, Capability::Read | Capability::Data)
}

/// What the player has chosen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModsConfig {
    pub enabled: BTreeSet<String>,
    /// Pack id -> the capabilities approved for it.
    pub approved: BTreeMap<String, BTreeSet<String>>,
    /// Start the next launch with the base pack alone (cleared once used).
    pub safe_mode: bool,
}

impl ModsConfig {
    /// Reads the saved choices. A missing file is the default; a damaged one is the default and a note says so.
    pub fn load(storage: &dyn Storage) -> (ModsConfig, Option<String>) {
        let bytes = match storage.read(CONFIG) {
            Ok(Some(b)) => b,
            Ok(None) => return (ModsConfig::default(), None),
            Err(e) => {
                return (
                    ModsConfig::default(),
                    Some(format!(
                        "The mods settings could not be read ({e}); starting fresh."
                    )),
                )
            }
        };
        match ModsConfig::parse(&String::from_utf8_lossy(&bytes)) {
            Ok(c) => (c, None),
            Err(e) => (
                ModsConfig::default(),
                Some(format!(
                    "The mods settings are damaged ({e}); starting fresh."
                )),
            ),
        }
    }

    pub fn parse(text: &str) -> Result<ModsConfig, String> {
        let doc = json::parse(text).map_err(|e| e.to_string())?;
        let mut c = ModsConfig::default();
        if let Some(Canon::List(l)) = doc.get("enabled") {
            c.enabled = l
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
        }
        if let Some(Canon::Map(m)) = doc.get("approved") {
            for (pack, v) in m {
                if let Canon::List(l) = v {
                    c.approved.insert(
                        pack.clone(),
                        l.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect(),
                    );
                }
            }
        }
        c.safe_mode = matches!(doc.get("safe_mode"), Some(Canon::Bool(true)));
        Ok(c)
    }

    pub fn to_text(&self) -> String {
        let list =
            |s: &BTreeSet<String>| Canon::List(s.iter().map(|x| Canon::str(x.clone())).collect());
        Canon::map([
            ("enabled", list(&self.enabled)),
            (
                "approved",
                Canon::Map(
                    self.approved
                        .iter()
                        .map(|(k, v)| (k.clone(), list(v)))
                        .collect(),
                ),
            ),
            ("safe_mode", Canon::Bool(self.safe_mode)),
        ])
        .to_canonical_string()
    }

    pub fn save(&self, storage: &dyn Storage) -> std::io::Result<()> {
        storage.write_atomic(CONFIG, self.to_text().as_bytes())
    }

    pub fn is_approved(&self, pack: &str, cap: &str) -> bool {
        self.approved.get(pack).is_some_and(|s| s.contains(cap))
    }

    pub fn set_approved(&mut self, pack: &str, cap: &str, on: bool) {
        let set = self.approved.entry(pack.to_owned()).or_default();
        if on {
            set.insert(cap.to_owned());
        } else {
            set.remove(cap);
        }
    }
}

/// What the loader learned about one installed pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The folder under `packs/`.
    pub folder: String,
    pub path: PathBuf,
    pub outcome: Result<Summary, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Capability names, in name order.
    pub capabilities: Vec<String>,
    pub depends: Vec<String>,
    /// The content hash, the size of the pack's files in bytes and how many files it has.
    pub hash: String,
    pub size_bytes: u64,
    pub files: usize,
}

/// Looks at every folder in `<data_dir>/packs`. A folder that is not a valid pack comes back with the
/// loader's report so the Mods screen can show why. Sorted by folder name.
pub fn discover(data_dir: &Path) -> Vec<Installed> {
    let root = data_dir.join(PACKS_DIR);
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return found;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let folder = e.file_name().to_string_lossy().into_owned();
        let outcome = match load_pack(&DirPack::new(&path), &Limits::default()) {
            Ok(p) => Ok(Summary {
                id: p.manifest.id.to_string(),
                name: p.manifest.name.clone(),
                version: format!(
                    "{}.{}.{}",
                    p.manifest.version.major, p.manifest.version.minor, p.manifest.version.patch
                ),
                capabilities: p
                    .manifest
                    .capabilities
                    .iter()
                    .map(|c| c.name().to_owned())
                    .collect(),
                depends: p
                    .manifest
                    .depends
                    .iter()
                    .map(|d| d.id.to_string())
                    .collect(),
                hash: p.hash.clone(),
                size_bytes: p.total_bytes,
                files: p.file_count,
            }),
            Err(report) => Err(report.to_string()),
        };
        found.push(Installed {
            folder,
            path,
            outcome,
        });
    }
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// Why an installed pack is not going to be loaded, or `None` if it is.
pub fn skip_reason(config: &ModsConfig, s: &Summary) -> Option<String> {
    if s.id == BASE {
        return Some("the base game is always loaded".to_owned());
    }
    if !config.enabled.contains(&s.id) {
        return Some("it is not enabled".to_owned());
    }
    let missing: Vec<&str> = s
        .capabilities
        .iter()
        .filter(|c| Capability::from_name(c).is_some_and(needs_approval))
        .filter(|c| !config.is_approved(&s.id, c))
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "it needs your approval for: {}",
            missing.join(", ")
        ))
    }
}

/// The result of choosing what to load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Pack folders to load after the base pack, in folder order.
    pub dirs: Vec<PathBuf>,
    /// Things worth telling the player (packs left out, and why).
    pub notes: Vec<String>,
}

/// Decides which installed packs to load. Safe mode loads none.
pub fn plan(installed: &[Installed], config: &ModsConfig, safe_mode: bool) -> Plan {
    let mut p = Plan::default();
    if safe_mode {
        p.notes
            .push("Safe mode: only the base game was loaded.".to_owned());
        return p;
    }
    for i in installed {
        match &i.outcome {
            Err(_) => p.notes.push(format!(
                "The pack in '{}' is not valid and was left out.",
                i.folder
            )),
            Ok(s) if s.id == BASE => {}
            Ok(s) => match skip_reason(config, s) {
                None => p.dirs.push(i.path.clone()),
                Some(why) if config.enabled.contains(&s.id) => {
                    p.notes.push(format!("'{}' was left out: {why}.", s.name));
                }
                Some(_) => {}
            },
        }
    }
    p
}

/// Builds the content for a launch: the base pack, the developer's extra packs, then the packs the player
/// enabled. If they cannot be built together, the base pack alone is used and a note says why.
pub fn build_content(
    base: &Path,
    extra: &[PathBuf],
    data_dir: Option<&Path>,
    config: &ModsConfig,
    safe_mode: bool,
) -> Result<(Arc<ContentSet>, Vec<String>), String> {
    let installed = data_dir.map(discover).unwrap_or_default();
    let mut plan = plan(&installed, config, safe_mode);
    let load = |dirs: &[PathBuf]| -> Result<Arc<ContentSet>, String> {
        let mut packs = Vec::new();
        for dir in std::iter::once(base).chain(dirs.iter().map(PathBuf::as_path)) {
            match load_pack(&DirPack::new(dir), &Limits::default()) {
                Ok(p) => packs.push(p),
                Err(r) => return Err(format!("cannot load pack '{}':\n{r}", dir.display())),
            }
        }
        ContentSet::build(packs, ComponentRegistry::builtin())
            .map(Arc::new)
            .map_err(|r| format!("the content did not build:\n{r}"))
    };
    let mut dirs: Vec<PathBuf> = extra.to_vec();
    dirs.extend(plan.dirs.iter().cloned());
    match load(&dirs) {
        Ok(c) => Ok((c, plan.notes)),
        Err(why) if dirs.is_empty() => Err(why),
        Err(why) => {
            // The base game must still start.
            let first_line = why.lines().next().unwrap_or("").to_owned();
            plan.notes.push(format!(
                "The chosen packs could not be loaded together, so only the base game was loaded ({first_line})."
            ));
            load(&[]).map(|c| (c, plan.notes))
        }
    }
}

/// Copies a pack folder into `<data_dir>/packs/<id>` after checking that it is a valid pack. Returns the
/// pack's id. Refuses a pack that is already installed, and the base pack.
pub fn install(data_dir: &Path, source: &Path) -> Result<String, String> {
    let pack = load_pack(&DirPack::new(source), &Limits::default())
        .map_err(|r| format!("That folder is not a valid pack:\n{r}"))?;
    let id = pack.manifest.id.to_string();
    if id == BASE {
        return Err("The base game cannot be installed again.".to_owned());
    }
    let target = data_dir.join(PACKS_DIR).join(&id);
    if target.exists() {
        return Err(format!(
            "A pack named '{id}' is already installed; remove it first."
        ));
    }
    copy_dir(source, &target, 0).map_err(|e| {
        let _ = std::fs::remove_dir_all(&target);
        format!("The pack could not be copied: {e}")
    })?;
    Ok(id)
}

fn copy_dir(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    if depth > 8 {
        return Err(std::io::Error::other("folders are nested too deeply"));
    }
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        let kind = e.file_type()?;
        if kind.is_dir() {
            copy_dir(&e.path(), &target, depth + 1)?;
        } else if kind.is_file() {
            std::fs::copy(e.path(), &target)?;
        }
        // Links and anything else are not copied.
    }
    Ok(())
}

/// Removes an installed pack's folder.
pub fn remove(data_dir: &Path, folder: &str) -> Result<(), String> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder == ".." || folder == "." {
        return Err("not a pack folder".to_owned());
    }
    std::fs::remove_dir_all(data_dir.join(PACKS_DIR).join(folder))
        .map_err(|e| format!("The pack could not be removed: {e}"))
}

#[cfg(test)]
mod tests;
