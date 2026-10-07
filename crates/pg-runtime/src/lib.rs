//! Sim thread, command queue, job queue, session, autosave, snapshot publisher. Blueprint §1, §6.
//!
//! Milestone 0.4 adds only the threaded path-batch executor; the rest of the runtime arrives in 0.8.

pub mod exec;
pub mod settings;
