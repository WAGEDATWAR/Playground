//! Scenario files (suggestion S-003): executable acceptance checks.
//!
//! A scenario is a small JSON document: a world identity and a list of steps. Steps build a town, run it,
//! save it, reload it, damage the save, load it again and assert what must be true. "Done when" lines from
//! the Roadmap become files that CI runs with `pg scenario run`, instead of sentences someone has to check
//! by hand.
//!
//! ```json
//! { "format": "playground-scenario", "version": 1, "name": "save-reload",
//!   "world": { "name": "Town", "seed": "s1" },
//!   "steps": [
//!     { "op": "create_map", "w": 32, "h": 24, "style": 1 },
//!     { "op": "spawn_pawns", "count": 6 },
//!     { "op": "run", "days": 1 },
//!     { "op": "reload", "slot": "a" },
//!     { "op": "corrupt", "slot": "a", "how": "flip" },
//!     { "op": "load", "slot": "a", "expect": "clean" } ] }
//! ```
//!
//! Steps: `create_map`, `spawn_pawns`, `command`, `run` (`ticks` or `days`), `save`, `load` (optional
//! `expect`: `clean`, `fell_back`, `no_manifest`, `damaged`, `newer`), `reload` (save, load, hashes must
//! match), `corrupt` (`flip`, `truncate`, `garbage`, `delete`, `delete_manifest`), `export_import`, `fork`
//! (snapshot/resume equivalence over `ticks`), `expect` (`pawns`, `commitments`, `tick`, `day`, `hash`),
//! `migrate` (load a pinned old save through the migration chain, check its recorded hash and carry on from
//! it) and `soak` (run N days with invariants, optionally re-simulating every day on another thread count:
//! shadow verification). The world section may name content `packs` (folders) for the environment to load.
//!
//! What a step needs from the machine (which pipeline to build, how to read a fixture, what the scripts
//! cost) comes from a [`ScenarioEnv`]; the default [`DevEnv`] is the plain dev pipeline, and the runtime
//! supplies one with content, scripts and thread counts.

use crate::export::{export_world, import_world, ImportOptions};
use crate::migrate::Migrations;
use crate::store::{LoadError, LoadOptions, Recovery, SlotStore};
use pg_core::canon::{json, ToCanon};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::pipeline::Pipeline;
use pg_core::read::{ReadError, Reader, Root};
use pg_core::sim::{Sim, SimSnapshot};
use pg_core::time::TICKS_PER_DAY;
use pg_core::world::WorldState;
use pg_host::{MemStorage, Storage};

pub const SCENARIO_FORMAT: &str = "playground-scenario";

/// What a scenario step needs from the machine around it.
pub trait ScenarioEnv {
    /// A sim resumed from `snapshot`, running its path batches on `threads` threads (so a run on a different
    /// thread count is a shadow verification).
    fn restore(&self, snapshot: SimSnapshot, threads: usize) -> Sim;

    /// A sim for a brand-new world.
    fn new_sim(&self, world: WorldState) -> Sim {
        self.restore(
            SimSnapshot {
                world,
                pending: Vec::new(),
                next_seq: 0,
            },
            1,
        )
    }

    /// The bytes of a file named in a scenario (a pinned fixture).
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        Err(format!("this environment cannot read files ('{path}')"))
    }

    /// Total script fuel used so far, if scripts run; a soak checks that its daily cost stays flat.
    fn script_fuel(&self) -> Option<u64> {
        None
    }
}

/// The plain dev pipeline with no content and no scripts.
pub struct DevEnv;

impl ScenarioEnv for DevEnv {
    fn restore(&self, snapshot: SimSnapshot, _threads: usize) -> Sim {
        Sim::restore(snapshot, dev_pipeline())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    CreateMap {
        w: i32,
        h: i32,
        style: u8,
    },
    SpawnPawns {
        count: u32,
    },
    Command(Command),
    Run {
        ticks: u64,
    },
    Save {
        slot: String,
    },
    Load {
        slot: String,
        expect: Option<String>,
    },
    Reload {
        slot: String,
    },
    Corrupt {
        slot: String,
        how: String,
    },
    ExportImport,
    Fork {
        ticks: u64,
    },
    Expect(Expectation),
    /// Loads a pinned save from `fixture` through the migrations, checks the hash recorded in `hash_file`
    /// and continues from that world.
    Migrate {
        fixture: String,
        hash_file: String,
    },
    /// Runs `days` days. With `shadow_threads`, every day is run a second time from the same snapshot on
    /// that many threads and the hashes must agree. The state may not grow more than
    /// `max_growth_permille` per mille over its size on day 5, and neither may the daily script fuel.
    Soak {
        days: u64,
        shadow_threads: Option<usize>,
        max_growth_permille: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expectation {
    pub pawns: Option<usize>,
    pub commitments: Option<usize>,
    pub tick: Option<u64>,
    pub day: Option<u64>,
    /// A prefix of the state hash (8 to 64 hex characters): pins the exact state across operating systems.
    pub hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub world_name: String,
    pub seed: String,
    /// Content pack folders the environment should load (relative to where the scenario is run).
    pub packs: Vec<String>,
    pub steps: Vec<Step>,
}

fn parse_step(r: Reader<'_>) -> Result<Step, ReadError> {
    let op = r.child("op")?;
    let op_name = op.reader().str()?;
    let slot = |r: &Reader<'_>| -> Result<String, ReadError> {
        Ok(r.child("slot")?.reader().str()?.to_owned())
    };
    match op_name {
        "create_map" => {
            r.only(&["op", "w", "h", "style"])?;
            Ok(Step::CreateMap {
                w: r.child("w")?.reader().i32()?,
                h: r.child("h")?.reader().i32()?,
                style: match r.maybe("style")? {
                    Some(s) => s.reader().u8()?,
                    None => 1,
                },
            })
        }
        "spawn_pawns" => {
            r.only(&["op", "count"])?;
            Ok(Step::SpawnPawns {
                count: r.child("count")?.reader().u32()?,
            })
        }
        "command" => {
            r.only(&["op", "cmd"])?;
            let c = r.child("cmd")?;
            Ok(Step::Command(
                Command::from_canon(c.reader().value()).map_err(|e| c.reader().err(e.0))?,
            ))
        }
        "run" => {
            r.only(&["op", "ticks", "days"])?;
            match (r.maybe("ticks")?, r.maybe("days")?) {
                (Some(t), None) => Ok(Step::Run {
                    ticks: t.reader().u64()?,
                }),
                (None, Some(d)) => Ok(Step::Run {
                    ticks: d
                        .reader()
                        .u64()?
                        .checked_mul(TICKS_PER_DAY)
                        .ok_or_else(|| d.reader().err("too many days"))?,
                }),
                _ => Err(r.err("run needs exactly one of 'ticks' or 'days'")),
            }
        }
        "save" => {
            r.only(&["op", "slot"])?;
            Ok(Step::Save { slot: slot(&r)? })
        }
        "load" => {
            r.only(&["op", "slot", "expect"])?;
            Ok(Step::Load {
                slot: slot(&r)?,
                expect: match r.maybe("expect")? {
                    Some(e) => Some(e.reader().str()?.to_owned()),
                    None => None,
                },
            })
        }
        "reload" => {
            r.only(&["op", "slot"])?;
            Ok(Step::Reload { slot: slot(&r)? })
        }
        "corrupt" => {
            r.only(&["op", "slot", "how"])?;
            let how = r.child("how")?;
            let h = how.reader().str()?;
            if !["flip", "truncate", "garbage", "delete", "delete_manifest"].contains(&h) {
                return Err(how
                    .reader()
                    .err("expected flip, truncate, garbage, delete or delete_manifest"));
            }
            Ok(Step::Corrupt {
                slot: slot(&r)?,
                how: h.to_owned(),
            })
        }
        "export_import" => {
            r.only(&["op"])?;
            Ok(Step::ExportImport)
        }
        "fork" => {
            r.only(&["op", "ticks"])?;
            Ok(Step::Fork {
                ticks: r.child("ticks")?.reader().u64()?,
            })
        }
        "expect" => {
            r.only(&["op", "pawns", "commitments", "tick", "day", "hash"])?;
            let e = Expectation {
                pawns: match r.maybe("pawns")? {
                    Some(v) => Some(v.reader().usize()?),
                    None => None,
                },
                commitments: match r.maybe("commitments")? {
                    Some(v) => Some(v.reader().usize()?),
                    None => None,
                },
                tick: match r.maybe("tick")? {
                    Some(v) => Some(v.reader().u64()?),
                    None => None,
                },
                day: match r.maybe("day")? {
                    Some(v) => Some(v.reader().u64()?),
                    None => None,
                },
                hash: match r.maybe("hash")? {
                    Some(v) => {
                        let h = v.reader().str()?;
                        if !(8..=64).contains(&h.len()) || !h.bytes().all(|b| b.is_ascii_hexdigit())
                        {
                            return Err(v.reader().err("expected 8 to 64 hex characters"));
                        }
                        Some(h.to_ascii_lowercase())
                    }
                    None => None,
                },
            };
            Ok(Step::Expect(e))
        }
        "migrate" => {
            r.only(&["op", "fixture", "hash_file"])?;
            Ok(Step::Migrate {
                fixture: r.child("fixture")?.reader().str()?.to_owned(),
                hash_file: r.child("hash_file")?.reader().str()?.to_owned(),
            })
        }
        "soak" => {
            r.only(&["op", "days", "shadow_threads", "max_growth_permille"])?;
            Ok(Step::Soak {
                days: r.child("days")?.reader().u64()?,
                shadow_threads: match r.maybe("shadow_threads")? {
                    Some(t) => Some(t.reader().usize()?),
                    None => None,
                },
                max_growth_permille: match r.maybe("max_growth_permille")? {
                    Some(g) => g.reader().u64()?,
                    None => 1500,
                },
            })
        }
        other => Err(op.reader().err(format!("unknown step '{other}'"))),
    }
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Scenario, String> {
        let doc = json::parse(text).map_err(|e| e.to_string())?;
        let root = Root::new(doc);
        let r = root.reader();
        let go = || -> Result<Scenario, ReadError> {
            r.only(&["format", "version", "name", "description", "world", "steps"])?;
            if r.child("format")?.reader().str()? != SCENARIO_FORMAT {
                return Err(r.err("not a playground scenario"));
            }
            if r.child("version")?.reader().u32()? != 1 {
                return Err(r.err("unsupported scenario version"));
            }
            let world = r.child("world")?;
            world.reader().only(&["name", "seed", "packs"])?;
            let mut packs = Vec::new();
            if let Some(list) = world.reader().maybe("packs")? {
                for p in list.reader().list()? {
                    packs.push(p.reader().str()?.to_owned());
                }
            }
            let steps = r
                .child("steps")?
                .reader()
                .list()?
                .iter()
                .map(|s| parse_step(s.reader()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Scenario {
                name: r.child("name")?.reader().str()?.to_owned(),
                description: match r.maybe("description")? {
                    Some(d) => d.reader().str()?.to_owned(),
                    None => String::new(),
                },
                world_name: world.reader().child("name")?.reader().str()?.to_owned(),
                seed: world.reader().child("seed")?.reader().str()?.to_owned(),
                packs,
                steps,
            })
        };
        go().map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOutcome {
    pub index: usize,
    pub op: &'static str,
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioReport {
    pub name: String,
    pub outcomes: Vec<StepOutcome>,
    /// The final state hash, so two runs of one scenario can be compared.
    pub final_hash: String,
}

impl ScenarioReport {
    pub fn ok(&self) -> bool {
        self.outcomes.iter().all(|o| o.ok)
    }
}

impl Step {
    fn op(&self) -> &'static str {
        match self {
            Step::CreateMap { .. } => "create_map",
            Step::SpawnPawns { .. } => "spawn_pawns",
            Step::Command(_) => "command",
            Step::Run { .. } => "run",
            Step::Save { .. } => "save",
            Step::Load { .. } => "load",
            Step::Reload { .. } => "reload",
            Step::Corrupt { .. } => "corrupt",
            Step::ExportImport => "export_import",
            Step::Fork { .. } => "fork",
            Step::Expect(_) => "expect",
            Step::Migrate { .. } => "migrate",
            Step::Soak { .. } => "soak",
        }
    }
}

fn dev_pipeline() -> Pipeline {
    let mut p = Pipeline::new();
    pg_core::dev::install(&mut p);
    p
}

fn command_input(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

struct Runner<'a> {
    sim: Sim,
    mem: MemStorage,
    env: &'a dyn ScenarioEnv,
}

impl Runner<'_> {
    fn run_step(&mut self, step: &Step) -> Result<String, String> {
        match step {
            Step::CreateMap { w, h, style } => {
                self.sim
                    .submit_now(command_input(Command::DevCreateMap {
                        w: *w,
                        h: *h,
                        style: *style,
                    }))
                    .map_err(|e| e.to_string())?;
                self.sim.run_ticks(1).map_err(|e| e.to_string())?;
                Ok(format!("map {w}x{h}"))
            }
            Step::SpawnPawns { count } => {
                let map = EntityId::new(Kind::Map, 1);
                for i in 0..*count {
                    self.sim
                        .submit_now(command_input(Command::DevSpawnPawn {
                            map,
                            at: None,
                            name: format!("P{i}"),
                        }))
                        .map_err(|e| e.to_string())?;
                }
                self.sim.run_ticks(1).map_err(|e| e.to_string())?;
                Ok(format!("{} pawn(s) now", self.sim.world().pawns.len()))
            }
            Step::Command(c) => {
                self.sim
                    .submit_now(command_input(c.clone()))
                    .map_err(|e| e.to_string())?;
                self.sim.run_ticks(1).map_err(|e| e.to_string())?;
                Ok("applied".to_owned())
            }
            Step::Run { ticks } => {
                self.sim.run_ticks(*ticks).map_err(|e| e.to_string())?;
                Ok(format!("tick {}", self.sim.world().clock.tick()))
            }
            Step::Save { slot } => {
                let r = self.save(slot)?;
                Ok(r)
            }
            Step::Load { slot, expect } => self.load(slot, expect.as_deref()),
            Step::Reload { slot } => {
                let before = self.sim.world().state_hash();
                self.save(slot)?;
                let msg = self.load(slot, Some("clean"))?;
                let after = self.sim.world().state_hash();
                if before != after {
                    return Err(format!("the reloaded world differs ({before} vs {after})"));
                }
                Ok(format!("{msg}; hash {}", &before.to_hex()[..8]))
            }
            Step::Corrupt { slot, how } => self.corrupt(slot, how),
            Step::ExportImport => {
                let w = self.sim.world();
                let bytes = export_world(w, &[], "scenario", "");
                let plan =
                    import_world(&bytes, &ImportOptions::default()).map_err(|e| e.to_string())?;
                if plan.world.state_hash() != w.state_hash() {
                    return Err("the imported world differs from the exported one".to_owned());
                }
                Ok(format!("{} bytes round-tripped", bytes.len()))
            }
            Step::Fork { ticks } => {
                let snap = self.sim.snapshot();
                let mut resumed = self.env.restore(snap, 1);
                resumed.run_ticks(*ticks).map_err(|e| e.to_string())?;
                self.sim.run_ticks(*ticks).map_err(|e| e.to_string())?;
                if resumed.world().state_hash() != self.sim.world().state_hash() {
                    return Err("snapshot/resume diverged from the uninterrupted run".to_owned());
                }
                Ok(format!("{ticks} ticks identical after snapshot/resume"))
            }
            Step::Migrate { fixture, hash_file } => self.migrate(fixture, hash_file),
            Step::Soak {
                days,
                shadow_threads,
                max_growth_permille,
            } => self.soak(*days, *shadow_threads, *max_growth_permille),
            Step::Expect(e) => {
                let w = self.sim.world();
                let mut problems = Vec::new();
                let mut check = |name: &str, want: Option<u64>, got: u64| {
                    if let Some(want) = want {
                        if want != got {
                            problems.push(format!("{name}: expected {want}, found {got}"));
                        }
                    }
                };
                check("pawns", e.pawns.map(|v| v as u64), w.pawns.len() as u64);
                check(
                    "commitments",
                    e.commitments.map(|v| v as u64),
                    w.commitments.len() as u64,
                );
                check("tick", e.tick, w.clock.tick());
                check("day", e.day, w.clock.day());
                if let Some(want) = &e.hash {
                    let got = w.state_hash().to_hex();
                    if !got.starts_with(want.as_str()) {
                        problems.push(format!(
                            "state hash: expected {want}..., found {}",
                            &got[..want.len().min(got.len())]
                        ));
                    }
                }
                if problems.is_empty() {
                    Ok("as expected".to_owned())
                } else {
                    Err(problems.join("; "))
                }
            }
        }
    }

    fn migrate(&mut self, fixture: &str, hash_file: &str) -> Result<String, String> {
        let text = String::from_utf8(self.env.read_file(fixture)?)
            .map_err(|_| format!("{fixture} is not text"))?;
        let want = String::from_utf8(self.env.read_file(hash_file)?)
            .map_err(|_| format!("{hash_file} is not text"))?;
        let raw = json::parse(&text).map_err(|e| format!("{fixture}: {e}"))?;
        let schema = raw
            .get("schema")
            .and_then(|s| s.as_i64())
            .and_then(|s| u32::try_from(s).ok())
            .ok_or_else(|| format!("{fixture} has no schema number"))?;
        let migrations = Migrations::builtin();
        let migrated = migrations
            .migrate(raw, schema)
            .map_err(|e| format!("{fixture}: {e}"))?;
        let world = WorldState::from_canon(&migrated).map_err(|e| format!("{fixture}: {e}"))?;
        let got = world.state_hash().to_hex();
        if got != want.trim() {
            return Err(format!(
                "the migrated world hashes to {} but {hash_file} records {}",
                &got[..8],
                want.trim().chars().take(8).collect::<String>()
            ));
        }
        let (pawns, tick) = (world.pawns.len(), world.clock.tick());
        self.sim = self.env.new_sim(world);
        Ok(format!(
            "schema {schema} -> {} migrated; {pawns} pawn(s) at tick {tick}; hash {}",
            migrations.current(),
            &got[..8]
        ))
    }

    fn soak(
        &mut self,
        days: u64,
        shadow_threads: Option<usize>,
        max_growth_permille: u64,
    ) -> Result<String, String> {
        let pawns = self.sim.world().pawns.len();
        let inputs_before = self.sim.applied_inputs().len();
        let mut sizes: Vec<u64> = Vec::new();
        let mut fuel: Vec<u64> = Vec::new();
        for day in 1..=days {
            let snap = self.sim.snapshot();
            let fuel_before = self.env.script_fuel();
            self.sim
                .run_ticks(TICKS_PER_DAY)
                .map_err(|e| e.to_string())?;
            if let (Some(a), Some(b)) = (fuel_before, self.env.script_fuel()) {
                fuel.push(b.saturating_sub(a));
            }
            if let Some(threads) = shadow_threads {
                let mut other = self.env.restore(snap, threads);
                other.run_ticks(TICKS_PER_DAY).map_err(|e| e.to_string())?;
                if other.world().state_hash() != self.sim.world().state_hash() {
                    return Err(format!(
                        "shadow verification diverged on day {day} ({threads} thread(s) disagree with the main run)"
                    ));
                }
            }
            sizes.push(self.sim.world().to_canon().to_canonical_string().len() as u64);
        }
        let mut notes = vec![format!("{days} day(s)")];
        // Bounded state: after the first days settle, the world may not keep growing.
        if sizes.len() > 5 {
            let base = sizes.get(4).copied().unwrap_or(1).max(1);
            let worst = sizes.iter().skip(5).copied().max().unwrap_or(base);
            if worst * 1000 > base * max_growth_permille {
                return Err(format!(
                    "the world state grew from {base} to {worst} bytes (more than {max_growth_permille} per mille of day 5)"
                ));
            }
            notes.push(format!("state {base} -> {worst} bytes"));
        }
        if fuel.len() > 5 {
            let base = fuel.get(4).copied().unwrap_or(1).max(1);
            let worst = fuel.iter().skip(5).copied().max().unwrap_or(base);
            if worst * 1000 > base * max_growth_permille {
                return Err(format!(
                    "script fuel per day grew from {base} to {worst} (more than {max_growth_permille} per mille of day 5)"
                ));
            }
            notes.push(format!("script fuel per day {base} -> {worst}"));
        }
        if self.sim.world().pawns.len() != pawns {
            return Err(format!(
                "the population changed during the soak ({pawns} -> {})",
                self.sim.world().pawns.len()
            ));
        }
        let inputs_after = self.sim.applied_inputs().len();
        if inputs_after != inputs_before {
            return Err(format!(
                "the input log grew by {} during a run with no inputs",
                inputs_after.saturating_sub(inputs_before)
            ));
        }
        if let Some(t) = shadow_threads {
            notes.push(format!(
                "every day re-simulated on {t} thread(s): identical"
            ));
        }
        Ok(notes.join("; "))
    }

    fn slot_id(slot: &str) -> String {
        slot.to_owned()
    }

    fn save(&mut self, slot: &str) -> Result<String, String> {
        if !self.sim.snapshot().pending.is_empty() {
            return Err("cannot save while inputs are queued for future ticks".to_owned());
        }
        let store = SlotStore::new(&self.mem);
        let r = store
            .save(&Self::slot_id(slot), self.sim.world(), &[], "scenario")
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "generation {} ({} -> {} bytes)",
            r.generation, r.uncompressed_bytes, r.compressed_bytes
        ))
    }

    fn load(&mut self, slot: &str, expect: Option<&str>) -> Result<String, String> {
        let store = SlotStore::new(&self.mem);
        let result = store.load(&Self::slot_id(slot), &LoadOptions::default());
        let got = match &result {
            Ok(l) => match l.recovery {
                Recovery::Clean => "clean",
                Recovery::FellBack { .. } => "fell_back",
                Recovery::NoManifest { .. } => "no_manifest",
            },
            Err(LoadError::Damaged(_)) => "damaged",
            Err(LoadError::Newer(_)) => "newer",
            Err(LoadError::NotFound) => "not_found",
            Err(LoadError::Io(_)) => "io_error",
        };
        if let Some(want) = expect {
            if want != got {
                return Err(format!("expected the load to be '{want}', it was '{got}'"));
            }
        } else if result.is_err() {
            return Err(format!("load failed ('{got}')"));
        }
        match result {
            Ok(loaded) => {
                let gen = loaded.generation;
                self.sim = self.env.new_sim(loaded.world);
                Ok(format!("loaded generation {gen} ({got})"))
            }
            Err(_) => Ok(format!("refused as expected ('{got}')")),
        }
    }

    fn corrupt(&mut self, slot: &str, how: &str) -> Result<String, String> {
        let id = Self::slot_id(slot);
        let store = SlotStore::new(&self.mem);
        let manifest = store
            .manifest(&id)
            .ok_or("the slot has no manifest to find its newest generation")?;
        let newest = manifest
            .current()
            .map(|g| g.generation)
            .ok_or("no generations")?;
        let state = format!("worlds/{id}/state.{newest}.pgsave");
        let manifest_name = format!("worlds/{id}/manifest.json");
        let bytes = self
            .mem
            .read(&state)
            .map_err(|e| e.to_string())?
            .ok_or("the newest generation file is missing")?;
        match how {
            "flip" => {
                let mut b = bytes;
                let i = b.len() / 2;
                if let Some(x) = b.get_mut(i) {
                    *x ^= 0xFF;
                }
                self.mem.put_raw(&state, b);
            }
            "truncate" => self.mem.put_raw(&state, bytes[..bytes.len() / 3].to_vec()),
            "garbage" => self.mem.put_raw(&state, vec![0xAB; 256]),
            "delete" => self.mem.delete(&state).map_err(|e| e.to_string())?,
            _ => self.mem.delete(&manifest_name).map_err(|e| e.to_string())?,
        }
        Ok(format!("{how} applied to generation {newest}"))
    }
}

/// Runs a scenario against a fresh world and an in-memory store, in the plain dev environment.
pub fn run(scenario: &Scenario) -> ScenarioReport {
    run_with(scenario, &DevEnv)
}

/// Runs a scenario in `env`. Stops at the first failing step.
pub fn run_with(scenario: &Scenario, env: &dyn ScenarioEnv) -> ScenarioReport {
    let mut runner = Runner {
        sim: env.new_sim(WorldState::new(
            scenario.world_name.clone(),
            scenario.seed.clone(),
        )),
        mem: MemStorage::new(),
        env,
    };
    let mut outcomes = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        let (ok, message) = match runner.run_step(step) {
            Ok(m) => (true, m),
            Err(m) => (false, m),
        };
        outcomes.push(StepOutcome {
            index,
            op: step.op(),
            ok,
            message,
        });
        if !ok {
            break;
        }
    }
    ScenarioReport {
        name: scenario.name.clone(),
        outcomes,
        final_hash: runner.sim.world().state_hash().to_hex(),
    }
}

/// Convenience for tests and the CLI: parse then run.
pub fn run_text(text: &str) -> Result<ScenarioReport, String> {
    Ok(run(&Scenario::parse(text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str =
        r#""format":"playground-scenario","version":1,"name":"t","world":{"name":"T","seed":"s"}"#;

    fn scenario(steps: &str) -> String {
        format!(r#"{{{HEAD},"steps":[{steps}]}}"#)
    }

    const BUILD: &str =
        r#"{"op":"create_map","w":24,"h":18,"style":1},{"op":"spawn_pawns","count":5},"#;

    #[test]
    fn a_save_reload_scenario_passes_and_is_deterministic() {
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"run","days":1}},{{"op":"reload","slot":"a"}},{{"op":"run","ticks":3000}},
               {{"op":"fork","ticks":2000}},{{"op":"export_import"}},{{"op":"expect","pawns":5,"day":1}}"#
        ));
        let a = run_text(&text).unwrap();
        assert!(a.ok(), "{:?}", a.outcomes);
        assert_eq!(a.outcomes.len(), 8);
        let b = run_text(&text).unwrap();
        assert_eq!(a.final_hash, b.final_hash);
    }

    #[test]
    fn corruption_and_recovery_are_expressible() {
        // After a fall-back the world is the older save (tick 2002: two ticks building, then 2000 run);
        // with only the manifest gone, the newest generation is still valid (tick 4002).
        for (how, expect, tick) in [
            ("flip", "fell_back", 2002),
            ("truncate", "fell_back", 2002),
            ("garbage", "fell_back", 2002),
            ("delete", "fell_back", 2002),
            ("delete_manifest", "no_manifest", 4002),
        ] {
            let text = scenario(&format!(
                r#"{BUILD}{{"op":"run","ticks":2000}},{{"op":"save","slot":"a"}},{{"op":"run","ticks":2000}},
                   {{"op":"save","slot":"a"}},{{"op":"corrupt","slot":"a","how":"{how}"}},
                   {{"op":"load","slot":"a","expect":"{expect}"}},{{"op":"expect","tick":{tick}}}"#
            ));
            let r = run_text(&text).unwrap();
            assert!(r.ok(), "{how}: {:?}", r.outcomes);
        }
    }

    #[test]
    fn total_loss_is_reported_as_damaged() {
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"save","slot":"a"}},{{"op":"corrupt","slot":"a","how":"garbage"}},
               {{"op":"load","slot":"a","expect":"damaged"}}"#
        ));
        let r = run_text(&text).unwrap();
        assert!(r.ok(), "{:?}", r.outcomes);
    }

    #[test]
    fn a_failing_expectation_stops_the_run_with_a_reason() {
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"expect","pawns":99}},{{"op":"run","ticks":10}}"#
        ));
        let r = run_text(&text).unwrap();
        assert!(!r.ok());
        assert_eq!(
            r.outcomes.len(),
            3,
            "the step after the failure did not run"
        );
        let last = r.outcomes.last().unwrap();
        assert!(
            !last.ok && last.message.contains("expected 99, found 5"),
            "{last:?}"
        );
        // A wrong expectation about a load is a failure too.
        let t = scenario(&format!(
            r#"{BUILD}{{"op":"save","slot":"a"}},{{"op":"load","slot":"a","expect":"damaged"}}"#
        ));
        assert!(!run_text(&t).unwrap().ok());
    }

    #[test]
    fn bad_scenario_files_are_refused_with_paths() {
        for (text, needle) in [
            ("{}", "missing field"),
            ("not json", ""),
            (&scenario(r#"{"op":"dance"}"#), "unknown step 'dance'"),
            (&scenario(r#"{"op":"run"}"#), "exactly one of"),
            (
                &scenario(r#"{"op":"run","ticks":1,"days":1}"#),
                "exactly one of",
            ),
            (
                &scenario(r#"{"op":"corrupt","slot":"a","how":"melt"}"#),
                "expected flip",
            ),
            (&scenario(r#"{"op":"save"}"#), "missing field 'slot'"),
            (
                &scenario(r#"{"op":"command","cmd":{"type":"nope"}}"#),
                "unknown command",
            ),
            (&scenario(r#"{"op":"expect","pawns":-1}"#), "non-negative"),
        ] {
            let e = Scenario::parse(text).unwrap_err();
            assert!(e.contains(needle), "{text} -> {e}");
        }
    }

    #[test]
    fn commands_in_scenarios_are_applied() {
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"command","cmd":{{"type":"dev_nudge","amount":42}}}},{{"op":"expect","pawns":5}}"#
        ));
        assert!(run_text(&text).unwrap().ok());
    }

    // ---- environment-dependent steps ------------------------------------------------------------------

    /// Reads fixtures from the repository, like the CLI does.
    struct FileEnv;

    impl ScenarioEnv for FileEnv {
        fn restore(&self, snapshot: SimSnapshot, _threads: usize) -> Sim {
            Sim::restore(snapshot, dev_pipeline())
        }

        fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
            std::fs::read(format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR")))
                .map_err(|e| format!("{path}: {e}"))
        }
    }

    #[test]
    fn a_pinned_old_save_is_migrated_checked_and_carried_on() {
        let text = scenario(
            r#"{"op":"migrate","fixture":"fixtures/saves/world-v3.json","hash_file":"fixtures/saves/world-v3.hash"},
               {"op":"run","days":1},{"op":"reload","slot":"a"},{"op":"expect","pawns":4}"#,
        );
        let sc = Scenario::parse(&text).unwrap();
        let r = run_with(&sc, &FileEnv);
        assert!(r.ok(), "{:?}", r.outcomes);
        assert!(
            r.outcomes[0].message.contains("migrated"),
            "{}",
            r.outcomes[0].message
        );
        // The default environment cannot read files and says so.
        let r = run(&sc);
        assert!(
            !r.ok() && r.outcomes[0].message.contains("cannot read files"),
            "{:?}",
            r.outcomes
        );
        // A wrong recorded hash is a failure that names both.
        let bad = scenario(
            r#"{"op":"migrate","fixture":"fixtures/saves/world-v3.json","hash_file":"fixtures/saves/world-v3.json"}"#,
        );
        let r = run_with(&Scenario::parse(&bad).unwrap(), &FileEnv);
        assert!(!r.ok());
    }

    #[test]
    fn a_soak_checks_shadow_verification_bounds_and_the_input_log() {
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"soak","days":8,"shadow_threads":2}},{{"op":"expect","day":8,"pawns":5}}"#
        ));
        let r = run_text(&text).unwrap();
        assert!(r.ok(), "{:?}", r.outcomes);
        assert!(
            r.outcomes[2].message.contains("identical"),
            "{}",
            r.outcomes[2].message
        );
        // Parsing: the defaults and the strictness.
        assert!(Scenario::parse(&scenario(r#"{"op":"soak","days":3,"nope":1}"#)).is_err());
    }

    #[test]
    fn a_soak_catches_a_system_that_depends_on_something_outside_the_world() {
        use pg_core::pipeline::{Cadence, Placement, System, SystemSlot, TickCtx};
        use std::sync::atomic::{AtomicU64, Ordering};
        static OUTSIDE: AtomicU64 = AtomicU64::new(0);
        struct Leaky;
        impl System for Leaky {
            fn id(&self) -> &str {
                "test.leaky"
            }
            fn run(&mut self, ctx: &mut TickCtx<'_>) {
                ctx.world.probe.value +=
                    i64::try_from(OUTSIDE.fetch_add(1, Ordering::SeqCst) % 7).unwrap_or(0);
            }
        }
        struct LeakyEnv;
        impl ScenarioEnv for LeakyEnv {
            fn restore(&self, snapshot: SimSnapshot, _t: usize) -> Sim {
                let mut p = dev_pipeline();
                let _ = p.add_extension(
                    SystemSlot::Maintenance,
                    Placement::After,
                    Cadence::Tick,
                    Box::new(Leaky),
                );
                Sim::restore(snapshot, p)
            }
        }
        let text = scenario(&format!(
            r#"{BUILD}{{"op":"soak","days":2,"shadow_threads":2}}"#
        ));
        let r = run_with(&Scenario::parse(&text).unwrap(), &LeakyEnv);
        assert!(!r.ok());
        assert!(
            r.outcomes[2]
                .message
                .contains("shadow verification diverged"),
            "{}",
            r.outcomes[2].message
        );
    }
}
