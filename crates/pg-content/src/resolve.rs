//! Template resolution (Blueprint §4.3).
//!
//! `resolve_template(id)`:
//!
//! 1. walk `extends` to the root, rejecting cycles, missing parents, chains deeper than
//!    [`MAX_EXTENDS_DEPTH`], and chains that do not end at `base.object`;
//! 2. from root to leaf, merge components by name: objects deep-merge, arrays and scalars replace, and
//!    a `null` deletes the inherited component;
//! 3. union the tags;
//! 4. validate every component against its registered schema, filling defaults;
//! 5. output a [`ResolvedTemplate`], which is immutable by construction (no mutating methods).

use crate::component::ComponentRegistry;
use crate::ids::{ComponentName, Tag, TemplateId};
use crate::report::ValidationReport;
use crate::template::ObjectTemplate;
use pg_canon::{Canon, ToCanon};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_EXTENDS_DEPTH: usize = 16;

/// A template with its whole inheritance chain flattened. Read-only: it is shared (`Arc`) by every
/// object instance made from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTemplate {
    id: TemplateId,
    chain: Vec<TemplateId>,
    tags: BTreeSet<Tag>,
    components: BTreeMap<ComponentName, Canon>,
}

impl ResolvedTemplate {
    pub fn id(&self) -> &TemplateId {
        &self.id
    }

    /// The inheritance chain from the root (`base.object`) down to this template.
    pub fn chain(&self) -> &[TemplateId] {
        &self.chain
    }

    pub fn tags(&self) -> &BTreeSet<Tag> {
        &self.tags
    }

    /// Normalized (defaults filled) params of every component this template ends up with.
    pub fn components(&self) -> &BTreeMap<ComponentName, Canon> {
        &self.components
    }

    pub fn component(&self, name: &str) -> Option<&Canon> {
        ComponentName::new(name)
            .ok()
            .and_then(|n| self.components.get(&n))
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        Tag::new(tag).is_ok_and(|t| self.tags.contains(&t))
    }
}

impl ToCanon for ResolvedTemplate {
    fn to_canon(&self) -> Canon {
        Canon::map([
            ("id", self.id.to_canon()),
            (
                "chain",
                Canon::List(self.chain.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "tags",
                Canon::List(self.tags.iter().map(ToCanon::to_canon).collect()),
            ),
            (
                "components",
                Canon::Map(
                    self.components
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// Deep-merges `over` onto `base`: objects merge key by key, everything else (arrays, scalars,
/// type changes) is replaced by `over`.
pub fn deep_merge(base: &Canon, over: &Canon) -> Canon {
    match (base, over) {
        (Canon::Map(b), Canon::Map(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                let next = match b.get(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => v.clone(),
                };
                merged.insert(k.clone(), next);
            }
            Canon::Map(merged)
        }
        _ => over.clone(),
    }
}

/// The root-to-leaf chain for `id`, or `None` (with errors recorded) if it is malformed.
pub fn inheritance_chain(
    id: &TemplateId,
    templates: &BTreeMap<TemplateId, ObjectTemplate>,
    report: &mut ValidationReport,
) -> Option<Vec<TemplateId>> {
    let mut chain = vec![id.clone()];
    let mut seen: BTreeSet<&TemplateId> = BTreeSet::new();
    seen.insert(id);
    let mut current = templates.get(id)?;
    while let Some(parent_id) = &current.extends {
        if seen.contains(parent_id) {
            let mut cycle: Vec<String> = chain.iter().map(ToString::to_string).collect();
            cycle.push(parent_id.to_string());
            report.error(
                "extends_cycle",
                id.to_string(),
                format!("inheritance cycle: {}", cycle.join(" -> ")),
            );
            return None;
        }
        let Some(parent) = templates.get(parent_id) else {
            report.error(
                "missing_parent",
                format!("{}.extends", current.id),
                format!(
                    "'{}' extends '{parent_id}', which does not exist",
                    current.id
                ),
            );
            return None;
        };
        chain.push(parent_id.clone());
        if chain.len() > MAX_EXTENDS_DEPTH {
            report.error(
                "extends_too_deep",
                id.to_string(),
                format!("inheritance chain is deeper than {MAX_EXTENDS_DEPTH}"),
            );
            return None;
        }
        seen.insert(parent_id);
        current = parent;
    }
    if !current.id.is_root() {
        report.error(
            "bad_root",
            id.to_string(),
            format!(
                "inheritance chain ends at '{}', but every chain must end at '{}'",
                current.id,
                TemplateId::ROOT
            ),
        );
        return None;
    }
    chain.reverse();
    Some(chain)
}

/// Resolves one template. Returns `None` (with errors recorded) if anything is wrong.
pub fn resolve_template(
    id: &TemplateId,
    templates: &BTreeMap<TemplateId, ObjectTemplate>,
    registry: &ComponentRegistry,
    report: &mut ValidationReport,
) -> Option<ResolvedTemplate> {
    if !templates.contains_key(id) {
        report.error("unknown_template", id.to_string(), "no such template");
        return None;
    }
    let before = report.error_count();
    let chain = inheritance_chain(id, templates, report)?;

    let mut tags = BTreeSet::new();
    let mut merged: BTreeMap<ComponentName, Canon> = BTreeMap::new();
    for link in &chain {
        let Some(t) = templates.get(link) else {
            continue;
        };
        tags.extend(t.tags.iter().cloned());
        for (name, op) in &t.components {
            match op {
                None => {
                    if merged.remove(name).is_none() {
                        report.warn(
                            "delete_missing",
                            format!("{}.components.{name}", t.id),
                            "deletes a component that is not inherited",
                        );
                    }
                }
                Some(params) => {
                    let next = match merged.get(name) {
                        Some(existing) => deep_merge(existing, params),
                        None => params.clone(),
                    };
                    merged.insert(name.clone(), next);
                }
            }
        }
    }

    let mut components = BTreeMap::new();
    for (name, params) in &merged {
        let path = format!("{id}.components.{name}");
        match registry.get(name) {
            None => report.error(
                "unknown_component",
                path,
                format!("no component named '{name}' is registered"),
            ),
            Some(def) => {
                let normalized = def.schema.check(params, &path, report);
                components.insert(name.clone(), normalized);
            }
        }
    }
    if report.error_count() > before {
        return None;
    }
    Some(ResolvedTemplate {
        id: id.clone(),
        chain,
        tags,
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_canon::json::parse;

    fn tid(s: &str) -> TemplateId {
        s.parse().unwrap()
    }

    fn set(jsons: &[&str]) -> BTreeMap<TemplateId, ObjectTemplate> {
        let mut out = BTreeMap::new();
        for j in jsons {
            let mut r = ValidationReport::new();
            let t = ObjectTemplate::from_canon(&parse(j).unwrap(), "t", &mut r)
                .unwrap_or_else(|| panic!("fixture should load: {j}\n{r}"));
            out.insert(t.id.clone(), t);
        }
        out
    }

    fn base_chain() -> Vec<&'static str> {
        vec![
            r#"{"id":"base.object","schema":1}"#,
            r#"{"id":"base.item","schema":1,"extends":"base.object","tags":["item"],"components":{"value":{"base":5},"physical":{"weight":10}}}"#,
            r#"{"id":"base.furniture","schema":1,"extends":"base.item","tags":["furniture"],"components":{"physical":{"blocks_movement":true,"width":2}}}"#,
            r#"{"id":"furniture.drawer","schema":1,"extends":"base.furniture","components":{"value":{"base":40},"container":{"containers":[{"id":"c","capacity":8}]}}}"#,
        ]
    }

    fn resolve(
        templates: &BTreeMap<TemplateId, ObjectTemplate>,
        id: &str,
    ) -> (Option<ResolvedTemplate>, ValidationReport) {
        let mut r = ValidationReport::new();
        let out = resolve_template(&tid(id), templates, &ComponentRegistry::builtin(), &mut r);
        (out, r)
    }

    #[test]
    fn the_blueprint_example_chain_resolves() {
        let t = set(&base_chain());
        let (res, r) = resolve(&t, "furniture.drawer");
        assert!(r.is_ok(), "{r}");
        let res = res.unwrap();
        assert_eq!(
            res.chain()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "base.object",
                "base.item",
                "base.furniture",
                "furniture.drawer"
            ]
        );
        assert!(
            res.has_tag("item") && res.has_tag("furniture"),
            "tags union along the chain"
        );
        // value: leaf overrides; physical: deep-merged across three levels with defaults filled.
        assert_eq!(
            res.component("value").unwrap().to_canonical_string(),
            r#"{"base":40}"#
        );
        assert_eq!(
            res.component("physical").unwrap().to_canonical_string(),
            r#"{"blocks_movement":true,"height":1,"weight":10,"width":2}"#
        );
        assert!(res.component("container").is_some());
    }

    #[test]
    fn arrays_replace_rather_than_merge() {
        let mut v = base_chain();
        v.push(r#"{"id":"furniture.big","schema":1,"extends":"furniture.drawer","components":{"container":{"containers":[{"id":"only","capacity":1}]}}}"#);
        let t = set(&v);
        let (res, r) = resolve(&t, "furniture.big");
        assert!(r.is_ok(), "{r}");
        let c = res
            .unwrap()
            .component("container")
            .unwrap()
            .to_canonical_string();
        assert!(
            c.contains(r#""id":"only""#) && !c.contains(r#""id":"c""#),
            "{c}"
        );
    }

    #[test]
    fn null_deletes_an_inherited_component() {
        let mut v = base_chain();
        v.push(r#"{"id":"furniture.ghost","schema":1,"extends":"furniture.drawer","components":{"value":null}}"#);
        let (res, r) = resolve(&set(&v), "furniture.ghost");
        assert!(r.is_ok(), "{r}");
        assert!(res.unwrap().component("value").is_none());
    }

    #[test]
    fn deleting_something_not_inherited_only_warns() {
        let mut v = base_chain();
        v.push(r#"{"id":"furniture.odd","schema":1,"extends":"furniture.drawer","components":{"damage":null}}"#);
        let (res, r) = resolve(&set(&v), "furniture.odd");
        assert!(
            res.is_some() && r.is_ok() && r.has_code("delete_missing"),
            "{r}"
        );
    }

    #[test]
    fn cycles_are_rejected_with_the_cycle_named() {
        let t = set(&[
            r#"{"id":"base.object","schema":1}"#,
            r#"{"id":"a.one","schema":1,"extends":"a.two"}"#,
            r#"{"id":"a.two","schema":1,"extends":"a.three"}"#,
            r#"{"id":"a.three","schema":1,"extends":"a.one"}"#,
        ]);
        let (res, r) = resolve(&t, "a.one");
        assert!(res.is_none());
        let msg = r
            .errors()
            .find(|i| i.code == "extends_cycle")
            .unwrap()
            .message
            .clone();
        assert!(msg.contains("a.one -> a.two -> a.three -> a.one"), "{msg}");
    }

    #[test]
    fn a_template_extending_itself_is_a_cycle() {
        let t = set(&[
            r#"{"id":"base.object","schema":1}"#,
            r#"{"id":"a.me","schema":1,"extends":"a.me"}"#,
        ]);
        assert!(resolve(&t, "a.me").1.has_code("extends_cycle"));
    }

    #[test]
    fn depth_is_limited_to_sixteen() {
        fn chain_of(len: usize) -> BTreeMap<TemplateId, ObjectTemplate> {
            let mut v = vec![r#"{"id":"base.object","schema":1}"#.to_owned()];
            for i in 1..len {
                let parent = if i == 1 {
                    "base.object".to_owned()
                } else {
                    format!("a.t{}", i - 1)
                };
                v.push(format!(
                    r#"{{"id":"a.t{i}","schema":1,"extends":"{parent}"}}"#
                ));
            }
            set(&v.iter().map(String::as_str).collect::<Vec<_>>())
        }
        // 16 links including base.object and the leaf: allowed.
        let ok = chain_of(16);
        assert!(resolve(&ok, "a.t15").0.is_some());
        let too_deep = chain_of(17);
        let (res, r) = resolve(&too_deep, "a.t16");
        assert!(res.is_none() && r.has_code("extends_too_deep"), "{r}");
    }

    #[test]
    fn missing_parents_and_bad_roots_are_errors() {
        let t = set(&[
            r#"{"id":"base.object","schema":1}"#,
            r#"{"id":"a.b","schema":1,"extends":"nope.x"}"#,
        ]);
        assert!(resolve(&t, "a.b").1.has_code("missing_parent"));
        let t = set(&[r#"{"id":"a.rootless","schema":1}"#]);
        assert!(resolve(&t, "a.rootless").1.has_code("bad_root"));
        assert!(resolve(&t, "no.such").1.has_code("unknown_template"));
    }

    #[test]
    fn invalid_component_params_are_reported_at_the_template_that_resolved_them() {
        let mut v = base_chain();
        v.push(r#"{"id":"furniture.bad","schema":1,"extends":"furniture.drawer","components":{"value":{"base":-5},"mystery":{"x":1}}}"#);
        let (res, r) = resolve(&set(&v), "furniture.bad");
        assert!(res.is_none());
        assert!(
            r.errors().any(
                |i| i.code == "out_of_range" && i.path == "furniture.bad.components.value.base"
            ),
            "{r}"
        );
        assert!(r.errors().any(|i| i.code == "unknown_component"), "{r}");
    }

    #[test]
    fn deep_merge_semantics() {
        let m = |s: &str| parse(s).unwrap();
        assert_eq!(
            deep_merge(
                &m(r#"{"a":1,"b":{"x":1,"y":2}}"#),
                &m(r#"{"b":{"y":9,"z":3}}"#)
            ),
            m(r#"{"a":1,"b":{"x":1,"y":9,"z":3}}"#)
        );
        assert_eq!(deep_merge(&m("[1,2]"), &m("[3]")), m("[3]"));
        assert_eq!(
            deep_merge(&m(r#"{"a":1}"#), &m("5")),
            m("5"),
            "type change replaces"
        );
        assert_eq!(
            deep_merge(&m(r#"{"a":[1,2]}"#), &m(r#"{"a":[3]}"#)),
            m(r#"{"a":[3]}"#)
        );
    }

    #[test]
    fn resolution_is_deterministic_regardless_of_declaration_order() {
        let mut forward = base_chain();
        let a = resolve(&set(&forward), "furniture.drawer")
            .0
            .unwrap()
            .to_canon()
            .to_canonical_string();
        forward.reverse();
        let b = resolve(&set(&forward), "furniture.drawer")
            .0
            .unwrap()
            .to_canon()
            .to_canonical_string();
        assert_eq!(a, b);
    }
}
