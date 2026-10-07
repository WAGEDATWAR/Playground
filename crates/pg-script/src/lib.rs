//! Luau host: the `ScriptVm` boundary, sandbox profile, fuel and memory metering (Blueprint §23).
//!
//! Only the private `luau` module imports the binding crate; everything else speaks the project's own
//! value, registration and error types from [`vm`].

pub mod host;
mod luau;
pub mod vm;

pub use luau::{LuauVm, MAX_SOURCE_BYTES};
pub use vm::*;

#[cfg(test)]
mod tests;
