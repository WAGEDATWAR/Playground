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
//! (snapshot/resume equivalence over `ticks`), and `expect` (`pawns`, `commitments`, `tick`, `day`).

use crate::export::{export_world, import_world, ImportOptions};
use crate::store::{LoadError, LoadOptions, Recovery, SlotStore};
use pg_core::canon::json;
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

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    CreateMap { w: i32, h: i32, style: u8 },
    SpawnPawns { count: u32 },
    Command(Command),
    Run { ticks: u64 },
    Save { slot: String },
    Load { slot: String, expect: Option<String> },
    Reload { slot: String },
    Corrupt { slot: String, how: String },
    ExportImport,
    Fork { ticks: u64 },
    Expect(Expectation),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expectation {
    pub pawns: Option<usize>,
    pub commitments: Option<usize>,
    pub tick: Option<u64>,
    pub day: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub world_name: String,
    pub seed: String,
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
                (Some(t), None) => Ok(Step::Run { ticks: t.reader().u64()? }),
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
                return Err(how.reader().err("expected flip, truncate, garbage, delete or delete_manifest"));
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
            r.only(&["op", "pawns", "commitments", "tick", "day"])?;
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
            };
            Ok(Step::Expect(e))
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
            world.reader().only(&["name", "seed"])?;
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
        }
    }
}

fn dev_pipeline() -> Pipeline {
    let mut p = Pipeline::new();
    pg_core::dev::install(&mut p);
    p
}

fn restore(world: WorldState) -> Sim {
    Sim::restore(
        SimSnapshot {
            world,
            pending: Vec::new(),
            next_seq: 0,
        },
        dev_pipeline(),
    )
}

fn command_input(c: Command) -> SimInput {
    SimInput::Command { actor: None, cmd: c }
}

struct Runner {
    sim: Sim,
    mem: MemStorage,
}

impl Runner {
    fn run_step(&mut self, step: &Step) -> Result<String, String> {
        match step {
            Step::CreateMap { w, h, style } => {
                self.sim
                    .submit_now(command_input(Command::DevCreateMap { w: *w, h: *h, style: *style }))
                    .map_err(|e| e.to_string())?;
                self.sim.run_ticks(1).map_err(|e| e.to_string())?;
                Ok(format!("map {w}x{h}"))
            }
            Step::SpawnPawns { count } => {
                let map = EntityId::new(Kind::Map, 1);
                for i in 0..*count {
                    self.sim
                        .submit_now(command_input(Command::DevSpawnPawn { map, at: None, name: format!("P{i}") }))
                        .map_err(|e| e.to_string())?;
                }
                self.sim.run_ticks(1).map_err(|e| e.to_string())?;
                Ok(format!("{} pawn(s) now", self.sim.world().pawns.len()))
            }
            Step::Command(c) => {
                self.sim.submit_now(command_input(c.clone())).map_err(|e| e.to_string())?;
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
                let plan = import_world(&bytes, &ImportOptions::default()).map_err(|e| e.to_string())?;
                if plan.world.state_hash() != w.state_hash() {
                    return Err("the imported world differs from the exported one".to_owned());
                }
                Ok(format!("{} bytes round-tripped", bytes.len()))
            }
            Step::Fork { ticks } => {
                let snap = self.sim.snapshot();
                let mut resumed = Sim::restore(snap, dev_pipeline());
                resumed.run_ticks(*ticks).map_err(|e| e.to_string())?;
                self.sim.run_ticks(*ticks).map_err(|e| e.to_string())?;
                if resumed.world().state_hash() != self.sim.world().state_hash() {
                    return Err("snapshot/resume diverged from the uninterrupted run".to_owned());
                }
                Ok(format!("{ticks} ticks identical after snapshot/resume"))
            }
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
                check("commitments", e.commitments.map(|v| v as u64), w.commitments.len() as u64);
                check("tick", e.tick, w.clock.tick());
                check("day", e.day, w.clock.day());
                if problems.is_empty() {
                    Ok("as expected".to_owned())
                } else {
                    Err(problems.join("; "))
                }
            }
        }
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
        Ok(format!("generation {} ({} -> {} bytes)", r.generation, r.uncompressed_bytes, r.compressed_bytes))
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
                self.sim = restore(loaded.world);
                Ok(format!("loaded generation {gen} ({got})"))
            }
            Err(_) => Ok(format!("refused as expected ('{got}')")),
        }
    }

    fn corrupt(&mut self, slot: &str, how: &str) -> Result<String, String> {
        let id = Self::slot_id(slot);
        let store = SlotStore::new(&self.mem);
        let manifest = store.manifest(&id).ok_or("the slot has no manifest to find its newest generation")?;
        let newest = manifest.current().map(|g| g.generation).ok_or("no generations")?;
        let state = format!("worlds/{id}/state.{newest}.pgsave");
        let manifest_name = format!("worlds/{id}/manifest.json");
        let bytes = self.mem.read(&state).map_err(|e| e.to_string())?.ok_or("the newest generation file is missing")?;
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

/// Runs a scenario against a fresh world and an in-memory store. Stops at the first failing step.
pub fn run(scenario: &Scenario) -> ScenarioReport {
    let mut runner = Runner {
        sim: Sim::with_dev_systems(WorldState::new(scenario.world_name.clone(), scenario.seed.clone())),
        mem: MemStorage::new(),
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

    const HEAD: &str = r#""format":"playground-scenario","version":1,"name":"t","world":{"name":"T","seed":"s"}"#;

    fn scenario(steps: &str) -> String {
        format!(r#"{{{HEAD},"steps":[{steps}]}}"#)
    }

    const BUILD: &str = r#"{"op":"create_map","w":24,"h":18,"style":1},{"op":"spawn_pawns","count":5},"#;

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
        let text = scenario(&format!(r#"{BUILD}{{"op":"expect","pawns":99}},{{"op":"run","ticks":10}}"#));
        let r = run_text(&text).unwrap();
        assert!(!r.ok());
        assert_eq!(r.outcomes.len(), 3, "the step after the failure did not run");
        let last = r.outcomes.last().unwrap();
        assert!(!last.ok && last.message.contains("expected 99, found 5"), "{last:?}");
        // A wrong expectation about a load is a failure too.
        let t = scenario(&format!(r#"{BUILD}{{"op":"save","slot":"a"}},{{"op":"load","slot":"a","expect":"damaged"}}"#));
        assert!(!run_text(&t).unwrap().ok());
    }

    #[test]
    fn bad_scenario_files_are_refused_with_paths() {
        for (text, needle) in [
            ("{}", "missing field"),
            ("not json", ""),
            (&scenario(r#"{"op":"dance"}"#), "unknown step 'dance'"),
            (&scenario(r#"{"op":"run"}"#), "exactly one of"),
            (&scenario(r#"{"op":"run","ticks":1,"days":1}"#), "exactly one of"),
            (&scenario(r#"{"op":"corrupt","slot":"a","how":"melt"}"#), "expected flip"),
            (&scenario(r#"{"op":"save"}"#), "missing field 'slot'"),
            (&scenario(r#"{"op":"command","cmd":{"type":"nope"}}"#), "unknown command"),
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
}
