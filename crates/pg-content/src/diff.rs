//! Content diff (suggestion S-014): what changed between two loaded content sets.
//!
//! Used by `pg content diff` and to explain a content mismatch when a save or replay is loaded against
//! different packs. Templates are compared after resolution (inheritance applied), because that is what
//! the simulation actually sees: editing a parent template shows up as a change in every child.

use crate::content_set::ContentSet;
use crate::ids::TemplateId;
use pg_canon::Canon;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentDiff {
    /// Pack-level changes: `+ pack`, `- pack`, or `~ pack: version/hash`.
    pub packs: Vec<String>,
    pub added: Vec<TemplateId>,
    pub removed: Vec<TemplateId>,
    /// Templates present in both whose resolved form differs, with one line per difference.
    pub changed: Vec<(TemplateId, Vec<String>)>,
}

impl ContentDiff {
    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
    }
}

fn short(h: &str) -> String {
    h.chars().take(8).collect()
}

/// Lists the paths at which two values differ, as `path: old -> new` (or `added` / `removed`).
fn value_diff(path: &str, old: &Canon, new: &Canon, out: &mut Vec<String>) {
    if old == new {
        return;
    }
    match (old, new) {
        (Canon::Map(a), Canon::Map(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let p = if path.is_empty() {
                    k.to_string()
                } else {
                    format!("{path}.{k}")
                };
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => value_diff(&p, x, y, out),
                    (Some(x), None) => out.push(format!("{p}: removed (was {})", x.to_canonical_string())),
                    (None, Some(y)) => out.push(format!("{p}: added ({})", y.to_canonical_string())),
                    (None, None) => {}
                }
            }
        }
        _ => out.push(format!(
            "{path}: {} -> {}",
            old.to_canonical_string(),
            new.to_canonical_string()
        )),
    }
}

/// Compares `old` with `new`. Output lists are in id order, so the result is deterministic.
pub fn diff(old: &ContentSet, new: &ContentSet) -> ContentDiff {
    let mut d = ContentDiff::default();
    let (old_refs, new_refs) = (old.refs(), new.refs());
    for o in &old_refs {
        match new_refs.iter().find(|n| n.pack_id == o.pack_id) {
            None => d.packs.push(format!("- {} {}", o.pack_id, o.version)),
            Some(n) if n.version != o.version => d.packs.push(format!(
                "~ {}: version {} -> {}",
                o.pack_id, o.version, n.version
            )),
            Some(n) if n.hash != o.hash => d.packs.push(format!(
                "~ {}: content changed ({} -> {}) at version {}",
                o.pack_id,
                short(&o.hash),
                short(&n.hash),
                o.version
            )),
            Some(_) => {}
        }
    }
    for n in &new_refs {
        if !old_refs.iter().any(|o| o.pack_id == n.pack_id) {
            d.packs.push(format!("+ {} {}", n.pack_id, n.version));
        }
    }
    let old_ids: BTreeSet<&TemplateId> = old.ids().collect();
    let new_ids: BTreeSet<&TemplateId> = new.ids().collect();
    d.added = new_ids.difference(&old_ids).map(|id| (*id).clone()).collect();
    d.removed = old_ids.difference(&new_ids).map(|id| (*id).clone()).collect();
    for id in old_ids.intersection(&new_ids) {
        let (Some(a), Some(b)) = (old.get(id), new.get(id)) else {
            continue;
        };
        let mut lines = Vec::new();
        if a.chain() != b.chain() {
            let chain = |c: &[TemplateId]| c.iter().map(ToString::to_string).collect::<Vec<_>>().join(" > ");
            lines.push(format!("extends: {} -> {}", chain(a.chain()), chain(b.chain())));
        }
        for t in a.tags().difference(b.tags()) {
            lines.push(format!("tag removed: {t}"));
        }
        for t in b.tags().difference(a.tags()) {
            lines.push(format!("tag added: {t}"));
        }
        let (ca, cb) = (
            Canon::Map(a.components().iter().map(|(k, v)| (k.to_string(), v.clone())).collect()),
            Canon::Map(b.components().iter().map(|(k, v)| (k.to_string(), v.clone())).collect()),
        );
        value_diff("", &ca, &cb, &mut lines);
        if !lines.is_empty() {
            d.changed.push(((*id).clone(), lines));
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentRegistry;
    use crate::pack::{load_pack, Limits, MemoryPack};

    fn set(version: &str, templates: &str) -> ContentSet {
        let pack = MemoryPack::new()
            .with(
                "pack.json",
                &format!(r#"{{"id":"base","name":"Base","version":"{version}"}}"#),
            )
            .with("data/templates/t.json", templates);
        ContentSet::build(
            vec![load_pack(&pack, &Limits::default()).unwrap()],
            ComponentRegistry::builtin(),
        )
        .unwrap()
    }

    const BASE: &str = r#"[{"id":"base.object","schema":1},
        {"id":"base.item","schema":1,"extends":"base.object","tags":["item"]},
        {"id":"item.coin","schema":1,"extends":"base.item"},
        {"id":"item.gem","schema":1,"extends":"base.item","tags":["precious"]}]"#;

    #[test]
    fn identical_sets_have_an_empty_diff() {
        assert!(diff(&set("0.1.0", BASE), &set("0.1.0", BASE)).is_empty());
    }

    #[test]
    fn added_removed_and_retagged_templates_are_listed() {
        let new = r#"[{"id":"base.object","schema":1},
            {"id":"base.item","schema":1,"extends":"base.object","tags":["item","portable"]},
            {"id":"item.coin","schema":1,"extends":"base.item"},
            {"id":"item.ruby","schema":1,"extends":"base.item"}]"#;
        let d = diff(&set("0.1.0", BASE), &set("0.2.0", new));
        assert_eq!(d.packs, ["~ base: version 0.1.0 -> 0.2.0"]);
        assert_eq!(d.added.iter().map(ToString::to_string).collect::<Vec<_>>(), ["item.ruby"]);
        assert_eq!(d.removed.iter().map(ToString::to_string).collect::<Vec<_>>(), ["item.gem"]);
        // The parent's new tag shows up on the parent and on its surviving child.
        let changed: Vec<String> = d.changed.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(changed, ["base.item", "item.coin"]);
        assert!(d.changed[0].1.contains(&"tag added: portable".to_owned()));
    }

    #[test]
    fn component_values_are_compared_field_by_field() {
        let a = r#"[{"id":"base.object","schema":1},
            {"id":"item.thing","schema":1,"extends":"base.object","components":{"physical":{"weight":5}}}]"#;
        let b = a.replace("\"weight\":5", "\"weight\":9");
        let d = diff(&set("0.1.0", a), &set("0.1.0", &b));
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].1, ["physical.weight: 5 -> 9"]);
        assert_eq!(d.packs.len(), 1, "same version, different hash is called out");
        assert!(d.packs[0].contains("content changed"));
    }
}
