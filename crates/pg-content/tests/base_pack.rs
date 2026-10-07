//! The shipped base pack must always load, validate and resolve (Blueprint §17: "built-ins are tested in CI").

use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits, TemplateId};
use std::path::PathBuf;

fn base_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/base")
}

fn tid(s: &str) -> TemplateId {
    s.parse().unwrap()
}

fn load_base() -> ContentSet {
    let pack = load_pack(&DirPack::new(base_dir()), &Limits::default())
        .unwrap_or_else(|r| panic!("the base pack must load:\n{r}"));
    ContentSet::build(vec![pack], ComponentRegistry::builtin())
        .unwrap_or_else(|r| panic!("the base pack must build:\n{r}"))
}

#[test]
fn the_base_pack_loads_with_no_warnings() {
    let set = load_base();
    assert!(set.warnings().is_empty(), "{}", set.warnings());
    assert_eq!(
        set.load_order()
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["base"]
    );
    assert_eq!(set.len(), 15);
}

#[test]
fn the_blueprints_example_chains_exist() {
    let set = load_base();
    let chain = |id: &str| {
        set.get(&tid(id))
            .unwrap()
            .chain()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        chain("furniture.drawer"),
        [
            "base.object",
            "base.item",
            "base.furniture",
            "furniture.drawer"
        ]
    );
    assert_eq!(
        chain("creature.human"),
        [
            "base.object",
            "base.creature",
            "base.sapient",
            "base.humanoid",
            "creature.human"
        ]
    );
}

#[test]
fn inheritance_merges_as_designed() {
    let set = load_base();
    let drawer = set.get(&tid("furniture.drawer")).unwrap();
    assert!(drawer.has_tag("item") && drawer.has_tag("furniture"));
    // physical: weight from base.furniture (50), blocks_movement from base.furniture, size defaults.
    assert_eq!(
        drawer.component("physical").unwrap().to_canonical_string(),
        r#"{"blocks_movement":true,"height":1,"weight":50,"width":1}"#
    );
    // The drawer's container comes from the `containers` sugar and refuses furniture.
    assert!(drawer
        .component("container")
        .unwrap()
        .to_canonical_string()
        .contains(r#""exclude_tags":["furniture"]"#));
    let human = set.get(&tid("creature.human")).unwrap();
    assert!(human.has_tag("humanoid") && human.has_tag("sapient") && human.has_tag("creature"));
    assert!(
        human.component("container").is_some(),
        "humans inherit the inventory"
    );
    assert_eq!(
        human.component("damage").unwrap().to_canonical_string(),
        r#"{"max_health":100}"#
    );
}

#[test]
fn needs_restoring_objects_declare_their_effects() {
    let set = load_base();
    let bed = set.get(&tid("furniture.bed")).unwrap();
    assert_eq!(
        bed.component("needs_restore")
            .unwrap()
            .to_canonical_string(),
        r#"{"effects":[{"need":"energy","per_minute":12}]}"#
    );
    let sandwich = set.get(&tid("item.sandwich")).unwrap();
    assert!(sandwich.has_tag("food"));
}

#[test]
fn the_content_hash_is_stable_across_loads() {
    assert_eq!(load_base().refs(), load_base().refs());
}
