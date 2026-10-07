//! The scenario environment with the whole runtime behind it (milestone 0.11).
//!
//! `pg-persist`'s scenario runner only needs a way to build and resume sims; this one builds them with the
//! loaded content and its scripts, runs path batches on the requested number of threads (a different
//! number is what makes a re-run a shadow verification), reads fixtures from the repository, and meters
//! script fuel so a soak can check that the daily cost stays flat.

use crate::keyframes::SimFactory;
use pg_content::ContentSet;
use pg_core::sim::{Sim, SimSnapshot};
use pg_persist::scenario::ScenarioEnv;
use pg_script::host::ScriptMeter;
use std::path::PathBuf;
use std::sync::Arc;

pub struct RuntimeEnv {
    content: Option<Arc<ContentSet>>,
    meter: Arc<ScriptMeter>,
    root: PathBuf,
}

impl RuntimeEnv {
    /// `root` is where file names in scenarios are resolved from (the repository root).
    pub fn new(content: Option<Arc<ContentSet>>, root: impl Into<PathBuf>) -> RuntimeEnv {
        RuntimeEnv {
            content,
            meter: Arc::new(ScriptMeter::new()),
            root: root.into(),
        }
    }

    pub fn runs_scripts(&self) -> bool {
        SimFactory::dev(self.content.clone(), 1).runs_scripts()
    }
}

impl ScenarioEnv for RuntimeEnv {
    fn restore(&self, snapshot: SimSnapshot, threads: usize) -> Sim {
        SimFactory::dev(self.content.clone(), threads)
            .with_meter(Some(Arc::clone(&self.meter)))
            .restore(snapshot)
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        if path.contains("..") || std::path::Path::new(path).is_absolute() {
            return Err(format!(
                "'{path}' must be a relative path inside the repository"
            ));
        }
        std::fs::read(self.root.join(path)).map_err(|e| format!("{path}: {e}"))
    }

    fn script_fuel(&self) -> Option<u64> {
        self.runs_scripts()
            .then(|| self.meter.rows().iter().map(|(_, _, r)| r.fuel).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_content::{load_pack, ComponentRegistry, DirPack, Limits};
    use pg_persist::scenario::{run_with, Scenario};

    fn root() -> String {
        format!("{}/../..", env!("CARGO_MANIFEST_DIR"))
    }

    fn content(packs: &[&str]) -> Arc<ContentSet> {
        let loaded = packs
            .iter()
            .map(|p| {
                load_pack(&DirPack::new(format!("{}/{p}", root())), &Limits::default()).unwrap()
            })
            .collect();
        Arc::new(ContentSet::build(loaded, ComponentRegistry::builtin()).unwrap())
    }

    const HEAD: &str =
        r#""format":"playground-scenario","version":1,"name":"t","world":{"name":"T","seed":"s"}"#;

    #[test]
    fn a_soak_with_scripts_loaded_is_shadow_verified_and_its_fuel_is_flat() {
        let env = RuntimeEnv::new(
            Some(content(&[
                "data/base",
                "packs/cookbook/caffeine",
                "packs/cookbook/birthdays",
            ])),
            root(),
        );
        assert!(env.runs_scripts());
        let text = format!(
            r#"{{{HEAD},"steps":[{{"op":"create_map","w":30,"h":24,"style":1}},{{"op":"spawn_pawns","count":6}},
               {{"op":"soak","days":8,"shadow_threads":3}}]}}"#
        );
        let r = run_with(&Scenario::parse(&text).unwrap(), &env);
        assert!(r.ok(), "{:?}", r.outcomes);
        let msg = &r.outcomes[2].message;
        assert!(
            msg.contains("script fuel per day") && msg.contains("3 thread(s): identical"),
            "{msg}"
        );
    }

    #[test]
    fn fixtures_are_read_inside_the_repository_only() {
        let env = RuntimeEnv::new(None, root());
        assert!(env.read_file("fixtures/saves/world-v3.hash").is_ok());
        assert!(env.read_file("../outside.txt").is_err());
        assert!(env.read_file("C:/Windows/win.ini").is_err());
        assert!(env.script_fuel().is_none());
    }
}
