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
pub mod commands;
pub mod containment;
pub mod dev;
pub mod hash;
pub mod id;
pub mod input;
pub mod map;
pub mod movement;
pub mod num;
pub mod object;
pub mod occupancy;
pub mod path;
pub mod pawn;
pub mod pipeline;
pub mod replay;
pub mod rng;
pub mod sim;
pub mod table;
pub mod time;
pub mod vectors;
pub mod world;
