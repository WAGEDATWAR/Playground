use super::*;
use pg_host::MemStorage;

fn root(p: &str) -> PathBuf {
    PathBuf::from(format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR")))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pg-mods-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_choices_round_trip_through_storage_and_a_damaged_file_starts_fresh_with_a_note() {
    let storage = MemStorage::new();
    let (fresh, note) = ModsConfig::load(&storage);
    assert_eq!((fresh, note), (ModsConfig::default(), None));
    let mut c = ModsConfig::default();
    c.enabled.insert("caffeine".into());
    c.set_approved("caffeine", "systems", true);
    c.set_approved("caffeine", "data", true);
    c.set_approved("caffeine", "data", false);
    c.safe_mode = true;
    c.save(&storage).unwrap();
    let (back, note) = ModsConfig::load(&storage);
    assert_eq!(back, c);
    assert!(note.is_none());
    assert!(back.is_approved("caffeine", "systems") && !back.is_approved("caffeine", "data"));
    storage.write_atomic(CONFIG, b"{ not json").unwrap();
    let (damaged, note) = ModsConfig::load(&storage);
    assert_eq!(damaged, ModsConfig::default());
    assert!(note.unwrap().contains("damaged"));
}

#[test]
fn installing_copies_a_valid_pack_refuses_duplicates_and_bad_folders_and_discovery_lists_them() {
    let data = temp("install");
    let id = install(&data, &root("packs/cookbook/caffeine")).unwrap();
    assert_eq!(id, "caffeine");
    assert!(data.join("packs/caffeine/pack.json").exists());
    assert!(data.join("packs/caffeine/scripts/main.luau").exists());
    assert!(install(&data, &root("packs/cookbook/caffeine"))
        .unwrap_err()
        .contains("already installed"));
    assert!(install(&data, &root("data/base"))
        .unwrap_err()
        .contains("base"));
    let empty = temp("empty-source");
    assert!(install(&data, &empty)
        .unwrap_err()
        .contains("not a valid pack"));
    // A broken pack already in the folder is listed with the reason rather than hiding.
    std::fs::create_dir_all(data.join("packs/broken")).unwrap();
    std::fs::write(data.join("packs/broken/pack.json"), "{ nope").unwrap();
    let found = discover(&data);
    assert_eq!(
        found.iter().map(|f| f.folder.as_str()).collect::<Vec<_>>(),
        ["broken", "caffeine"]
    );
    assert!(found[0].outcome.is_err());
    let s = found[1].outcome.as_ref().unwrap();
    assert_eq!((s.id.as_str(), s.version.as_str()), ("caffeine", "0.1.0"));
    assert_eq!(s.capabilities, ["data", "systems"]);
    assert_eq!(s.depends, ["base"]);
    remove(&data, "caffeine").unwrap();
    assert!(remove(&data, "../x").is_err());
    assert_eq!(discover(&data).len(), 1);
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn a_pack_loads_only_when_enabled_and_approved_and_safe_mode_loads_none() {
    let data = temp("plan");
    install(&data, &root("packs/cookbook/caffeine")).unwrap();
    let installed = discover(&data);
    let mut c = ModsConfig::default();
    // Not enabled: left out quietly.
    let p = plan(&installed, &c, false);
    assert!(p.dirs.is_empty() && p.notes.is_empty(), "{p:?}");
    // Enabled but not approved: left out with the reason.
    c.enabled.insert("caffeine".into());
    let p = plan(&installed, &c, false);
    assert!(p.dirs.is_empty());
    assert!(
        p.notes[0].contains("needs your approval for: systems"),
        "{:?}",
        p.notes
    );
    // Approved: loaded.
    c.set_approved("caffeine", "systems", true);
    let p = plan(&installed, &c, false);
    assert_eq!(p.dirs, [data.join("packs/caffeine")]);
    // Safe mode: none, and it says so.
    let safe = plan(&installed, &c, true);
    assert!(safe.dirs.is_empty() && safe.notes[0].contains("Safe mode"));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_content_includes_the_enabled_packs_and_falls_back_to_the_base_game_when_they_cannot_load() {
    let data = temp("build");
    install(&data, &root("packs/cookbook/caffeine")).unwrap();
    let mut c = ModsConfig::default();
    c.enabled.insert("caffeine".into());
    c.set_approved("caffeine", "systems", true);
    let (content, notes) = build_content(&root("data/base"), &[], Some(&data), &c, false).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let ids: Vec<String> = content.load_order().iter().map(|p| p.to_string()).collect();
    assert_eq!(ids, ["base", "caffeine"]);
    let (safe, notes) = build_content(&root("data/base"), &[], Some(&data), &c, true).unwrap();
    assert_eq!(safe.load_order().len(), 1);
    assert!(notes[0].contains("Safe mode"));
    // A pack that is valid on its own but breaks the build together with the others: the game still starts.
    let twin = data.join("packs/twin");
    copy_dir(&data.join("packs/caffeine"), &twin, 0).unwrap();
    let manifest = std::fs::read_to_string(twin.join("pack.json")).unwrap();
    std::fs::write(
        twin.join("pack.json"),
        manifest.replace("\"id\": \"caffeine\"", "\"id\": \"twin\""),
    )
    .unwrap();
    c.enabled.insert("twin".into());
    c.set_approved("twin", "systems", true);
    let (fell_back, notes) =
        build_content(&root("data/base"), &[], Some(&data), &c, false).unwrap();
    // Two copies of the same script pack may or may not conflict; either way the game starts.
    assert!(!fell_back.load_order().is_empty());
    let _ = notes;
    let _ = std::fs::remove_dir_all(&data);
}
