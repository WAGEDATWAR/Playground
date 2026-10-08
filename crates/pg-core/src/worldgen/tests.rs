use super::*;
use crate::canon::ToCanon;
use pg_content::gamedata::{parse_files, KNOWN_FILES};
use pg_content::report::ValidationReport;

fn data() -> GameData {
    let root = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let files = KNOWN_FILES
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(format!("{root}/{p}")).ok()?;
            Some(((*p).to_owned(), crate::canon::json::parse(&text).ok()?))
        })
        .collect();
    let mut report = ValidationReport::new();
    let d = parse_files(&files, &mut report);
    assert!(report.is_ok(), "{report}");
    d
}

fn generate(seed: &str, w: i32, h: i32, water: u32, residents: usize) -> (WorldState, GenOutcome) {
    let mut world = WorldState::new("Gen", seed);
    let out = generate_town(
        &mut world,
        &data(),
        &GenParams {
            width: w,
            height: h,
            water_percent: water,
            residents,
            tone: TonePreset::Standard,
        },
    )
    .unwrap_or_else(|e| panic!("{seed} {w}x{h}: {e}"));
    (world, out)
}

fn params(w: i32, h: i32, residents: usize) -> GenParams {
    GenParams {
        width: w,
        height: h,
        water_percent: 18,
        residents,
        tone: TonePreset::Standard,
    }
}

#[test]
fn the_same_seed_gives_the_same_town_and_the_starting_hash_can_be_recomputed() {
    let (a, out_a) = generate("repeat", 64, 48, 18, 14);
    let (b, out_b) = generate("repeat", 64, 48, 18, 14);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(render_ascii(&a), render_ascii(&b));
    assert_eq!(out_a, out_b);
    let (c, _) = generate("another", 64, 48, 18, 14);
    assert_ne!(a.state_hash(), c.state_hash());
    // The stored starting hash is the hash of the world before it was recorded.
    let mut bare = a.clone();
    bare.town.starting_hash = None;
    assert_eq!(
        a.town.starting_hash.as_deref(),
        Some(bare.state_hash().to_hex().as_str())
    );
    assert_eq!(out_a.starting_hash, bare.state_hash().to_hex());
}

#[test]
fn many_seeds_and_sizes_all_generate_valid_towns_with_homes_for_everyone() {
    let sizes = [(48, 36), (64, 48), (96, 72)];
    let mut retried = 0;
    for (i, seed) in (0..24).map(|n| format!("seed-{n}")).enumerate() {
        for (w, h) in sizes {
            let residents = [10, 14, 20][i % 3];
            let (world, out) = generate(&seed, w, h, 10 + (i as u32 % 5) * 6, residents);
            retried += usize::from(out.attempt > 0);
            validate_world(&world).unwrap_or_else(|e| panic!("{seed} {w}x{h}: {e}"));
            assert_eq!(world.pawns.len(), residents);
            let homes: BTreeSet<Tile> = world
                .pawns
                .iter()
                .filter_map(|(_, p)| p.home_tile)
                .collect();
            assert_eq!(
                homes.len(),
                residents,
                "every resident has a tile of their own"
            );
            for (_, p) in world.pawns.iter() {
                assert_eq!(
                    Some(p.position.tile),
                    p.home_tile,
                    "residents start at home"
                );
                assert!(p.household.is_some() && p.occupation.is_some());
            }
            assert!(world.town.districts.len() >= 2);
            let homes_built = world.town.buildings_with(BuildingRole::Home).count();
            assert!(
                homes_built >= world.households.len(),
                "{seed}: {homes_built} homes for {} households",
                world.households.len()
            );
            assert!(!world.town.gathering.is_empty());
        }
    }
    assert!(
        retried <= 12,
        "retries are the exception, not the rule: {retried} of 72"
    );
}

#[test]
fn the_generator_controls_do_what_they_say() {
    let water_share = |w: &WorldState| {
        let m = w.maps.iter().next().unwrap().1;
        let all = (m.width() * m.height()) as usize;
        let water = (0..m.height())
            .flat_map(|y| (0..m.width()).map(move |x| Tile::new(x, y)))
            .filter(|t| m.terrain_at(*t) == Some(WATER))
            .count();
        water * 100 / all
    };
    let dry = generate("controls", 64, 48, 0, 12).0;
    assert_eq!(water_share(&dry), 0, "no water was asked for");
    let wet = generate("controls", 64, 48, 35, 12).0;
    let share = water_share(&wet);
    assert!(
        (30..=50).contains(&share),
        "35 percent asked for, {share} made"
    );
    // Size and population are exact.
    let (w, _) = generate("controls", 48, 36, 18, 0);
    assert_eq!((w.pawns.len(), w.households.len()), (0, 0));
    let m = w.maps.iter().next().unwrap().1;
    assert_eq!((m.width(), m.height()), (48, 36));
    // The tone preset is kept with the world.
    let mut world = WorldState::new("Tone", "tone");
    generate_town(
        &mut world,
        &data(),
        &GenParams {
            width: 48,
            height: 36,
            water_percent: 10,
            residents: 8,
            tone: TonePreset::Cozy,
        },
    )
    .unwrap();
    assert_eq!(world.settings.tone, TonePreset::Cozy);
}

#[test]
fn bad_requests_are_refused_without_touching_the_world() {
    let d = data();
    let mut world = WorldState::new("Bad", "bad");
    let before = world.state_hash();
    assert!(matches!(
        generate_town(&mut world, &d, &params(10, 48, 10)),
        Err(GenError::BadParams(_))
    ));
    assert!(matches!(
        generate_town(&mut world, &d, &params(64, 300, 10)),
        Err(GenError::BadParams(_))
    ));
    assert!(matches!(
        generate_town(&mut world, &d, &params(64, 48, 51)),
        Err(GenError::Plan(PlanError::TooMany(51)))
    ));
    assert!(matches!(
        generate_town(&mut world, &GameData::default(), &params(64, 48, 5)),
        Err(GenError::MissingData(_))
    ));
    assert_eq!(world.state_hash(), before, "refusals change nothing");
    generate_town(&mut world, &d, &params(64, 48, 10)).unwrap();
    assert!(matches!(
        generate_town(&mut world, &d, &params(64, 48, 10)),
        Err(GenError::BadParams(m)) if m.contains("already has a map")
    ));
}

#[test]
fn a_town_that_cannot_house_everyone_fails_after_its_attempts_with_a_reason() {
    // A tiny, mostly-water map cannot hold fifty residents' homes.
    let mut world = WorldState::new("Crowded", "crowded");
    let err = generate_town(
        &mut world,
        &data(),
        &GenParams {
            water_percent: 50,
            ..params(24, 24, 50)
        },
    )
    .unwrap_err();
    assert!(
        matches!(&err, GenError::Failed { attempts, .. } if *attempts == MAX_ATTEMPTS),
        "{err}"
    );
    assert!(
        world.maps.is_empty() && world.pawns.is_empty(),
        "a failed attempt leaves nothing behind"
    );
}

#[test]
fn buildings_do_not_overlap_face_a_street_and_have_a_walkable_door() {
    let walls = |b: &Building| -> Vec<Tile> {
        (b.y..b.y + b.h)
            .flat_map(|y| (b.x..b.x + b.w).map(move |x| Tile::new(x, y)))
            .collect()
    };
    let around = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    for seed in ["facing-a", "facing-b", "facing-c"] {
        let (world, _) = generate(seed, 64, 48, 18, 14);
        let (map_id, m) = world.maps.iter().next().unwrap();
        for (i, b) in world.town.buildings.iter().enumerate() {
            for t in walls(b) {
                assert!(
                    m.is_blocked(t),
                    "{seed}: footprint tile {t} of building {i}"
                );
            }
            assert!(
                world.is_passable(map_id, b.entrance),
                "{seed}: door {}",
                b.entrance
            );
            assert!(!b.contains(b.entrance));
            let at_wall = around
                .iter()
                .any(|(dx, dy)| b.contains(Tile::new(b.entrance.x + dx, b.entrance.y + dy)));
            assert!(at_wall, "{seed}: building {i} door is not at its wall");
            let on_street = around.iter().any(|(dx, dy)| {
                m.surface_at(Tile::new(b.entrance.x + dx, b.entrance.y + dy)) == Some(SIDEWALK)
            });
            assert!(on_street, "{seed}: building {i} does not face a street");
            for other in world.town.buildings.iter().skip(i + 1) {
                assert!(!b.overlaps(other));
            }
        }
    }
}

#[test]
fn a_generated_world_saves_loads_and_keeps_its_hash_and_defects_are_caught() {
    let (world, _) = generate("round-trip", 64, 48, 18, 14);
    let back = WorldState::from_canon(
        &crate::canon::json::parse(&world.to_canon().to_canonical_string()).unwrap(),
    )
    .unwrap();
    assert_eq!(back.state_hash(), world.state_hash());
    assert_eq!(back.town, world.town);
    assert!(validate_world(&back).is_ok());
    // A resident with nowhere to live is a defect.
    let mut homeless = world.clone();
    if let Some((_, p)) = homeless.pawns.iter_mut().next() {
        p.home_tile = None;
    }
    assert!(validate_world(&homeless).unwrap_err().contains("no home"));
    // So is a plaza nobody can reach, and a building on top of another, and no plaza at all.
    let mut cut_off = world.clone();
    let plaza = cut_off.town.gathering[0];
    let map = cut_off.maps.iter().next().unwrap().0;
    for (dx, dy) in [
        (0, -1),
        (1, 0),
        (0, 1),
        (-1, 0),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ] {
        let n = Tile::new(plaza.x + dx, plaza.y + dy);
        let _ = cut_off.maps.get_mut(map).unwrap().set_blocked(n, true);
    }
    assert!(validate_world(&cut_off).is_err());
    let mut overlapping = world.clone();
    let first = overlapping.town.buildings[0];
    overlapping.town.buildings.push(first);
    assert!(validate_world(&overlapping)
        .unwrap_err()
        .contains("overlaps"));
    let mut bare = world.clone();
    bare.town.gathering.clear();
    assert!(validate_world(&bare).unwrap_err().contains("gathering"));
}

#[test]
fn the_command_generates_through_the_input_log_and_a_second_one_is_refused() {
    use crate::commands::Command;
    use crate::input::SimInput;
    use crate::sim::Sim;
    use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
    let content =
        std::sync::Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
    let cmd = |tone: &str| SimInput::Command {
        actor: None,
        cmd: Command::GenerateTown {
            w: 48,
            h: 36,
            water: 15,
            residents: 10,
            tone: tone.into(),
        },
    };
    let mut sim = Sim::with_dev_systems(WorldState::new("Cmd", "cmd-seed")).with_content(content);
    sim.submit(0, cmd("standard")).unwrap();
    sim.submit(1, cmd("standard")).unwrap();
    sim.submit(2, cmd("loud")).unwrap();
    let mut kinds = Vec::new();
    for _ in 0..4 {
        for e in sim.step().unwrap().events {
            kinds.push((e.kind, e.detail));
        }
    }
    assert_eq!(
        kinds.iter().filter(|(k, _)| k == "town.generated").count(),
        1
    );
    let rejected: Vec<String> = kinds
        .iter()
        .filter(|(k, _)| k == "input_rejected")
        .map(|(_, d)| d.to_canonical_string())
        .collect();
    assert_eq!(rejected.len(), 2, "{rejected:?}");
    assert!(rejected.iter().any(|r| r.contains("already has a map")));
    assert!(rejected.iter().any(|r| r.contains("tone")));
    assert_eq!(sim.world().pawns.len(), 10);
    // The input log records the command.
    let log = crate::replay::ReplayLog::record(&sim);
    assert!(log.inputs.iter().any(|i| i
        .input
        .to_canon()
        .to_canonical_string()
        .contains("generate_town")));
}
