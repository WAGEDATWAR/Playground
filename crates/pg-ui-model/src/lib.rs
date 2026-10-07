//! View-model builders and UI state machines (no drawing). Blueprint §14.
//!
//! * [`widget`]: the widget tree every screen produces, with a text snapshot (suggestion S-027).
//! * [`types`]: events in, effects out, and the data screens show.
//! * [`app`] and `screens`: the screen state machines, pure functions from events to state and effects.
//! * [`overlay`]: the developer overlay's model.
//!
//! Nothing in this crate touches a window, the disk, the network or the simulation, so every flow is tested
//! headless.

pub mod app;
pub mod overlay;
mod screens;
pub mod types;
pub mod widget;

pub use app::{AppModel, Screen};
pub use types::{AppEffect, Key, UiEvent};
pub use widget::{Tree, Widget};

#[cfg(test)]
mod tests;
