//! The typed event catalog (suggestion S-022, Blueprint §6.2).
//!
//! Events stay `(kind, detail)` values and are not part of hashed state, but every kind is declared once
//! here: its category, its detail fields (the same schema machinery as components) and whether it shows by
//! default in the event viewer. Debug builds validate every event a step emits (so a mistyped kind or a
//! forgotten field fails a test immediately), `pg events list` prints the catalog, and the overlay's event
//! viewer reads the same declarations. Packs add events as `<pack>.<kind>` through [`EventCatalog::register`].

use crate::pipeline::Event;
use pg_content::schema::{Field, FieldSchema, ParamSchema};
use pg_content::ValidationReport;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Input,
    World,
    Movement,
    Schedule,
    Task,
    Commitment,
    Dev,
    /// Declared by a content pack.
    Pack,
}

impl Category {
    pub const fn name(self) -> &'static str {
        match self {
            Category::Input => "input",
            Category::World => "world",
            Category::Movement => "movement",
            Category::Schedule => "schedule",
            Category::Task => "task",
            Category::Commitment => "commitment",
            Category::Dev => "dev",
            Category::Pack => "pack",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EventDef {
    pub kind: String,
    pub category: Category,
    pub fields: ParamSchema,
    /// Shown in the event viewer unless the player filters it out (noisy kinds default to hidden).
    pub default_visible: bool,
    pub summary: String,
}

#[derive(Clone, Debug, Default)]
pub struct EventCatalog {
    defs: BTreeMap<String, EventDef>,
}

fn id(kind: &str) -> Field {
    Field::required(FieldSchema::EntityId { kind: kind.to_owned() })
}

fn tile() -> Field {
    Field::required(FieldSchema::Tile)
}

fn int() -> Field {
    Field::required_int(0, i64::MAX)
}

fn text(max: usize) -> Field {
    Field::required_text(max)
}

fn maybe_text(max: usize) -> Field {
    Field::maybe(FieldSchema::Text { max_len: max })
}

fn schema(fields: &[(&str, Field)]) -> ParamSchema {
    fields
        .iter()
        .fold(ParamSchema::new(), |s, (n, f)| s.field(n, f.clone()))
}

impl EventCatalog {
    pub fn new() -> EventCatalog {
        EventCatalog::default()
    }

    /// The engine's own events.
    pub fn builtin() -> EventCatalog {
        let mut c = EventCatalog::new();
        let mut add = |kind: &str, category, visible: bool, summary: &str, fields: &[(&str, Field)]| {
            c.defs.insert(
                kind.to_owned(),
                EventDef {
                    kind: kind.to_owned(),
                    category,
                    fields: schema(fields),
                    default_visible: visible,
                    summary: summary.to_owned(),
                },
            );
        };
        use Category::*;
        add("input_rejected", Input, true, "A command or setting change was refused.", &[("command", maybe_text(64)), ("reason", text(400))]);
        add("setting_changed", Input, true, "A setting changed.", &[("key", text(64)), ("value", int())]);
        add("dev.nudged", Dev, true, "Developer command: the probe value changed.", &[("amount", Field::required_int(i64::from(i32::MIN), i64::from(i32::MAX)))]);
        add("dev.day_started", Dev, true, "Developer scaffolding: a day began.", &[("day", int())]);
        add("map.created", World, true, "A map was created.", &[("map", id("map")), ("w", int()), ("h", int()), ("style", Field::required_int(0, 255))]);
        add("pawn.spawned", World, true, "A pawn was created.", &[("pawn", id("pawn")), ("tile", tile())]);
        add("object.spawned", World, true, "An object was created.", &[("object", id("obj")), ("tile", tile())]);
        add("object.contained", World, true, "An object was put in a container.", &[("object", id("obj")), ("owner", id("obj")), ("container", text(64))]);
        add("world_edited", World, true, "A map tile was edited.", &[("map", id("map")), ("tile", tile()), ("blocked", Field::required(FieldSchema::Bool))]);
        add("move.requested", Movement, true, "A pawn was sent somewhere.", &[("pawn", id("pawn")), ("to", tile())]);
        add("move.arrived", Movement, false, "A pawn reached its destination.", &[("pawn", id("pawn")), ("tile", tile())]);
        add("move.failed", Movement, true, "A pawn's route failed.", &[("pawn", id("pawn")), ("reason", text(64))]);
        add("move.sidestep", Movement, false, "A pawn stepped aside to get past another.", &[("pawn", id("pawn")), ("tile", tile())]);
        add("schedule.planned", Schedule, true, "A pawn's day was planned.", &[("pawn", id("pawn")), ("day", int()), ("reservations", int()), ("dropped", int())]);
        add("schedule.replanned", Schedule, true, "The rest of a pawn's day was replanned.", &[("pawn", id("pawn")), ("from", int()), ("why", text(400))]);
        add("task.started", Task, false, "A pawn began a task.", &[("pawn", id("pawn")), ("reservation", int()), ("action", text(64))]);
        add("task.done", Task, false, "A pawn finished a task.", &[("pawn", id("pawn")), ("reservation", int()), ("action", text(64))]);
        add(
            "task.failed",
            Task,
            true,
            "A task ended in failure, with the reason.",
            &[
                ("pawn", id("pawn")),
                ("reservation", Field::maybe(FieldSchema::Int { min: 0, max: i64::MAX })),
                ("reason", Field::required(FieldSchema::Any)),
                ("explain", text(400)),
            ],
        );
        add("commitment.proposed", Commitment, true, "One pawn asked another to meet.", &[("commitment", id("cmt")), ("proposer", id("pawn")), ("invitee", id("pawn")), ("start", int()), ("len", int())]);
        for (state, summary) in [
            ("accepted", "The invitee accepted; both schedules reserved the time."),
            ("declined", "The invitee declined."),
            ("expired", "The proposal expired unanswered."),
            ("active", "The commitment began."),
            ("completed", "Both pawns kept the commitment."),
            ("failed", "The commitment failed."),
            ("cancelled", "The commitment was cancelled."),
        ] {
            add(&format!("commitment.{state}"), Commitment, true, summary, &[("commitment", id("cmt")), ("explain", maybe_text(400))]);
        }
        c
    }

    /// The shared engine catalog.
    pub fn shared() -> &'static EventCatalog {
        static CATALOG: OnceLock<EventCatalog> = OnceLock::new();
        CATALOG.get_or_init(EventCatalog::builtin)
    }

    /// Declares a pack event. The kind must be `<pack>.<name>`.
    pub fn register(&mut self, pack: &str, name: &str, fields: ParamSchema, default_visible: bool, summary: &str) -> Result<(), String> {
        let ok_part = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok_part(pack) || !ok_part(name) {
            return Err(format!("'{pack}.{name}' is not a valid event kind"));
        }
        let kind = format!("{pack}.{name}");
        if self.defs.contains_key(&kind) {
            return Err(format!("event '{kind}' is already declared"));
        }
        self.defs.insert(
            kind.clone(),
            EventDef { kind, category: Category::Pack, fields, default_visible, summary: summary.to_owned() },
        );
        Ok(())
    }

    pub fn get(&self, kind: &str) -> Option<&EventDef> {
        self.defs.get(kind)
    }

    /// Declarations in kind order.
    pub fn iter(&self) -> impl Iterator<Item = &EventDef> {
        self.defs.values()
    }

    /// Checks an event against its declaration.
    pub fn validate(&self, e: &Event) -> Result<(), String> {
        let def = self.defs.get(&e.kind).ok_or_else(|| {
            format!(
                "event kind '{}' is not in the catalog{}",
                e.kind,
                pg_content::hints::hint(&e.kind, self.defs.keys().map(String::as_str))
            )
        })?;
        let mut report = ValidationReport::new();
        def.fields.check(&e.detail, &e.kind, &mut report);
        let first = report
            .errors()
            .next()
            .map(|i| format!("event '{}' has a bad detail: {}: {}", e.kind, i.path, i.message));
        first.map_or(Ok(()), Err)
    }

    /// Validates a batch, returning every problem.
    pub fn validate_all<'a>(&self, events: impl IntoIterator<Item = &'a Event>) -> Vec<String> {
        events.into_iter().filter_map(|e| self.validate(e).err()).collect()
    }
}

#[cfg(test)]
mod tests;
