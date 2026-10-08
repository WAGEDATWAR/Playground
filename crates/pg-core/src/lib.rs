//! Pure, headless, deterministic simulation core. Blueprint §0, §4-§9.
//! No I/O, no clock, no scripting VM, no floating point in authoritative state.
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic, clippy::float_cmp)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub use pg_canon as canon;
pub mod action;
pub mod activity;
pub mod capacity;
pub mod commands;
pub mod commitment;
pub mod containment;
pub mod dev;
pub mod events;
pub mod ext;
pub mod hash;
pub mod hooks;
pub mod id;
pub mod input;
pub mod life;
pub mod map;
pub mod movement;
pub mod num;
pub mod object;
pub mod occupancy;
pub mod path;
pub mod pawn;
pub mod pipeline;
pub mod population;
pub mod read;
pub mod reason;
pub mod replay;
#[cfg(test)]
mod restore_tests;
pub mod rng;
pub mod schedule;
pub mod sim;
pub mod social;
pub mod table;
pub mod time;
pub mod town;
pub mod vectors;
pub mod world;
pub mod worldgen;
