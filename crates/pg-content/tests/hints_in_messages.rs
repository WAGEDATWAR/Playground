//! "Did you mean…?" hints must show up in the real validation messages authors read (S-012).

use pg_content::{
    load_pack, ComponentRegistry, ContentSet, Limits, LoadedPack, MemoryPack, ValidationReport,
};

fn pack(manifest: &str, templates: &str) -> MemoryPack {
    MemoryPack::new()
        .with("pack.json", manifest)
        .with("data/templates/t.json", templates)
}

fn base() -> LoadedPack {
    load_pack(
        &pack(
            r#"{"id":"base","name":"Base","version":"0.1.0"}"#,
            r#"[{"id":"base.object","schema":1},
                {"id":"base.item","schema":1,"extends":"base.object"}]"#,
        ),
        &Limits::default(),
    )
    .unwrap()
}

fn build_with(extra_manifest: &str, templates: &str) -> ValidationReport {
    let m = load_pack(&pack(extra_manifest, templates), &Limits::default()).unwrap();
    ContentSet::build(vec![base(), m], ComponentRegistry::builtin()).unwrap_err()
}

fn message_of(report: &ValidationReport, code: &str) -> String {
    report
        .errors()
        .find(|i| i.code == code)
        .map(|i| i.message.clone())
        .unwrap_or_default()
}

#[test]
fn an_unknown_component_field_suggests_the_right_one() {
    let r = build_with(
        r#"{"id":"m","name":"M","version":"1","depends":[{"id":"base"}]}"#,
        r#"{"id":"m.x","schema":1,"extends":"base.item","components":{"physical":{"wieght":5}}}"#,
    );
    assert!(
        message_of(&r, "unknown_field").contains("did you mean 'weight'?"),
        "{r}"
    );
}

#[test]
fn an_unknown_component_suggests_the_registered_one() {
    let r = build_with(
        r#"{"id":"m","name":"M","version":"1","depends":[{"id":"base"}]}"#,
        r#"{"id":"m.x","schema":1,"extends":"base.item","components":{"phyiscal":{}}}"#,
    );
    assert!(
        message_of(&r, "unknown_component").contains("did you mean 'physical'?"),
        "{r}"
    );
}

#[test]
fn a_missing_parent_suggests_a_known_template() {
    let r = build_with(
        r#"{"id":"m","name":"M","version":"1","depends":[{"id":"base"}]}"#,
        r#"{"id":"m.x","schema":1,"extends":"base.itme"}"#,
    );
    assert!(
        message_of(&r, "missing_parent").contains("did you mean 'base.item'?"),
        "{r}"
    );
}

#[test]
fn a_bad_enum_value_suggests_the_allowed_one() {
    let r = build_with(
        r#"{"id":"m","name":"M","version":"1","depends":[{"id":"base"}]}"#,
        r#"{"id":"m.x","schema":1,"extends":"base.item","components":{"needs_restore":{"effects":[{"need":"hungar","per_minute":5}]}}}"#,
    );
    assert!(
        message_of(&r, "bad_enum").contains("did you mean 'hunger'?"),
        "{r}"
    );
}

#[test]
fn a_misspelled_dependency_suggests_the_installed_pack() {
    let m = load_pack(
        &pack(
            r#"{"id":"m","name":"M","version":"1","depends":[{"id":"bsae"}]}"#,
            r#"[]"#,
        ),
        &Limits::default(),
    )
    .unwrap();
    let r = ContentSet::build(vec![base(), m], ComponentRegistry::builtin()).unwrap_err();
    assert!(
        message_of(&r, "missing_dependency").contains("did you mean 'base'?"),
        "{r}"
    );
}

#[test]
fn template_and_manifest_field_typos_are_helped_too() {
    let mut r = ValidationReport::new();
    let c = pg_canon::json::parse(r#"{"id":"a.b","schema":1,"extnds":"base.object"}"#).unwrap();
    pg_content::ObjectTemplate::from_canon(&c, "t", &mut r);
    assert!(
        message_of(&r, "unknown_field").contains("did you mean 'extends'?"),
        "{r}"
    );

    let mut r = ValidationReport::new();
    let c =
        pg_canon::json::parse(r#"{"id":"a","name":"A","version":"1","capabilites":[]}"#).unwrap();
    pg_content::PackManifest::from_canon(&c, "pack.json", &mut r);
    assert!(
        message_of(&r, "unknown_field").contains("did you mean 'capabilities'?"),
        "{r}"
    );

    let mut r = ValidationReport::new();
    let c =
        pg_canon::json::parse(r#"{"id":"a","name":"A","version":"1","capabilities":["sytems"]}"#)
            .unwrap();
    pg_content::PackManifest::from_canon(&c, "pack.json", &mut r);
    assert!(
        message_of(&r, "unknown_capability").contains("did you mean 'systems'?"),
        "{r}"
    );
}

#[test]
fn unrelated_names_get_no_hint() {
    let r = build_with(
        r#"{"id":"m","name":"M","version":"1","depends":[{"id":"base"}]}"#,
        r#"{"id":"m.x","schema":1,"extends":"base.item","components":{"physical":{"colour":5}}}"#,
    );
    assert!(
        !message_of(&r, "unknown_field").contains("did you mean"),
        "{r}"
    );
}
