//! The inheritance forest (suggestion S-015): which template extends which, and from which pack.
//!
//! `pg content tree` prints it as indented text or as Graphviz. A chain deeper than [`DEEP_CHAIN`] is
//! flagged, since long `extends` chains are usually an accident.

use crate::content_set::ContentSet;
use crate::ids::TemplateId;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Chains with more than this many templates (root included) are marked.
pub const DEEP_CHAIN: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: TemplateId,
    pub pack: String,
    pub children: Vec<Node>,
}

/// Every template, arranged under the template it extends. Roots are templates that extend nothing (or
/// whose parent is missing, which content validation already refuses), in id order.
pub fn forest(set: &ContentSet) -> Vec<Node> {
    let mut children: BTreeMap<TemplateId, Vec<TemplateId>> = BTreeMap::new();
    let mut roots = Vec::new();
    for id in set.ids() {
        match set.source(id).and_then(|t| t.extends.clone()) {
            Some(parent) if set.source(&parent).is_some() => {
                children.entry(parent).or_default().push(id.clone());
            }
            _ => roots.push(id.clone()),
        }
    }
    fn build(
        id: &TemplateId,
        set: &ContentSet,
        kids: &BTreeMap<TemplateId, Vec<TemplateId>>,
    ) -> Node {
        Node {
            id: id.clone(),
            pack: set.origin(id).map(ToString::to_string).unwrap_or_default(),
            children: kids
                .get(id)
                .map(|v| v.iter().map(|c| build(c, set, kids)).collect())
                .unwrap_or_default(),
        }
    }
    roots.iter().map(|r| build(r, set, &children)).collect()
}

/// The depth of the deepest chain under `n`, counting `n` itself.
pub fn depth(n: &Node) -> usize {
    1 + n.children.iter().map(depth).max().unwrap_or(0)
}

/// Indented text, one template per line with its pack, and a note on deep chains.
pub fn render_text(roots: &[Node]) -> String {
    fn walk(n: &Node, level: usize, out: &mut String) {
        let flag = if level + 1 > DEEP_CHAIN {
            "   <- deep chain"
        } else {
            ""
        };
        let _ = writeln!(out, "{}{}  [{}]{flag}", "  ".repeat(level), n.id, n.pack);
        for c in &n.children {
            walk(c, level + 1, out);
        }
    }
    let mut out = String::new();
    for r in roots {
        walk(r, 0, &mut out);
    }
    out
}

/// A Graphviz digraph, one colour per pack.
pub fn render_dot(roots: &[Node]) -> String {
    const COLOURS: [&str; 6] = [
        "#cfe8ff", "#ffe2b8", "#d6f5d6", "#f5d6f0", "#fff2a8", "#e0e0e0",
    ];
    let mut packs: Vec<&str> = Vec::new();
    fn collect<'a>(n: &'a Node, packs: &mut Vec<&'a str>) {
        if !packs.contains(&n.pack.as_str()) {
            packs.push(&n.pack);
        }
        n.children.iter().for_each(|c| collect(c, packs));
    }
    roots.iter().for_each(|r| collect(r, &mut packs));
    let colour = |pack: &str| {
        let at = packs.iter().position(|p| *p == pack).unwrap_or(0);
        COLOURS
            .get(at % COLOURS.len())
            .copied()
            .unwrap_or("#ffffff")
    };
    fn edges(n: &Node, out: &mut String) {
        for c in &n.children {
            let _ = writeln!(out, "  \"{}\" -> \"{}\";", n.id, c.id);
            edges(c, out);
        }
    }
    fn nodes(n: &Node, colour: &dyn Fn(&str) -> &'static str, out: &mut String) {
        let _ = writeln!(
            out,
            "  \"{}\" [style=filled, fillcolor=\"{}\", tooltip=\"{}\"];",
            n.id,
            colour(&n.pack),
            n.pack
        );
        n.children.iter().for_each(|c| nodes(c, colour, out));
    }
    let mut out = String::from("digraph content {\n  rankdir=LR;\n  node [shape=box];\n");
    roots.iter().for_each(|r| nodes(r, &colour, &mut out));
    roots.iter().for_each(|r| edges(r, &mut out));
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentRegistry;
    use crate::pack::{load_pack, Limits, MemoryPack};

    fn pack(id: &str, depends: &str, templates: &str) -> crate::pack::LoadedPack {
        let m = MemoryPack::new()
            .with(
                "pack.json",
                &format!(r#"{{"id":"{id}","name":"{id}","version":"1.0.0","depends":{depends}}}"#),
            )
            .with("data/templates/t.json", templates);
        load_pack(&m, &Limits::default()).unwrap_or_else(|r| panic!("{r}"))
    }

    fn set() -> ContentSet {
        let base = pack(
            "base",
            "[]",
            r#"[{"id":"base.object","schema":1},
                {"id":"thing.a","schema":1,"extends":"base.object"},
                {"id":"thing.b","schema":1,"extends":"thing.a"},
                {"id":"thing.c","schema":1,"extends":"thing.b"}]"#,
        );
        let extra = pack(
            "extra",
            r#"[{"id":"base"}]"#,
            r#"[{"id":"extra.d","schema":1,"extends":"thing.b"}]"#,
        );
        ContentSet::build(vec![base, extra], ComponentRegistry::builtin())
            .unwrap_or_else(|r| panic!("{r}"))
    }

    #[test]
    fn the_forest_nests_children_under_their_parent_and_names_each_pack() {
        let f = forest(&set());
        let text = render_text(&f);
        assert!(text.contains("base.object  [base]"), "{text}");
        assert!(text.contains("      thing.c  [base]"), "{text}");
        assert!(text.contains("      extra.d  [extra]"), "{text}");
        assert_eq!(f.len(), 1, "one root: {text}");
        assert!(depth(&f[0]) >= 4);
    }

    #[test]
    fn graphviz_output_has_every_edge_and_a_colour_per_pack() {
        let dot = render_dot(&forest(&set()));
        assert!(dot.starts_with("digraph content {"));
        assert!(dot.contains("\"thing.b\" -> \"extra.d\";"), "{dot}");
        assert!(dot.contains("fillcolor"), "{dot}");
        assert!(dot.contains("#cfe8ff") && dot.contains("#ffe2b8"), "{dot}");
    }
}
