//! Sim thread, command queue, job queue, session, autosave, snapshot publisher. Blueprint §1, §6.
//!
//! Milestone 0.4 adds only the threaded path-batch executor; the rest of the runtime arrives in 0.8.

pub mod app;
pub mod console;
pub mod control;
pub mod devtools;
pub mod dialogue;
pub mod exec;
pub mod guard;
pub mod keyframes;
pub mod pool;
pub mod profile;
pub mod scenario_env;
pub mod session;
pub mod settings;
pub mod sim_loop;
pub mod snapshot;
pub mod thumbnail;
