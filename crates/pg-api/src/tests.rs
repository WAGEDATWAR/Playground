use super::*;

#[test]
fn every_function_is_unique_and_well_formed() {
    let mut paths = function_paths();
    paths.sort();
    let n = paths.len();
    paths.dedup();
    assert_eq!(paths.len(), n, "duplicate function path");
    for f in FUNCTIONS {
        assert!(f.path().starts_with("pg."));
        assert!(!f.summary.is_empty() && f.signature.starts_with('('));
        assert!(f.capability.is_none_or(|c| CAPABILITIES.contains(&c)));
    }
}

#[test]
fn hook_points_are_sane_and_resolve_with_clamping() {
    for h in HOOK_POINTS {
        assert!(h.min <= h.default && h.default <= h.max, "{}", h.id);
    }
    let h = hook_point("movement.speed_modifier").unwrap();
    assert_eq!(h.resolve(&[]), 1000);
    assert_eq!(h.resolve(&[1200]), 1200);
    assert_eq!(h.resolve(&[1200, 1200]), 1440, "permille factors multiply");
    assert_eq!(h.resolve(&[100]), 500, "clamped up");
    assert_eq!(h.resolve(&[9000, 9000]), 1500, "clamped down");
    assert!(hook_point("movement.speed_modifer").is_none());
}

#[test]
fn combiners_combine() {
    assert_eq!(Combiner::Sum.combine(&[1, 2, 3], 0), 6);
    assert_eq!(Combiner::Min.combine(&[4, 2, 9], 0), 2);
    assert_eq!(Combiner::Max.combine(&[4, 2, 9], 0), 9);
    assert_eq!(Combiner::FirstWins.combine(&[7, 8], 0), 7);
    assert_eq!(Combiner::ProductPermille.combine(&[500, 500], 1000), 250);
    assert_eq!(Combiner::Sum.combine(&[], 42), 42);
    assert_eq!(Combiner::Sum.combine(&[i64::MAX, 5], 0), i64::MAX);
    assert_eq!(
        Combiner::ProductPermille.combine(&[i64::MAX, i64::MAX], 1000),
        i64::MAX
    );
}

#[test]
fn capabilities_decide_what_is_allowed() {
    let none = allowed_functions([]);
    assert!(none.iter().all(|f| f.capability.is_none()));
    assert!(none.iter().any(|f| f.path() == "pg.rand"));
    let data = allowed_functions(["data"]);
    assert!(data.iter().any(|f| f.path() == "pg.components.register"));
    assert!(!data.iter().any(|f| f.path() == "pg.systems.register"));
}

#[test]
fn unknown_api_names_get_hints() {
    assert!(unknown_path_hint("pg.component.register").contains("pg.components.register"));
    assert!(unknown_path_hint("pg.hook.on").contains("pg.hooks.on"));
    assert_eq!(unknown_path_hint("pg.totally.unrelated.thing"), "");
}

#[test]
fn the_generated_definitions_and_docs_mention_everything() {
    let defs = render_luau_defs();
    let docs = render_docs();
    for f in FUNCTIONS {
        assert!(
            defs.contains(&format!("{}: {}", f.name, f.signature)),
            "{}",
            f.path()
        );
        assert!(docs.contains(&f.path()));
    }
    for t in TYPES {
        assert!(defs.contains(&format!("export type {} =", t.name)));
    }
    for h in HOOK_POINTS {
        assert!(docs.contains(h.id));
    }
    assert!(defs.starts_with("--!strict"));
    assert!(defs.contains("declare pg: {"));
}
