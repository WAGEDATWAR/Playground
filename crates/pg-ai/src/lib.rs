//! Provider adapters, circuit breaker, settings and the AI client. Blueprint §9-§10.
//!
//! Milestone 0.7 builds the skeleton: pure provider adapters, the breaker, settings and key handling,
//! and a client that is safe to fail. Dialogue prompts (Stage 1) and proposals (Stage 10) build on it.

pub mod breaker;
pub mod client;
pub mod error;
pub mod login;
pub mod provider;
pub mod selfcheck;
pub mod settings;
