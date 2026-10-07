use super::*;
use crate::codec;
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::sim::Sim;
use pg_host::{Faults, MemStorage};

fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

/// A small living world at the given tick.
fn world_at(ticks: u64) -> WorldState {
    let mut sim = Sim::with_dev_systems(WorldState::new("Slot Town", "slot-seed"));
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 24,
            h: 18,
            style: 1,
        }),
    )
    .unwrap();
    let map = EntityId::new(Kind::Map, 1);
    for i in 0..4 {
        sim.submit(
            0,
            cmd(Command::DevSpawnPawn {
                map,
                at: None,
                name: format!("P{i}"),
            }),
        )
        .unwrap();
    }
    sim.run_ticks(ticks).unwrap();
    sim.world().clone()
}

fn refs() -> Vec<ContentRefRecord> {
    vec![ContentRefRecord {
        pack_id: "base".into(),
        version: "0.1.0".into(),
        hash: "abcdef0123456789".into(),
    }]
}

fn load(s: &SlotStore<'_>, id: &str) -> Result<Loaded, LoadError> {
    s.load(id, &LoadOptions::default())
}

#[test]
fn save_then_load_gives_the_same_world() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    let w = world_at(3_300);
    let report = store
        .save("town", &w, &refs(), "2026-10-06T12:00:00Z")
        .unwrap();
    assert_eq!(report.generation, 1);
    assert!(report.compressed_bytes < report.uncompressed_bytes);
    let loaded = load(&store, "town").unwrap();
    assert_eq!(loaded.world.state_hash(), w.state_hash());
    assert_eq!(loaded.world.to_canon(), w.to_canon());
    assert_eq!(loaded.recovery, Recovery::Clean);
    assert_eq!(loaded.migrated_from, None);
    let m = loaded.manifest.unwrap();
    assert_eq!((m.name.as_str(), m.generations.len()), ("Slot Town", 1));
    assert_eq!(m.current().unwrap().play_ticks, 3_300);
    assert_eq!(m.content_refs, refs());
}

#[test]
fn exactly_two_generations_are_kept() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    for (i, ticks) in [100u64, 200, 300, 400].into_iter().enumerate() {
        let r = store.save("town", &world_at(ticks), &[], "t").unwrap();
        assert_eq!(r.generation, i as u64 + 1);
        assert!(r.leftovers.is_empty());
    }
    let names: Vec<String> = mem
        .names()
        .into_iter()
        .filter(|n| n.ends_with(".pgsave"))
        .collect();
    assert_eq!(
        names,
        ["worlds/town/state.3.pgsave", "worlds/town/state.4.pgsave"]
    );
    let m = store.manifest("town").unwrap();
    assert_eq!(
        m.generations
            .iter()
            .map(|g| g.generation)
            .collect::<Vec<_>>(),
        [4, 3]
    );
}

#[test]
fn bad_ids_and_missing_worlds_are_refused() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    for bad in ["", "Town", "a/b", "../x", "with space", &"x".repeat(41)] {
        assert!(
            matches!(
                store.save(bad, &world_at(10), &[], "t"),
                Err(SaveError::BadWorldId(_))
            ),
            "{bad:?}"
        );
        assert!(matches!(load(&store, bad), Err(LoadError::NotFound)));
    }
    assert!(matches!(load(&store, "nothing"), Err(LoadError::NotFound)));
}

#[test]
fn worlds_are_listed_and_independent() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("beta", &world_at(10), &[], "t").unwrap();
    store.save("alpha", &world_at(20), &[], "t").unwrap();
    assert_eq!(store.list_worlds().unwrap(), ["alpha", "beta"]);
}

// ---- fault injection: a crash at ANY point of a save leaves a loadable world ------------------------

#[test]
fn a_crash_at_every_step_of_a_save_leaves_the_old_or_the_new_world() {
    let old = world_at(1_000);
    let new = world_at(2_000);
    let (old_hash, new_hash) = (old.state_hash(), new.state_hash());
    let base = MemStorage::new();
    SlotStore::new(&base)
        .save("town", &old, &refs(), "t1")
        .unwrap();

    // Count the storage operations one save performs.
    let probe = base.duplicate();
    SlotStore::new(&probe)
        .save("town", &new, &refs(), "t2")
        .unwrap();
    let total_ops = probe.ops();
    assert!(total_ops >= 5, "{total_ops}");

    let mut saw_old = false;
    let mut saw_new = false;
    for fail_at in 0..=total_ops {
        let mem = base.duplicate();
        mem.set_faults(Faults {
            fail_from_op: Some(fail_at),
            ..Faults::default()
        });
        let _ = SlotStore::new(&mem).save("town", &new, &refs(), "t2");
        mem.set_faults(Faults::default());
        let loaded = load(&SlotStore::new(&mem), "town")
            .unwrap_or_else(|e| panic!("crash at op {fail_at}: unloadable: {e}"));
        let h = loaded.world.state_hash();
        assert!(
            h == old_hash || h == new_hash,
            "crash at op {fail_at}: loaded a third world"
        );
        saw_old |= h == old_hash;
        saw_new |= h == new_hash;
        assert!(!SlotStore::new(&mem).is_marked_damaged("town"));
    }
    assert!(
        saw_old && saw_new,
        "the sweep covered both sides of the commit point"
    );
}

#[test]
fn torn_writes_and_a_full_disk_never_cost_the_old_save() {
    let old = world_at(500);
    let new = world_at(900);
    for faults in [
        Faults {
            torn_write: Some(0),
            ..Faults::default()
        }, // the new generation is half written
        Faults {
            torn_write: Some(1),
            ..Faults::default()
        }, // the manifest is half written
        Faults {
            disk_full: true,
            ..Faults::default()
        },
    ] {
        let mem = MemStorage::new();
        SlotStore::new(&mem).save("town", &old, &[], "t1").unwrap();
        mem.reset_counters(); // fault indexes count from here
        mem.set_faults(faults);
        assert!(
            SlotStore::new(&mem).save("town", &new, &[], "t2").is_err(),
            "{faults:?}"
        );
        mem.set_faults(Faults::default());
        let loaded = load(&SlotStore::new(&mem), "town").unwrap();
        let h = loaded.world.state_hash();
        assert!(h == old.state_hash() || h == new.state_hash(), "{faults:?}");
        // The next save repairs everything and cleans up.
        let r = SlotStore::new(&mem).save("town", &new, &[], "t3").unwrap();
        assert!(r.leftovers.is_empty());
        assert_eq!(
            load(&SlotStore::new(&mem), "town")
                .unwrap()
                .world
                .state_hash(),
            new.state_hash()
        );
        let files = mem
            .names()
            .into_iter()
            .filter(|n| n.ends_with(".pgsave"))
            .count();
        assert!(files <= 2, "{faults:?}: {files} generation files");
    }
}

// ---- recovery ----------------------------------------------------------------------------------------

fn two_generations() -> (MemStorage, WorldState, WorldState) {
    let mem = MemStorage::new();
    let (a, b) = (world_at(1_000), world_at(2_500));
    let store = SlotStore::new(&mem);
    store.save("town", &a, &refs(), "t1").unwrap();
    store.save("town", &b, &refs(), "t2").unwrap();
    (mem, a, b)
}

#[test]
fn a_corrupt_newest_generation_falls_back_and_reports_the_time_lost() {
    for damage in ["flip", "truncate", "empty", "garbage", "delete"] {
        let (mem, a, _) = two_generations();
        let name = "worlds/town/state.2.pgsave";
        let mut bytes = mem.get_raw(name).unwrap();
        match damage {
            "flip" => {
                let i = bytes.len() / 2;
                bytes[i] ^= 0xFF;
                mem.put_raw(name, bytes);
            }
            "truncate" => mem.put_raw(name, bytes[..bytes.len() / 3].to_vec()),
            "empty" => mem.put_raw(name, Vec::new()),
            "garbage" => mem.put_raw(name, vec![0xAB; 500]),
            _ => mem.delete(name).unwrap(),
        }
        let loaded =
            load(&SlotStore::new(&mem), "town").unwrap_or_else(|e| panic!("{damage}: {e}"));
        assert_eq!(loaded.world.state_hash(), a.state_hash(), "{damage}");
        assert_eq!(loaded.generation, 1);
        match loaded.recovery {
            Recovery::FellBack {
                wanted: 2,
                used: 1,
                ticks_lost,
                ..
            } => assert_eq!(ticks_lost, Some(1_500), "{damage}"),
            other => panic!("{damage}: {other:?}"),
        }
    }
}

#[test]
fn losing_the_manifest_still_finds_the_newest_valid_generation() {
    for damage in ["delete", "garbage", "wrong_format"] {
        let (mem, _, b) = two_generations();
        match damage {
            "delete" => mem.delete("worlds/town/manifest.json").unwrap(),
            "garbage" => mem.put_raw("worlds/town/manifest.json", b"{{{{".to_vec()),
            _ => mem.put_raw(
                "worlds/town/manifest.json",
                br#"{"format":"other"}"#.to_vec(),
            ),
        }
        let loaded =
            load(&SlotStore::new(&mem), "town").unwrap_or_else(|e| panic!("{damage}: {e}"));
        assert_eq!(loaded.world.state_hash(), b.state_hash(), "{damage}");
        assert_eq!(loaded.recovery, Recovery::NoManifest { used: 2 });
        // And the next save rebuilds a proper manifest.
        SlotStore::new(&mem).save("town", &b, &[], "t").unwrap();
        assert_eq!(
            load(&SlotStore::new(&mem), "town").unwrap().recovery,
            Recovery::Clean
        );
    }
}

#[test]
fn when_everything_is_damaged_the_slot_is_marked_and_its_files_are_kept() {
    let (mem, _, _) = two_generations();
    mem.put_raw("worlds/town/state.1.pgsave", vec![1, 2, 3]);
    mem.put_raw("worlds/town/state.2.pgsave", vec![4, 5, 6]);
    // Another slot must be untouched by all this.
    SlotStore::new(&mem)
        .save("other", &world_at(50), &[], "t")
        .unwrap();
    let before_other: Vec<_> = mem
        .names()
        .into_iter()
        .filter(|n| n.contains("worlds/other/"))
        .collect();

    let err = load(&SlotStore::new(&mem), "town").unwrap_err();
    let LoadError::Damaged(attempts) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(attempts.len(), 2);
    assert!(err.to_string().contains("damaged") && err.to_string().contains("kept"));
    assert!(SlotStore::new(&mem).is_marked_damaged("town"));
    assert_eq!(
        mem.get_raw("worlds/town/state.2.pgsave").unwrap(),
        vec![4, 5, 6],
        "files are kept"
    );
    let after_other: Vec<_> = mem
        .names()
        .into_iter()
        .filter(|n| n.contains("worlds/other/"))
        .collect();
    assert_eq!(before_other, after_other);
    assert!(load(&SlotStore::new(&mem), "other").is_ok());
    // A fresh save heals the slot and clears the mark.
    SlotStore::new(&mem)
        .save("town", &world_at(60), &[], "t")
        .unwrap();
    assert!(!SlotStore::new(&mem).is_marked_damaged("town"));
    assert!(load(&SlotStore::new(&mem), "town").is_ok());
}

#[test]
fn swapped_generation_files_fail_the_manifest_hash_check() {
    let (mem, _, _) = two_generations();
    let (one, two) = (
        mem.get_raw("worlds/town/state.1.pgsave").unwrap(),
        mem.get_raw("worlds/town/state.2.pgsave").unwrap(),
    );
    mem.put_raw("worlds/town/state.1.pgsave", two);
    mem.put_raw("worlds/town/state.2.pgsave", one);
    assert!(matches!(
        load(&SlotStore::new(&mem), "town"),
        Err(LoadError::Damaged(_))
    ));
}

#[test]
fn a_save_from_a_newer_version_is_refused_not_marked_damaged() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("town", &world_at(100), &[], "t").unwrap();
    // Rewrite the newest file as if a future build had written it.
    let (_, payload) =
        codec::decode(&mem.get_raw("worlds/town/state.1.pgsave").unwrap(), 1 << 30).unwrap();
    let text = String::from_utf8(payload)
        .unwrap()
        .replacen("\"schema\":3", "\"schema\":99", 1);
    mem.put_raw(
        "worlds/town/state.1.pgsave",
        codec::encode(99, text.as_bytes()),
    );
    let err = load(&store, "town").unwrap_err();
    assert!(
        matches!(err, LoadError::Newer(MigrateError::Newer { found: 99, .. })),
        "{err:?}"
    );
    assert!(err.to_string().contains("newer version"));
    assert!(!store.is_marked_damaged("town"));
}

#[test]
fn header_and_data_schema_must_agree() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("town", &world_at(100), &[], "t").unwrap();
    let (_, payload) =
        codec::decode(&mem.get_raw("worlds/town/state.1.pgsave").unwrap(), 1 << 30).unwrap();
    mem.put_raw("worlds/town/state.1.pgsave", codec::encode(2, &payload));
    assert!(matches!(load(&store, "town"), Err(LoadError::Damaged(_))));
}

#[test]
fn a_decompression_bomb_is_refused_by_the_size_limit() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("town", &world_at(100), &[], "t").unwrap();
    let opts = LoadOptions {
        max_uncompressed: 1_000,
        ..LoadOptions::default()
    };
    let err = store.load("town", &opts).unwrap_err();
    assert!(err.to_string().contains("limit"), "{err}");
}

#[test]
fn the_content_compatibility_report_comes_with_the_load() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("town", &world_at(100), &refs(), "t").unwrap();
    let mut installed = refs();
    installed[0].version = "0.2.0".into();
    let opts = LoadOptions {
        installed_refs: Some(installed),
        ..LoadOptions::default()
    };
    let loaded = store.load("town", &opts).unwrap();
    let rep = loaded.compat.unwrap();
    assert!(!rep.is_exact() && !rep.is_blocking());
    assert!(rep.explain()[0].contains("0.2.0"));
    assert!(
        load(&store, "town").unwrap().compat.is_none(),
        "no installed refs given, no report"
    );
}

#[test]
fn manifests_round_trip() {
    let m = Manifest {
        name: "N".into(),
        seed_text: "s".into(),
        schema: 3,
        content_refs: refs(),
        summary: Some(WorldSummary {
            population: 12,
            maps: 1,
            objects: 3,
            thumbnail: Some("worlds/x/thumb.5.png".into()),
        }),
        generations: vec![GenerationInfo {
            generation: 5,
            state_hash: "ab".into(),
            play_ticks: 9,
            day: 1,
            saved_iso: "x".into(),
        }],
    };
    let root = Root::new(m.to_canon());
    assert_eq!(Manifest::from_reader(root.reader()).unwrap(), m);
    // More than two generations, or none, is refused.
    let mut bad = m.clone();
    bad.generations = Vec::new();
    assert!(Manifest::from_reader(Root::new(bad.to_canon()).reader()).is_err());
}

#[test]
fn saves_record_a_summary_for_the_saved_worlds_list() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    let w = world_at(2_000);
    store.save("town", &w, &refs(), "t").unwrap();
    let m = store.manifest("town").unwrap();
    let s = m.summary.clone().unwrap();
    assert_eq!(
        (s.population, s.maps, s.objects, s.thumbnail),
        (4, 1, 0, None)
    );
    assert_eq!(
        (m.name.as_str(), m.current().unwrap().day),
        ("Slot Town", 0)
    );
}

#[test]
fn manifests_written_before_summaries_still_load() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    store.save("town", &world_at(500), &[], "t").unwrap();
    // Strip the summary from the stored manifest, as an older build would have written it.
    let text = String::from_utf8(mem.get_raw("worlds/town/manifest.json").unwrap()).unwrap();
    let mut doc = json::parse(&text).unwrap();
    if let Canon::Map(m) = &mut doc {
        assert!(m.remove("summary").is_some());
    }
    mem.put_raw(
        "worlds/town/manifest.json",
        doc.to_canonical_string().into_bytes(),
    );
    let m = store.manifest("town").unwrap();
    assert!(m.summary.is_none());
    assert_eq!(load(&store, "town").unwrap().recovery, Recovery::Clean);
}

#[test]
fn thumbnails_are_stored_named_in_the_summary_and_cleaned_up_with_old_generations() {
    let mem = MemStorage::new();
    let store = SlotStore::new(&mem);
    for (i, ticks) in [100u64, 200, 300, 400].into_iter().enumerate() {
        let png = vec![0x89, b'P', b'N', b'G', i as u8];
        store
            .save_with("town", &world_at(ticks), &[], "t", Some(&png))
            .unwrap();
    }
    let m = store.manifest("town").unwrap();
    assert_eq!(
        m.summary.unwrap().thumbnail.as_deref(),
        Some("worlds/town/thumb.4.png")
    );
    let thumbs: Vec<String> = mem
        .names()
        .into_iter()
        .filter(|n| n.contains("thumb."))
        .collect();
    assert_eq!(
        thumbs,
        ["worlds/town/thumb.3.png", "worlds/town/thumb.4.png"],
        "only the kept generations keep thumbnails"
    );
    assert_eq!(mem.get_raw("worlds/town/thumb.4.png").unwrap()[4], 3);
    // A save without a thumbnail does not claim one.
    store.save("town", &world_at(500), &[], "t").unwrap();
    assert!(store
        .manifest("town")
        .unwrap()
        .summary
        .unwrap()
        .thumbnail
        .is_none());
}

#[test]
fn deleting_a_world_removes_every_file_of_that_world_and_only_that_world() {
    let storage = MemStorage::new();
    let store = SlotStore::new(&storage);
    let w = world_at(300);
    store
        .save_with("keep", &w, &[], "2026-01-01T00:00:00Z", Some(b"png"))
        .unwrap();
    store
        .save_with("drop", &w, &[], "2026-01-01T00:00:00Z", Some(b"png"))
        .unwrap();
    store.save("drop", &w, &[], "2026-01-02T00:00:00Z").unwrap();
    assert_eq!(store.list_worlds().unwrap(), ["drop", "keep"]);
    let removed = store.delete_world("drop").unwrap();
    assert!(
        removed >= 4,
        "generations, manifest and thumbnail: {removed}"
    );
    assert_eq!(store.list_worlds().unwrap(), ["keep"]);
    assert!(store.load("keep", &LoadOptions::default()).is_ok());
    assert_eq!(
        store.delete_world("drop").unwrap(),
        0,
        "deleting twice is harmless"
    );
    assert_eq!(
        store.delete_world("../etc").unwrap(),
        0,
        "bad ids remove nothing"
    );
}
