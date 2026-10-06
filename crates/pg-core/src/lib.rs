//! Pure, headless, deterministic simulation core. Blueprint §0, §4-§9.
//! No I/O, no clock, no scripting VM, no floating point in authoritative state.
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic, clippy::float_cmp)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing))]
