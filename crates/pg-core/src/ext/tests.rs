use super::*;
use crate::map::{MapKind, Tile};

fn schemas() -> ExtSchemas {
    let mut s = ExtSchemas::new();
    s.register(ComponentDef {
        name: "coffee.caffeine".into(),
        pack: "coffee".into(),
        applies_to: vec![Kind::Pawn],
        fields: vec![
            FieldDef {
                name: "level".into(),
                min: 0,
                max: 1000,
                default: 10,
            },
            FieldDef {
                name: "tolerance".into(),
                min: 0,
                max: 5,
                default: 0,
            },
        ],
        version: 1,
    })
    .unwrap();
    s
}

fn world() -> (WorldState, EntityId) {
    let mut w = WorldState::new("Ext", "seed");
    let m = w.create_map(MapKind::Overworld, 10, 10).unwrap();
    let p = w.spawn_pawn("Ann", m, Tile::new(1, 1)).unwrap();
    (w, p)
}

#[test]
fn an_empty_store_changes_nothing_about_hashes_or_canonical_form() {
    let (w, _) = world();
    assert!(w.ext.is_empty());
    assert!(w.to_canon().get("ext").is_none());
    assert!(w.table_hashes().iter().all(|(n, _)| *n != "ext"));
    assert!(w.row_hashes("ext").is_none());
}

#[test]
fn writes_are_validated_like_built_in_code() {
    let (mut w, p) = world();
    let s = schemas();
    let before = w.state_hash();
    let map = EntityId::new(Kind::Map, 1);
    assert!(matches!(
        apply_set_field(&mut w, &s, "coffee", p, "coffee.nope", "level", 1),
        Err(ExtError::UnknownComponent(_))
    ));
    assert!(matches!(
        apply_set_field(&mut w, &s, "other", p, "coffee.caffeine", "level", 1),
        Err(ExtError::NotOwner { .. })
    ));
    assert!(matches!(
        apply_set_field(
            &mut w,
            &s,
            "coffee",
            EntityId::new(Kind::Pawn, 99),
            "coffee.caffeine",
            "level",
            1
        ),
        Err(ExtError::UnknownEntity(_))
    ));
    assert!(matches!(
        apply_set_field(&mut w, &s, "coffee", map, "coffee.caffeine", "level", 1),
        Err(ExtError::WrongKind { .. })
    ));
    assert!(matches!(
        apply_set_field(&mut w, &s, "coffee", p, "coffee.caffeine", "nope", 1),
        Err(ExtError::UnknownField { .. })
    ));
    assert!(matches!(
        apply_set_field(&mut w, &s, "coffee", p, "coffee.caffeine", "level", 1001),
        Err(ExtError::OutOfRange { .. })
    ));
    assert!(matches!(
        apply_set_field(&mut w, &s, "coffee", p, "coffee.caffeine", "level", -1),
        Err(ExtError::OutOfRange { .. })
    ));
    assert_eq!(w.state_hash(), before, "refused writes change nothing");
    apply_set_field(&mut w, &s, "coffee", p, "coffee.caffeine", "level", 600).unwrap();
    assert_ne!(w.state_hash(), before);
    let def = s.get("coffee.caffeine").unwrap();
    let f = w.ext.fields_of(def, p);
    assert_eq!(
        (f["level"], f["tolerance"]),
        (600, 0),
        "unwritten fields keep their defaults"
    );
    assert!(ExtError::UnknownEntity(p).to_string().contains("pawn_"));
}

#[test]
fn defaults_apply_to_every_entity_of_the_kind() {
    let (w, p) = world();
    let s = schemas();
    let f = w.ext.fields_of(s.get("coffee.caffeine").unwrap(), p);
    assert_eq!(f["level"], 10);
}

#[test]
fn the_store_round_trips_and_localises_changes_by_row() {
    let (mut w, p) = world();
    let s = schemas();
    apply_set_field(&mut w, &s, "coffee", p, "coffee.caffeine", "level", 7).unwrap();
    w.ext.record_error("coffee", 40, 3, 1000, false);
    let back = WorldState::from_canon(&w.to_canon()).unwrap();
    assert_eq!(back.ext, w.ext);
    assert_eq!(back.state_hash(), w.state_hash());
    let mut other = w.clone();
    apply_set_field(&mut other, &s, "coffee", p, "coffee.caffeine", "level", 8).unwrap();
    assert_eq!(
        crate::world::differing_rows(&w, &other, "ext"),
        vec![format!("coffee.caffeine/{p}")]
    );
}

#[test]
fn orphan_values_survive_a_missing_pack_and_still_hash() {
    let (mut w, p) = world();
    apply_set_field(
        &mut w,
        &schemas(),
        "coffee",
        p,
        "coffee.caffeine",
        "level",
        7,
    )
    .unwrap();
    // The same world restored with no schemas keeps the values.
    let back = WorldState::from_canon(&w.to_canon()).unwrap();
    assert_eq!(back.ext.stored("coffee.caffeine", p, "level"), Some(7));
    assert_eq!(back.state_hash(), w.state_hash());
}

#[test]
fn errors_accumulate_into_a_quarantine_inside_a_window() {
    let mut s = ExtStore::new();
    assert!(!s.record_error("p", 10, 3, 100, false));
    assert!(!s.record_error("p", 20, 3, 100, false));
    assert!(
        s.record_error("p", 30, 3, 100, false),
        "the third error inside the window quarantines"
    );
    assert!(s.is_quarantined("p"));
    assert!(
        !s.record_error("p", 31, 3, 100, false),
        "already quarantined"
    );
    // Old errors fall out of the window.
    assert!(!s.record_error("q", 10, 3, 50, false));
    assert!(!s.record_error("q", 20, 3, 50, false));
    assert!(!s.record_error("q", 200, 3, 50, false));
    assert!(!s.is_quarantined("q"));
    // A load failure quarantines at once.
    assert!(s.record_error("r", 5, 3, 50, true));
    s.clear_health("r");
    assert!(!s.is_quarantined("r") && s.health("r").is_none());
}

#[test]
fn bad_saved_ext_data_is_rejected_with_a_path() {
    let (mut w, p) = world();
    apply_set_field(
        &mut w,
        &schemas(),
        "coffee",
        p,
        "coffee.caffeine",
        "level",
        7,
    )
    .unwrap();
    let mut c = w.to_canon();
    if let Canon::Map(m) = &mut c {
        m.insert(
            "ext".into(),
            Canon::map([(
                "values",
                Canon::map([(
                    "c.x",
                    Canon::map([("not_an_id", Canon::map([("a", Canon::Int(1))]))]),
                )]),
            )]),
        );
    }
    let e = WorldState::from_canon(&c).unwrap_err().to_string();
    assert!(e.contains("not_an_id"), "{e}");
}
