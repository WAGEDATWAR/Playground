//! A set of loaded packs combined into one resolved body of content (Blueprint §4.2, §17).
//!
//! Building a [`ContentSet`] checks everything that spans packs:
//!
//! * pack ids are unique; every dependency exists and satisfies its version range;
//! * dependencies are acyclic, and packs are ordered deterministically (dependencies first, ties by
//!   pack id) — this order is the "pack load order" used wherever order matters;
//! * the engine and API version ranges each pack declares accept this build;
//! * every template id is unique across packs, and non-`base` packs keep their templates in their own
//!   namespace (`<pack_id>.…`);
//! * a template may only extend templates from its own pack or a pack it (transitively) depends on;
//! * every template resolves and validates.

use crate::component::ComponentRegistry;
use crate::ids::{PackId, TemplateId};
use crate::manifest::{PackManifest, Version, API_VERSION, ENGINE_VERSION};
use crate::pack::LoadedPack;
use crate::report::ValidationReport;
use crate::resolve::{resolve_template, ResolvedTemplate};
use crate::template::ObjectTemplate;
use pg_canon::{Canon, ToCanon};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The id of the base game's own pack, which may define templates outside its namespace.
pub const BASE_PACK: &str = "base";

/// What a world records to know exactly which content it was made with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRef {
    pub pack_id: PackId,
    pub version: Version,
    pub hash: String,
}

impl ToCanon for ContentRef {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("pack_id", self.pack_id.to_canon()),
            ("version", Canon::str(self.version.to_string())),
            ("hash", Canon::str(self.hash.clone())),
        ])
    }
}

/// A pack's scripts as the script host needs them.
#[derive(Clone, Debug)]
pub struct ScriptPack<'a> {
    pub id: &'a str,
    pub version: String,
    pub hash: &'a str,
    pub capabilities: Vec<&'static str>,
    pub entry: &'a str,
    pub sources: &'a BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct ContentSet {
    packs: Vec<PackInfo>,
    templates: BTreeMap<TemplateId, ObjectTemplate>,
    origins: BTreeMap<TemplateId, PackId>,
    resolved: BTreeMap<TemplateId, Arc<ResolvedTemplate>>,
    registry: ComponentRegistry,
    strings: crate::strings::Strings,
    game: crate::gamedata::GameData,
    warnings: ValidationReport,
}

#[derive(Clone, Debug)]
struct PackInfo {
    manifest: PackManifest,
    hash: String,
    scripts: BTreeMap<String, String>,
}

impl ContentSet {
    /// Builds a content set from loaded packs. On any error the whole report is returned.
    pub fn build(
        packs: Vec<LoadedPack>,
        registry: ComponentRegistry,
    ) -> Result<ContentSet, ValidationReport> {
        let mut report = ValidationReport::new();
        for p in &packs {
            report.merge(p.warnings.clone());
        }

        // Unique pack ids.
        let mut by_id: BTreeMap<PackId, &LoadedPack> = BTreeMap::new();
        for p in &packs {
            if by_id.insert(p.manifest.id.clone(), p).is_some() {
                report.error(
                    "duplicate_pack",
                    p.manifest.id.to_string(),
                    "two packs have the same id",
                );
            }
        }

        // Version checks and dependency existence.
        for p in &packs {
            let m = &p.manifest;
            if !m.api.matches(&API_VERSION) {
                report.error(
                    "api_mismatch",
                    m.id.to_string(),
                    format!(
                        "requires modding API '{}' but this build provides {API_VERSION}",
                        m.api
                    ),
                );
            }
            if !m.engine.matches(&ENGINE_VERSION) {
                report.error(
                    "engine_mismatch",
                    m.id.to_string(),
                    format!(
                        "requires engine '{}' but this build is {ENGINE_VERSION}",
                        m.engine
                    ),
                );
            }
            for dep in &m.depends {
                match by_id.get(&dep.id) {
                    None => report.error(
                        "missing_dependency",
                        m.id.to_string(),
                        format!(
                            "depends on pack '{}', which is not installed{}",
                            dep.id,
                            crate::hints::hint(dep.id.as_str(), by_id.keys().map(PackId::as_str))
                        ),
                    ),
                    Some(d) if !dep.version.matches(&d.manifest.version) => report.error(
                        "dependency_version",
                        m.id.to_string(),
                        format!(
                            "needs '{}' {} but {} is installed",
                            dep.id, dep.version, d.manifest.version
                        ),
                    ),
                    Some(_) => {}
                }
            }
        }
        if !report.is_ok() {
            return Err(report);
        }

        let order = match load_order(&packs) {
            Ok(o) => o,
            Err(cycle) => {
                report.error(
                    "dependency_cycle",
                    cycle.join(" -> "),
                    "packs depend on each other in a cycle",
                );
                return Err(report);
            }
        };

        // Transitive dependency closure per pack (for the extends rule).
        let mut closure: BTreeMap<PackId, BTreeSet<PackId>> = BTreeMap::new();
        for id in &order {
            let mut set = BTreeSet::new();
            set.insert(id.clone());
            if let Some(p) = by_id.get(id) {
                for dep in &p.manifest.depends {
                    if let Some(c) = closure.get(&dep.id) {
                        set.extend(c.iter().cloned());
                    }
                }
            }
            closure.insert(id.clone(), set);
        }

        // Merge templates in load order; check namespaces and collisions.
        let mut templates: BTreeMap<TemplateId, ObjectTemplate> = BTreeMap::new();
        let mut origins: BTreeMap<TemplateId, PackId> = BTreeMap::new();
        for id in &order {
            let Some(p) = by_id.get(id) else { continue };
            for t in &p.templates {
                if id.as_str() != BASE_PACK && !t.id.in_namespace(id) {
                    report.error(
                        "namespace_violation",
                        t.id.to_string(),
                        format!("pack '{id}' may only define templates named '{id}.…'"),
                    );
                    continue;
                }
                if let Some(first) = origins.get(&t.id) {
                    report.error(
                        "duplicate_template",
                        t.id.to_string(),
                        format!("defined by both '{first}' and '{id}'"),
                    );
                    continue;
                }
                origins.insert(t.id.clone(), id.clone());
                templates.insert(t.id.clone(), t.clone());
            }
        }

        // A template may only extend templates it is allowed to see.
        for (tid, t) in &templates {
            if let (Some(parent), Some(owner)) = (&t.extends, origins.get(tid)) {
                if let (Some(parent_owner), Some(visible)) =
                    (origins.get(parent), closure.get(owner))
                {
                    if !visible.contains(parent_owner) {
                        report.error(
                            "undeclared_dependency",
                            format!("{tid}.extends"),
                            format!("extends '{parent}' from pack '{parent_owner}', but '{owner}' does not depend on it"),
                        );
                    }
                }
            }
        }

        // Resolve everything.
        let mut resolved = BTreeMap::new();
        for tid in templates.keys() {
            if let Some(r) = resolve_template(tid, &templates, &registry, &mut report) {
                resolved.insert(tid.clone(), Arc::new(r));
            }
        }
        if !report.is_ok() {
            return Err(report);
        }

        // Merge string tables in load order; a key defined twice is an error (first definition would win).
        let mut strings = crate::strings::Strings::new();
        for id in &order {
            if let Some(p) = by_id.get(id) {
                for (locale, table) in &p.strings {
                    for key in strings.add(locale, table) {
                        report.error(
                            "duplicate_string",
                            format!("{id}: {locale}.{key}"),
                            "this string key is already defined by an earlier pack",
                        );
                    }
                }
            }
        }
        if !report.is_ok() {
            return Err(report);
        }

        // Merge the game data tables in load order, then check the merged whole.
        let mut game = crate::gamedata::GameData::default();
        for id in &order {
            if let Some(p) = by_id.get(id) {
                game.merge(p.game.clone(), id.as_str(), &mut report);
            }
        }
        game.validate(&mut report);
        for key in game.string_keys() {
            if strings.get(crate::strings::FALLBACK_LOCALE, &key).is_none() {
                report.warn(
                    "missing_string",
                    key.clone(),
                    format!("game data refers to the string '{key}', which no pack defines"),
                );
            }
        }
        if !report.is_ok() {
            return Err(report);
        }

        let infos = order
            .iter()
            .filter_map(|id| by_id.get(id))
            .map(|p| PackInfo {
                manifest: p.manifest.clone(),
                hash: p.hash.clone(),
                scripts: p.scripts.clone(),
            })
            .collect();
        Ok(ContentSet {
            packs: infos,
            templates,
            origins,
            resolved,
            registry,
            strings,
            game,
            warnings: report,
        })
    }

    /// The resolved template for `id`.
    pub fn get(&self, id: &TemplateId) -> Option<&Arc<ResolvedTemplate>> {
        self.resolved.get(id)
    }

    /// The unresolved template as authored.
    pub fn source(&self, id: &TemplateId) -> Option<&ObjectTemplate> {
        self.templates.get(id)
    }

    /// Which pack defined `id`.
    pub fn origin(&self, id: &TemplateId) -> Option<&PackId> {
        self.origins.get(id)
    }

    /// All template ids, ascending.
    pub fn ids(&self) -> impl Iterator<Item = &TemplateId> {
        self.resolved.keys()
    }

    pub fn len(&self) -> usize {
        self.resolved.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.is_empty()
    }

    /// The merged string tables of every loaded pack.
    /// The merged game data tables (empty when no pack has any).
    pub fn game(&self) -> &crate::gamedata::GameData {
        &self.game
    }

    pub fn strings(&self) -> &crate::strings::Strings {
        &self.strings
    }

    pub fn registry(&self) -> &ComponentRegistry {
        &self.registry
    }

    /// Packs in load order (dependencies first).
    pub fn load_order(&self) -> Vec<&PackId> {
        self.packs.iter().map(|p| &p.manifest.id).collect()
    }

    pub fn manifests(&self) -> impl Iterator<Item = &PackManifest> {
        self.packs.iter().map(|p| &p.manifest)
    }

    /// The content references a new world records, in load order.
    pub fn refs(&self) -> Vec<ContentRef> {
        self.packs
            .iter()
            .map(|p| ContentRef {
                pack_id: p.manifest.id.clone(),
                version: p.manifest.version,
                hash: p.hash.clone(),
            })
            .collect()
    }

    /// The packs that have scripts, in load order: what the script host runs.
    pub fn script_packs(&self) -> Vec<ScriptPack<'_>> {
        self.packs
            .iter()
            .filter(|p| p.manifest.entry.is_some() && !p.scripts.is_empty())
            .map(|p| ScriptPack {
                id: p.manifest.id.as_str(),
                version: p.manifest.version.to_string(),
                hash: &p.hash,
                capabilities: p.manifest.capabilities.iter().map(|c| c.name()).collect(),
                entry: p.manifest.entry.as_deref().unwrap_or_default(),
                sources: &p.scripts,
            })
            .collect()
    }

    /// Non-fatal findings from loading and building.
    pub fn warnings(&self) -> &ValidationReport {
        &self.warnings
    }
}

/// Dependencies first; ties broken by pack id, so the order never depends on how the packs were
/// discovered. On a cycle returns the packs that could not be ordered.
fn load_order(packs: &[LoadedPack]) -> Result<Vec<PackId>, Vec<String>> {
    let mut remaining: BTreeMap<PackId, BTreeSet<PackId>> = packs
        .iter()
        .map(|p| {
            (
                p.manifest.id.clone(),
                p.manifest.depends.iter().map(|d| d.id.clone()).collect(),
            )
        })
        .collect();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let ready: Option<PackId> = remaining
            .iter()
            .find(|(_, deps)| deps.iter().all(|d| !remaining.contains_key(d)))
            .map(|(id, _)| id.clone());
        match ready {
            Some(id) => {
                remaining.remove(&id);
                order.push(id);
            }
            None => return Err(remaining.keys().map(ToString::to_string).collect()),
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{load_pack, Limits, MemoryPack};

    fn pack(manifest: &str, templates: &[&str]) -> LoadedPack {
        let mut p = MemoryPack::new().with("pack.json", manifest);
        if manifest.contains("scripts/main.luau") {
            p = p.with("scripts/main.luau", "-- empty");
        }
        for (i, t) in templates.iter().enumerate() {
            p = p.with(&format!("data/templates/t{i}.json"), t);
        }
        load_pack(&p, &Limits::default())
            .unwrap_or_else(|r| panic!("fixture pack should load:\n{r}"))
    }

    fn base() -> LoadedPack {
        pack(
            r#"{"id":"base","name":"Base","version":"0.1.0"}"#,
            &[
                r#"{"id":"base.object","schema":1}"#,
                r#"{"id":"base.item","schema":1,"extends":"base.object","tags":["item"],"components":{"value":{"base":5}}}"#,
                r#"{"id":"furniture.chair","schema":1,"extends":"base.item"}"#,
            ],
        )
    }

    fn mod_pack(id: &str, depends: &str, templates: &[&str]) -> LoadedPack {
        pack(
            &format!(r#"{{"id":"{id}","name":"{id}","version":"1.0.0","depends":{depends}}}"#),
            templates,
        )
    }

    fn build(packs: Vec<LoadedPack>) -> Result<ContentSet, ValidationReport> {
        ContentSet::build(packs, ComponentRegistry::builtin())
    }

    #[test]
    fn the_base_pack_alone_builds() {
        let set = build(vec![base()]).unwrap();
        assert_eq!(set.len(), 3);
        let chair = set.get(&"furniture.chair".parse().unwrap()).unwrap();
        assert_eq!(
            chair.component("value").unwrap().to_canonical_string(),
            r#"{"base":5}"#
        );
        assert_eq!(
            set.origin(&"furniture.chair".parse().unwrap())
                .unwrap()
                .as_str(),
            "base"
        );
        assert_eq!(set.refs().len(), 1);
    }

    #[test]
    fn a_dependent_pack_extends_base_and_loads_after_it() {
        let m = mod_pack(
            "coffee_shop",
            r#"[{"id":"base"}]"#,
            &[
                r#"{"id":"coffee_shop.machine","schema":1,"extends":"base.item","components":{"value":{"base":300}}}"#,
            ],
        );
        // Discovery order must not matter.
        let a = build(vec![m.clone(), base()]).unwrap();
        let b = build(vec![base(), m]).unwrap();
        assert_eq!(a.load_order(), b.load_order());
        assert_eq!(
            a.load_order()
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            ["base", "coffee_shop"]
        );
        assert_eq!(a.refs(), b.refs());
        let r = a.get(&"coffee_shop.machine".parse().unwrap()).unwrap();
        assert!(r.has_tag("item"));
    }

    #[test]
    fn missing_dependencies_and_version_mismatches_are_errors() {
        let m = mod_pack("m", r#"[{"id":"ghost"}]"#, &[]);
        assert!(build(vec![base(), m])
            .unwrap_err()
            .has_code("missing_dependency"));
        let m = mod_pack("m", r#"[{"id":"base","version":">=9"}]"#, &[]);
        assert!(build(vec![base(), m])
            .unwrap_err()
            .has_code("dependency_version"));
    }

    #[test]
    fn dependency_cycles_are_detected() {
        let a = mod_pack("aa", r#"[{"id":"bb"}]"#, &[]);
        let b = mod_pack("bb", r#"[{"id":"aa"}]"#, &[]);
        let r = build(vec![a, b]).unwrap_err();
        assert!(r.has_code("dependency_cycle"), "{r}");
    }

    #[test]
    fn duplicate_pack_ids_are_errors() {
        assert!(build(vec![base(), base()])
            .unwrap_err()
            .has_code("duplicate_pack"));
    }

    #[test]
    fn engine_and_api_ranges_are_enforced() {
        let p = pack(
            r#"{"id":"future","name":"F","version":"1","api":">=5"}"#,
            &[],
        );
        assert!(build(vec![p]).unwrap_err().has_code("api_mismatch"));
        let p = pack(
            r#"{"id":"future","name":"F","version":"1","engine":">=9"}"#,
            &[],
        );
        assert!(build(vec![p]).unwrap_err().has_code("engine_mismatch"));
    }

    #[test]
    fn non_base_packs_must_stay_in_their_namespace() {
        let m = mod_pack(
            "m",
            r#"[{"id":"base"}]"#,
            &[r#"{"id":"furniture.stolen","schema":1,"extends":"base.item"}"#],
        );
        let r = build(vec![base(), m]).unwrap_err();
        assert!(r.has_code("namespace_violation"), "{r}");
    }

    #[test]
    fn a_pack_cannot_redefine_another_packs_template() {
        let m = mod_pack(
            "base2",
            r#"[{"id":"base"}]"#,
            &[r#"{"id":"base2.x","schema":1,"extends":"base.item"}"#],
        );
        assert!(build(vec![base(), m]).is_ok());
        // Same id from two packs: only possible for `base`, which is exempt from the namespace rule.
        let dup = pack(
            r#"{"id":"base","name":"Base copy","version":"0.2.0"}"#,
            &[r#"{"id":"base.object","schema":1}"#],
        );
        assert!(build(vec![base(), dup])
            .unwrap_err()
            .has_code("duplicate_pack"));
    }

    #[test]
    fn extending_from_an_undeclared_pack_is_an_error() {
        let a = mod_pack(
            "aa",
            r#"[{"id":"base"}]"#,
            &[r#"{"id":"aa.thing","schema":1,"extends":"base.item"}"#],
        );
        // `bb` does not depend on `aa` but extends its template.
        let b = mod_pack(
            "bb",
            r#"[{"id":"base"}]"#,
            &[r#"{"id":"bb.thing","schema":1,"extends":"aa.thing"}"#],
        );
        let r = build(vec![base(), a, b]).unwrap_err();
        assert!(r.has_code("undeclared_dependency"), "{r}");
    }

    #[test]
    fn transitive_dependencies_are_visible() {
        let a = mod_pack(
            "aa",
            r#"[{"id":"base"}]"#,
            &[r#"{"id":"aa.thing","schema":1,"extends":"base.item"}"#],
        );
        let b = mod_pack(
            "bb",
            r#"[{"id":"aa"}]"#,
            &[r#"{"id":"bb.thing","schema":1,"extends":"aa.thing"}"#],
        );
        let c = mod_pack(
            "cc",
            r#"[{"id":"bb"}]"#,
            &[r#"{"id":"cc.thing","schema":1,"extends":"base.item"}"#],
        );
        assert!(
            build(vec![base(), a, b, c]).is_ok(),
            "cc reaches base through bb and aa"
        );
    }

    #[test]
    fn resolution_errors_surface_from_the_build() {
        let m = mod_pack(
            "m",
            r#"[{"id":"base"}]"#,
            &[
                r#"{"id":"m.bad","schema":1,"extends":"base.item","components":{"value":{"base":-1}}}"#,
            ],
        );
        let r = build(vec![base(), m]).unwrap_err();
        assert!(r.has_code("out_of_range"), "{r}");
    }

    #[test]
    fn warnings_are_kept_on_success() {
        let p = pack(
            r#"{"id":"base","name":"B","version":"1","entry":"scripts/main.luau"}"#,
            &[r#"{"id":"base.object","schema":1}"#],
        );
        let set = build(vec![p]).unwrap();
        assert!(set.warnings().has_code("script_without_capabilities"));
    }

    #[test]
    fn content_refs_carry_version_and_hash() {
        let set = build(vec![base()]).unwrap();
        let r = &set.refs()[0];
        assert_eq!(
            (r.pack_id.as_str(), r.version.to_string().as_str()),
            ("base", "0.1.0")
        );
        assert_eq!(r.hash.len(), 64);
        assert!(r
            .to_canon()
            .to_canonical_string()
            .contains("\"pack_id\":\"base\""));
    }
}
