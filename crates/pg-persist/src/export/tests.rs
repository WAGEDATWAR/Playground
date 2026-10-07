use super::*;
use crate::store::{LoadOptions, SlotStore};
use crate::testkit::{refs, world_at};
use pg_host::MemStorage;
use proptest::prelude::*;

fn export(ticks: u64) -> (WorldState, Vec<u8>) {
    let w = world_at(ticks);
    let bytes = export_world(&w, &refs(), "0.0.1", "2026-10-06T00:00:00Z");
    (w, bytes)
}

fn doc_with(payload: Canon, schema: i128) -> Vec<u8> {
    Canon::map([
        ("format", Canon::str(EXPORT_FORMAT)),
        ("kind", Canon::str("world")),
        ("schema", Canon::Int(schema)),
        ("app_version", Canon::str("1")),
        ("created_iso", Canon::str("x")),
        ("content_refs", Canon::List(vec![])),
        (
            "hash",
            Canon::str(pg_core::hash::hash_canon(&payload).to_hex()),
        ),
        ("payload", payload),
    ])
    .to_canonical_string()
    .into_bytes()
}

#[test]
fn export_then_import_gives_the_same_world_as_a_new_slot() {
    let (w, bytes) = export(3_700);
    let plan = import_world(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(plan.world.state_hash(), w.state_hash());
    assert_eq!(plan.app_version, "0.0.1");
    assert_eq!(plan.content_refs, refs());
    assert_eq!(plan.migrated_from, None);

    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    commit_import(&store, "imported", &plan, "now", false).unwrap();
    let loaded = store.load("imported", &LoadOptions::default()).unwrap();
    assert_eq!(loaded.world.state_hash(), w.state_hash());
    assert!(matches!(
        commit_import(&store, "imported", &plan, "now", false),
        Err(ImportError::SlotExists(_))
    ));
    commit_import(&store, "imported", &plan, "later", true).unwrap();
    assert!(matches!(
        commit_import(&store, "Bad Name", &plan, "now", true),
        Err(ImportError::BadWorldId(_))
    ));
}

#[test]
fn an_export_is_deterministic_and_carries_only_the_schema() {
    let (_, a) = export(500);
    let (_, b) = export(500);
    assert_eq!(a, b);
    let text = String::from_utf8(a).unwrap();
    let doc = json::parse(&text).unwrap();
    let Canon::Map(m) = &doc else { panic!() };
    let keys: Vec<&str> = m.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "app_version",
            "content_refs",
            "created_iso",
            "format",
            "hash",
            "kind",
            "payload",
            "schema"
        ]
    );
}

#[test]
fn tampering_is_caught_by_the_hash() {
    let (_, bytes) = export(500);
    let text = String::from_utf8(bytes).unwrap();
    let edited = text.replacen("\"name\":\"P0\"", "\"name\":\"Pz\"", 1);
    assert_ne!(edited, text);
    assert!(matches!(
        import_world(edited.as_bytes(), &ImportOptions::default()),
        Err(ImportError::HashMismatch)
    ));
}

#[test]
fn wrong_files_are_refused_with_plain_messages() {
    let opts = ImportOptions::default();
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"not json at all".to_vec(), "not valid export data"),
        (br#"{"format":"something-else"}"#.to_vec(), ""),
        (b"[1,2,3]".to_vec(), ""),
        (br#"{"a":1.5}"#.to_vec(), "not valid export data"),
        (vec![0xFF, 0xFE, 0x00], "not UTF-8"),
        (Vec::new(), "not valid export data"),
    ];
    for (bytes, needle) in cases {
        let e = import_world(&bytes, &opts)
            .err()
            .expect("must fail")
            .to_string();
        assert!(e.contains(needle), "{e}");
    }
    let (_, bytes) = export(10);
    let text = String::from_utf8(bytes)
        .unwrap()
        .replacen("\"kind\":\"world\"", "\"kind\":\"pack\"", 1);
    let e = import_world(text.as_bytes(), &opts)
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("not a world"), "{e}");
}

#[test]
fn newer_schemas_and_oversized_files_are_refused() {
    let (w, _) = export(10);
    let e = import_world(&doc_with(w.to_canon(), 99), &ImportOptions::default())
        .err()
        .unwrap();
    assert!(
        matches!(
            e,
            ImportError::Migrate(MigrateError::Newer { found: 99, .. })
        ),
        "{e}"
    );
    let (_, bytes) = export(10);
    let small = ImportOptions {
        max_bytes: 100,
        ..ImportOptions::default()
    };
    assert!(matches!(
        import_world(&bytes, &small),
        Err(ImportError::TooLarge { .. })
    ));
}

#[test]
fn a_structurally_invalid_world_is_refused_with_a_path() {
    let (w, _) = export(500);
    let mut payload = w.to_canon();
    if let Canon::Map(m) = &mut payload {
        m.insert("clock".into(), Canon::map([("tick", Canon::str("never"))]));
    }
    let e = import_world(
        &doc_with(payload, i128::from(w.schema)),
        &ImportOptions::default(),
    )
    .err()
    .unwrap();
    assert!(matches!(e, ImportError::Invalid(_)));
    assert!(e.to_string().contains("clock.tick"), "{e}");
}

#[test]
fn a_failed_import_writes_nothing() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("keep", &world_at(100), &[], "t").unwrap();
    let before = mem.names();
    let _ = import_world(b"garbage", &ImportOptions::default());
    assert_eq!(mem.names(), before);
}

#[test]
fn the_compat_report_travels_with_the_plan() {
    let (_, bytes) = export(10);
    let opts = ImportOptions {
        installed_refs: Some(Vec::new()),
        ..ImportOptions::default()
    };
    let plan = import_world(&bytes, &opts).unwrap();
    assert!(
        plan.compat.unwrap().is_blocking(),
        "the export needs 'base', which is not installed"
    );
}

proptest! {
    #[test]
    fn mutated_exports_never_panic(pos in 0usize..100_000, byte in any::<u8>()) {
        let (_, bytes) = export(300);
        let mut bytes = bytes;
        let i = pos % bytes.len();
        bytes[i] = byte;
        let _ = import_world(&bytes, &ImportOptions::default());
    }
}
